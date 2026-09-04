//! SQLite engine on top of `rusqlite`.
//!
//! # Concurrency model
//!
//! SQLite allows exactly one writer at a time; readers in WAL mode run
//! concurrently with it. Instead of letting a pool of N identical
//! connections race for the write lock (and surface `SQLITE_BUSY` to
//! each other), this engine has:
//!
//! - **one writer connection** behind a `tokio::sync::Mutex`. Every
//!   `execute` and every [`Transaction`] goes through it, so our own
//!   statements never contend; `busy_timeout` only matters for external
//!   processes touching the same file. The tokio mutex is async, so a
//!   transaction may `await` hooks or the JS runtime between statements
//!   without blocking a runtime thread.
//! - **N reader connections** (`readers`) with `query_only=ON`, handed out
//!   through a `Semaphore` plus a free-list. Callers use `query` for
//!   reads; a statement that writes must go through `execute` or a
//!   transaction.
//!
//! Every statement runs inside `tokio::task::spawn_blocking` because
//! rusqlite is synchronous and a page-cache miss is real disk I/O.
//!
//! # `:memory:`
//!
//! An in-memory database is private to the connection that opened it, so
//! `":memory:"` uses a single shared connection: `readers` is forced to
//! zero and every operation, reads included, runs on the writer under
//! its lock. That makes memory engines strictly serialized, which is
//! exactly what tests want. Note that on a memory engine a caller
//! holding a [`Transaction`] must run its reads *through the
//! transaction*; calling `engine.query` while the transaction is alive
//! would wait for the writer lock the transaction holds.
//!
//! # Placeholders
//!
//! Callers write `$1..$n` (the Postgres style, shared with the filter
//! compiler). SQLite accepts `?NNN`, so the statement string is rewritten
//! once and cached per distinct SQL string.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use cratebase_filter::Dialect;
use rusqlite::types::{ToSqlOutput, ValueRef};
use rusqlite::{ffi, Connection, OpenFlags};
use tokio::sync::{OwnedMutexGuard, Semaphore};

use crate::engine::{Engine, Executor, Row, Sql, Transaction, TransactionImpl};
use crate::error::{DbError, DbResult};

/// The single writer connection, shared between the engine and any live
/// transaction. `None` once the engine is closed.
type SharedConn = Arc<Mutex<Option<Connection>>>;

/// Prepared-statement cache size per connection.
const STATEMENT_CACHE: usize = 256;
/// Upper bound on distinct statement strings whose placeholder rewrite
/// is remembered; beyond that the map is cleared (never grows unbounded
/// from ad-hoc SQL).
const REWRITE_CACHE_MAX: usize = 1024;

#[derive(Clone)]
pub struct SqliteEngine {
    inner: Arc<Inner>,
}

struct Inner {
    path: String,
    writer: SharedConn,
    /// Async serialization of the writer. Held for the whole lifetime of
    /// a [`Transaction`].
    writer_lock: Arc<tokio::sync::Mutex<()>>,
    /// Idle reader connections. A reader is popped for the duration of a
    /// blocking query and pushed back afterwards.
    readers: Mutex<Vec<Connection>>,
    reader_permits: Arc<Semaphore>,
    reader_count: usize,
    rewrites: Mutex<HashMap<String, Arc<str>>>,
    closed: AtomicBool,
}

/// Number of reader connections to open by default: one per core,
/// clamped to `4..=16`, unless `DATABASE_MAX_CONNECTIONS` says otherwise.
pub fn default_readers() -> usize {
    if let Some(n) = std::env::var("DATABASE_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
    {
        return n;
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(4, 16)
}

impl SqliteEngine {
    /// Open (creating if needed) the database at `path` with `readers`
    /// read-only connections. `":memory:"` ignores `readers` (see the
    /// module docs).
    pub fn open(path: &str, readers: usize) -> DbResult<Self> {
        let memory = is_memory(path);
        if !memory {
            if let Some(parent) = Path::new(path).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
        }
        let writer = open_connection(path, false)?;
        let reader_count = if memory { 0 } else { readers };
        let mut reader_conns = Vec::with_capacity(reader_count);
        for _ in 0..reader_count {
            reader_conns.push(open_connection(path, true)?);
        }
        Ok(SqliteEngine {
            inner: Arc::new(Inner {
                path: path.to_string(),
                writer: Arc::new(Mutex::new(Some(writer))),
                writer_lock: Arc::new(tokio::sync::Mutex::new(())),
                readers: Mutex::new(reader_conns),
                reader_permits: Arc::new(Semaphore::new(reader_count)),
                reader_count,
                rewrites: Mutex::new(HashMap::new()),
                closed: AtomicBool::new(false),
            }),
        })
    }

    /// A private in-memory database (tests).
    pub fn open_memory() -> DbResult<Self> {
        Self::open(":memory:", 0)
    }

    pub fn path(&self) -> &str {
        &self.inner.path
    }

    pub fn is_memory(&self) -> bool {
        is_memory(&self.inner.path)
    }

    pub fn reader_count(&self) -> usize {
        self.inner.reader_count
    }

    /// Run `f` on the writer connection, serialized behind the async
    /// writer lock, on a blocking thread.
    async fn on_writer<T, F>(&self, f: F) -> DbResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> DbResult<T> + Send + 'static,
    {
        self.ensure_open()?;
        let _guard = self.inner.writer_lock.lock().await;
        run_on_shared(&self.inner.writer, f).await
    }

    /// Run `f` on an idle reader connection (or the writer when there are
    /// none, i.e. `:memory:`).
    async fn on_reader<T, F>(&self, f: F) -> DbResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> DbResult<T> + Send + 'static,
    {
        self.ensure_open()?;
        if self.inner.reader_count == 0 {
            return self.on_writer(f).await;
        }
        let _permit = self
            .inner
            .reader_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| DbError::Pool("sqlite engine closed".into()))?;
        // The permit guarantees a connection is free unless a previous
        // blocking task panicked and lost one; reopen in that case.
        let idle = lock(&self.inner.readers).pop();
        let conn = match idle {
            Some(c) => c,
            None => {
                let path = self.inner.path.clone();
                tokio::task::spawn_blocking(move || open_connection(&path, true)).await??
            }
        };
        let inner = self.inner.clone();
        let (conn, result) = tokio::task::spawn_blocking(move || {
            let r = f(&conn);
            (conn, r)
        })
        .await?;
        lock(&inner.readers).push(conn);
        result
    }

    fn ensure_open(&self) -> DbResult<()> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(DbError::Pool("sqlite engine closed".into()));
        }
        Ok(())
    }
}

fn is_memory(path: &str) -> bool {
    path == ":memory:" || path.is_empty()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn run_on_shared<T, F>(conn: &SharedConn, f: F) -> DbResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> DbResult<T> + Send + 'static,
{
    let conn = conn.clone();
    tokio::task::spawn_blocking(move || {
        let guard = lock(&conn);
        let c = guard
            .as_ref()
            .ok_or_else(|| DbError::Pool("sqlite engine closed".into()))?;
        f(c)
    })
    .await?
}

/// Open one connection and apply the per-connection PRAGMAs. Readers
/// additionally get `query_only=ON` so a stray write through the read
/// path fails loudly instead of silently taking the write lock.
fn open_connection(path: &str, reader: bool) -> DbResult<Connection> {
    let conn = if is_memory(path) {
        Connection::open_in_memory()?
    } else {
        Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?
    };
    conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE);
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA cache_size = -16000;
         PRAGMA temp_store = MEMORY;
         PRAGMA mmap_size = 268435456;
         PRAGMA journal_size_limit = 200000000;",
    )?;
    if reader {
        conn.execute_batch("PRAGMA query_only = ON;")?;
    }
    Ok(conn)
}

impl Inner {
    /// `$n` → `?n`, cached per statement string.
    fn rewritten(&self, sql: &str) -> Arc<str> {
        if let Some(hit) = lock(&self.rewrites).get(sql) {
            return hit.clone();
        }
        let rewritten: Arc<str> = rewrite_placeholders(sql).into();
        let mut cache = lock(&self.rewrites);
        if cache.len() >= REWRITE_CACHE_MAX {
            cache.clear();
        }
        cache.insert(sql.to_string(), rewritten.clone());
        rewritten
    }
}

/// Rewrite `$1`-style placeholders to `?1`, leaving quoted strings and
/// identifiers untouched.
pub fn rewrite_placeholders(sql: &str) -> String {
    if !sql.contains('$') {
        return sql.to_string();
    }
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                out.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' | '`' => {
                    quote = Some(c);
                    out.push(c);
                }
                '$' if chars.peek().is_some_and(|n| n.is_ascii_digit()) => out.push('?'),
                _ => out.push(c),
            },
        }
    }
    out
}

/// Bind adapter so a `&Sql` can be passed to rusqlite without copying.
struct Param<'a>(&'a Sql);

impl rusqlite::ToSql for Param<'_> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(match self.0 {
            Sql::Null => ValueRef::Null,
            Sql::Int(i) => ValueRef::Integer(*i),
            Sql::Real(f) => ValueRef::Real(*f),
            Sql::Text(s) => ValueRef::Text(s.as_bytes()),
            Sql::Blob(b) => ValueRef::Blob(b),
        }))
    }
}

fn decode(v: ValueRef<'_>) -> Sql {
    match v {
        ValueRef::Null => Sql::Null,
        ValueRef::Integer(i) => Sql::Int(i),
        ValueRef::Real(f) => Sql::Real(f),
        ValueRef::Text(t) => Sql::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Sql::Blob(b.to_vec()),
    }
}

fn bind(stmt: &mut rusqlite::Statement<'_>, params: &[Sql]) -> DbResult<()> {
    let expected = stmt.parameter_count();
    if expected != params.len() {
        return Err(DbError::Other(format!(
            "statement expects {expected} parameter(s), got {}",
            params.len()
        )));
    }
    for (i, p) in params.iter().enumerate() {
        stmt.raw_bind_parameter(i + 1, Param(p)).map_err(map_err)?;
    }
    Ok(())
}

/// Run a statement that produces rows.
pub(crate) fn run_query(conn: &Connection, sql: &str, params: &[Sql]) -> DbResult<Vec<Row>> {
    let mut stmt = conn.prepare_cached(sql).map_err(map_err)?;
    bind(&mut stmt, params)?;
    let mut rows = stmt.raw_query();
    let mut out = Vec::new();
    // Column names are read after the first step, not at prepare time:
    // a cached statement may have been compiled against an older table
    // layout, and SQLite re-prepares it transparently on step.
    let mut columns: Option<Arc<[String]>> = None;
    while let Some(row) = rows.next().map_err(map_err)? {
        let columns = columns.get_or_insert_with(|| {
            row.as_ref()
                .column_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
                .into()
        });
        let mut values = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            values.push(decode(row.get_ref(i).map_err(map_err)?));
        }
        out.push(Row {
            columns: columns.clone(),
            values,
        });
    }
    Ok(out)
}

/// Run a statement for its side effects. Rows it happens to return
/// (PRAGMAs, `RETURNING`) are drained and discarded. Returns the number
/// of rows changed by the most recent INSERT/UPDATE/DELETE.
pub(crate) fn run_execute(conn: &Connection, sql: &str, params: &[Sql]) -> DbResult<u64> {
    let mut stmt = conn.prepare_cached(sql).map_err(map_err)?;
    bind(&mut stmt, params)?;
    let mut rows = stmt.raw_query();
    while rows.next().map_err(map_err)?.is_some() {}
    drop(rows);
    Ok(conn.changes())
}

/// Map driver errors: unique violations and other constraint failures
/// become structured variants the record layer turns into validation
/// errors; everything else is passed through.
pub(crate) fn map_err(e: rusqlite::Error) -> DbError {
    if let rusqlite::Error::SqliteFailure(code, msg) = &e {
        let message = msg.as_deref().unwrap_or("");
        if code.extended_code == ffi::SQLITE_CONSTRAINT_UNIQUE
            || code.extended_code == ffi::SQLITE_CONSTRAINT_PRIMARYKEY
        {
            // "UNIQUE constraint failed: posts.slug" → "posts.slug"
            let detail = message
                .split_once("constraint failed: ")
                .map(|(_, d)| d)
                .unwrap_or(message);
            return DbError::UniqueViolation(detail.to_string());
        }
        if code.code == rusqlite::ErrorCode::ConstraintViolation {
            return DbError::Constraint(message.to_string());
        }
    }
    DbError::Sqlite(e)
}

#[async_trait]
impl Executor for SqliteEngine {
    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    async fn query(&self, sql: &str, params: &[Sql]) -> DbResult<Vec<Row>> {
        let sql = self.inner.rewritten(sql);
        let params = params.to_vec();
        self.on_reader(move |conn| run_query(conn, &sql, &params))
            .await
    }

    async fn execute(&self, sql: &str, params: &[Sql]) -> DbResult<u64> {
        let sql = self.inner.rewritten(sql);
        let params = params.to_vec();
        self.on_writer(move |conn| run_execute(conn, &sql, &params))
            .await
    }
}

#[async_trait]
impl Engine for SqliteEngine {
    async fn begin(&self) -> DbResult<Transaction> {
        self.ensure_open()?;
        let guard = self.inner.writer_lock.clone().lock_owned().await;
        // IMMEDIATE takes the write lock up front so the transaction can
        // never fail with BUSY at commit time after doing work.
        run_on_shared(&self.inner.writer, |conn| {
            conn.execute_batch("BEGIN IMMEDIATE").map_err(map_err)
        })
        .await?;
        Ok(Transaction::new(Box::new(SqliteTransaction {
            inner: self.inner.clone(),
            conn: self.inner.writer.clone(),
            _guard: guard,
            open: AtomicBool::new(true),
        })))
    }

    async fn table_exists(&self, table: &str) -> DbResult<bool> {
        let rows = self
            .query(
                "SELECT 1 FROM sqlite_master WHERE type IN ('table', 'view') AND name = $1",
                &[Sql::from(table)],
            )
            .await?;
        Ok(!rows.is_empty())
    }

    async fn table_columns(&self, table: &str) -> DbResult<Vec<String>> {
        let rows = self
            .query(
                "SELECT name FROM pragma_table_info($1)",
                &[Sql::from(table)],
            )
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.values.into_iter().next().map(Sql::into_string))
            .collect())
    }

    /// Explicit indexes only; the implicit `sqlite_autoindex_*` entries
    /// SQLite creates for PRIMARY KEY/UNIQUE constraints are omitted.
    async fn table_indexes(&self, table: &str) -> DbResult<Vec<String>> {
        let rows = self
            .query(
                "SELECT name FROM pragma_index_list($1) WHERE name NOT LIKE 'sqlite_%'",
                &[Sql::from(table)],
            )
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.values.into_iter().next().map(Sql::into_string))
            .collect())
    }

    async fn optimize(&self) -> DbResult<()> {
        self.on_writer(|conn| {
            conn.execute_batch("PRAGMA optimize; PRAGMA wal_checkpoint(TRUNCATE);")
                .map_err(map_err)
        })
        .await
    }

    async fn snapshot_to(&self, dest_path: &str) -> DbResult<()> {
        let escaped = dest_path.replace('\'', "''");
        self.on_writer(move |conn| {
            conn.execute_batch(&format!("VACUUM INTO '{escaped}'"))
                .map_err(map_err)
        })
        .await
    }

    async fn close(&self) -> DbResult<()> {
        if self.inner.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        // Wait for any in-flight transaction before dropping the writer.
        let _guard = self.inner.writer_lock.lock().await;
        self.inner.reader_permits.close();
        let readers: Vec<Connection> = std::mem::take(&mut *lock(&self.inner.readers));
        let writer = self.inner.writer.clone();
        tokio::task::spawn_blocking(move || {
            drop(readers);
            lock(&writer).take();
        })
        .await?;
        Ok(())
    }
}

/// A write transaction on the single writer connection. Holds the async
/// writer guard, so nothing else can write until it is committed,
/// rolled back, or dropped (which rolls back).
struct SqliteTransaction {
    inner: Arc<Inner>,
    conn: SharedConn,
    _guard: OwnedMutexGuard<()>,
    open: AtomicBool,
}

impl SqliteTransaction {
    async fn finish(&self, stmt: &'static str) -> DbResult<()> {
        run_on_shared(&self.conn, move |conn| {
            conn.execute_batch(stmt).map_err(map_err)
        })
        .await?;
        self.open.store(false, Ordering::Release);
        Ok(())
    }
}

#[async_trait]
impl Executor for SqliteTransaction {
    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    async fn query(&self, sql: &str, params: &[Sql]) -> DbResult<Vec<Row>> {
        let sql = self.inner.rewritten(sql);
        let params = params.to_vec();
        run_on_shared(&self.conn, move |conn| run_query(conn, &sql, &params)).await
    }

    async fn execute(&self, sql: &str, params: &[Sql]) -> DbResult<u64> {
        let sql = self.inner.rewritten(sql);
        let params = params.to_vec();
        run_on_shared(&self.conn, move |conn| run_execute(conn, &sql, &params)).await
    }
}

#[async_trait]
impl TransactionImpl for SqliteTransaction {
    async fn commit(self: Box<Self>) -> DbResult<()> {
        self.finish("COMMIT").await
    }

    async fn rollback(self: Box<Self>) -> DbResult<()> {
        self.finish("ROLLBACK").await
    }
}

impl Drop for SqliteTransaction {
    fn drop(&mut self) {
        if !self.open.load(Ordering::Acquire) {
            return;
        }
        // Best-effort synchronous rollback. We own the writer (the guard
        // is still held), so this only waits for a blocking statement of
        // this very transaction that may still be running, never for
        // another writer. ROLLBACK itself is a cheap in-memory operation.
        if let Some(conn) = lock(&self.conn).as_ref() {
            if let Err(e) = conn.execute_batch("ROLLBACK") {
                tracing::warn!(error = %e, "rollback on dropped transaction failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_placeholders_outside_quotes() {
        assert_eq!(
            rewrite_placeholders("SELECT $1, '$2', \"$3\", $10 FROM t WHERE a = $4"),
            "SELECT ?1, '$2', \"$3\", ?10 FROM t WHERE a = ?4"
        );
        assert_eq!(rewrite_placeholders("SELECT 1"), "SELECT 1");
        assert_eq!(rewrite_placeholders("SELECT '$'"), "SELECT '$'");
        assert_eq!(rewrite_placeholders("SELECT $x"), "SELECT $x");
    }

    #[tokio::test]
    async fn memory_engine_round_trips_values() {
        let e = SqliteEngine::open_memory().unwrap();
        assert_eq!(e.reader_count(), 0);
        e.execute(
            "CREATE TABLE t (i INTEGER, r REAL, s TEXT, b BLOB, n TEXT)",
            &[],
        )
        .await
        .unwrap();
        let n = e
            .execute(
                "INSERT INTO t VALUES ($1, $2, $3, $4, $5)",
                &[
                    Sql::Int(7),
                    Sql::Real(1.5),
                    Sql::from("hi"),
                    Sql::Blob(vec![1, 2]),
                    Sql::Null,
                ],
            )
            .await
            .unwrap();
        assert_eq!(n, 1);
        let rows = e.query("SELECT * FROM t", &[]).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].values[0], Sql::Int(7));
        assert_eq!(rows[0].values[1], Sql::Real(1.5));
        assert_eq!(rows[0].get_str("s"), Some("hi"));
        assert_eq!(rows[0].values[3], Sql::Blob(vec![1, 2]));
        assert!(rows[0].values[4].is_null());
    }

    #[tokio::test]
    async fn parameter_count_mismatch_is_an_error() {
        let e = SqliteEngine::open_memory().unwrap();
        let err = e.query("SELECT $1, $2", &[Sql::Int(1)]).await.unwrap_err();
        assert!(matches!(err, DbError::Other(_)), "{err:?}");
    }

    #[tokio::test]
    async fn unique_violation_is_mapped() {
        let e = SqliteEngine::open_memory().unwrap();
        e.execute("CREATE TABLE u (id TEXT PRIMARY KEY, email TEXT)", &[])
            .await
            .unwrap();
        e.execute("CREATE UNIQUE INDEX idx_email_x ON u (email)", &[])
            .await
            .unwrap();
        e.execute("INSERT INTO u VALUES ('a', 'x@y')", &[])
            .await
            .unwrap();
        let err = e
            .execute("INSERT INTO u VALUES ('b', 'x@y')", &[])
            .await
            .unwrap_err();
        match err {
            DbError::UniqueViolation(d) => assert_eq!(d, "u.email"),
            other => panic!("expected unique violation, got {other:?}"),
        }
        let err = e
            .execute("INSERT INTO u VALUES ('a', 'z@y')", &[])
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::UniqueViolation(_)));
        e.execute("CREATE TABLE nn (v TEXT NOT NULL)", &[])
            .await
            .unwrap();
        let err = e
            .execute("INSERT INTO nn VALUES ($1)", &[Sql::Null])
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::Constraint(_)), "{err:?}");
    }

    #[tokio::test]
    async fn transaction_commits_and_rolls_back_on_drop() {
        let e = SqliteEngine::open_memory().unwrap();
        e.execute("CREATE TABLE t (v TEXT)", &[]).await.unwrap();

        let tx = e.begin().await.unwrap();
        tx.execute("INSERT INTO t VALUES ('kept')", &[])
            .await
            .unwrap();
        assert_eq!(tx.query("SELECT * FROM t", &[]).await.unwrap().len(), 1);
        tx.commit().await.unwrap();

        let tx = e.begin().await.unwrap();
        tx.execute("INSERT INTO t VALUES ('dropped')", &[])
            .await
            .unwrap();
        drop(tx);

        let tx = e.begin().await.unwrap();
        tx.execute("INSERT INTO t VALUES ('rolled')", &[])
            .await
            .unwrap();
        tx.rollback().await.unwrap();

        let rows = e.query("SELECT v FROM t", &[]).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get_str("v"), Some("kept"));
    }

    #[tokio::test]
    async fn file_engine_serves_readers_during_a_write_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let e = SqliteEngine::open(path.to_str().unwrap(), 2).unwrap();
        assert_eq!(e.reader_count(), 2);
        e.execute("CREATE TABLE t (v TEXT)", &[]).await.unwrap();
        e.execute("INSERT INTO t VALUES ('a')", &[]).await.unwrap();

        let tx = e.begin().await.unwrap();
        tx.execute("INSERT INTO t VALUES ('b')", &[]).await.unwrap();
        // Readers see the committed snapshot while the writer is busy.
        let rows = e.query("SELECT v FROM t", &[]).await.unwrap();
        assert_eq!(rows.len(), 1);
        // A concurrent writer waits for the transaction instead of
        // failing with BUSY.
        let e2 = e.clone();
        let writer =
            tokio::spawn(async move { e2.execute("INSERT INTO t VALUES ('c')", &[]).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!writer.is_finished());
        tx.commit().await.unwrap();
        writer.await.unwrap().unwrap();
        let rows = e.query("SELECT v FROM t ORDER BY v", &[]).await.unwrap();
        assert_eq!(rows.len(), 3);

        // Readers are query_only.
        let err = e
            .query("INSERT INTO t VALUES ('x')", &[])
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::Sqlite(_)), "{err:?}");

        assert!(e.table_exists("t").await.unwrap());
        assert!(!e.table_exists("nope").await.unwrap());
        assert_eq!(e.table_columns("t").await.unwrap(), vec!["v"]);
        e.execute("CREATE INDEX idx_t_v ON t (v)", &[])
            .await
            .unwrap();
        assert_eq!(e.table_indexes("t").await.unwrap(), vec!["idx_t_v"]);
        e.optimize().await.unwrap();
        let snap = dir.path().join("snap.db");
        e.snapshot_to(snap.to_str().unwrap()).await.unwrap();
        assert!(snap.exists());
        e.close().await.unwrap();
        assert!(e.query("SELECT 1", &[]).await.is_err());
    }

    #[tokio::test]
    async fn many_concurrent_readers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.db");
        let e = SqliteEngine::open(path.to_str().unwrap(), 3).unwrap();
        e.execute("CREATE TABLE t (v INTEGER)", &[]).await.unwrap();
        for i in 0..20 {
            e.execute("INSERT INTO t VALUES ($1)", &[Sql::Int(i)])
                .await
                .unwrap();
        }
        let mut tasks = Vec::new();
        for _ in 0..32 {
            let e = e.clone();
            tasks.push(tokio::spawn(async move {
                e.query("SELECT COUNT(*) FROM t", &[]).await.unwrap()[0].values[0].as_i64()
            }));
        }
        for t in tasks {
            assert_eq!(t.await.unwrap(), Some(20));
        }
        assert_eq!(lock(&e.inner.readers).len(), 3);
    }
}
