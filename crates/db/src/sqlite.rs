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
//! - **one checkpointer connection**, used by nothing but the background
//!   WAL checkpointer (see below).
//!
//! Every statement runs inside `tokio::task::spawn_blocking` because
//! rusqlite is synchronous and a page-cache miss is real disk I/O.
//!
//! # WAL checkpointing
//!
//! SQLite's automatic checkpoint (`wal_autocheckpoint`, 1000 pages by
//! default) runs *inline, on the connection that commits the transaction
//! which pushes the WAL past the threshold*. One unlucky request
//! therefore pays for folding four megabytes of WAL back into the main
//! database, with our single writer connection locked for the duration.
//!
//! That is the whole write-latency tail, and it is not a small part of
//! it. 500 `DELETE /api/collections/posts/records/:id` at concurrency 20
//! against 6200 rows, request logging on, six rounds:
//!
//! | | req/s | p50 | p99 | max | WAL |
//! |---|---|---|---|---|---|
//! | SQLite's automatic checkpoint | 9.3k-10.2k | 1.3ms | 17-21ms | 21ms | 4 MB |
//! | checkpointed here instead | 14.4k-16.2k | 1.2ms | 1.5-2.8ms | 3.0ms | 28 MB |
//!
//! Exactly one request per round of 500 saw the stall, and it was ~20ms
//! of it — on an *idle* machine. On a host under memory and I/O pressure
//! the same checkpoint took 350-450ms, which is enough on its own to drag
//! a 500-request benchmark cell from 8000 req/s to 1040 req/s: one stall
//! filled the whole batch.
//!
//! So on a file database the automatic checkpoint is turned **off** and
//! [`SqliteEngine::spawn_checkpointer`] runs the checkpoint every
//! [`CHECKPOINT_INTERVAL`] on a connection of its own instead. What keeps
//! the log from growing without bound is described on [`checkpoint`]; the
//! short version is that a passive pass copies the WAL into the database
//! without blocking anyone, and once the log is past
//! [`CHECKPOINT_SOFT_LIMIT_PAGES`] a second pass under the writer lock
//! lets SQLite restart it. [`Engine::close`] then truncates on the way
//! out, so a restart never replays a large log and a backup never copies
//! one.
//!
//! If there is no Tokio runtime to spawn that task on — a synchronous
//! test, a CLI subcommand that opens the database and exits — the
//! automatic checkpoint is left exactly as SQLite ships it. The WAL is
//! never left with nobody responsible for it.
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

/// How often the background WAL checkpointer wakes up. Short enough that
/// a burst of writes never accumulates a WAL big enough for one pass to
/// be expensive, long enough to be free when nothing is writing (a
/// checkpoint with an empty WAL is a lock acquisition and nothing else).
const CHECKPOINT_INTERVAL: Duration = Duration::from_millis(250);

/// WAL size, in pages, above which the checkpointer stops letting the log
/// grow and takes the writer lock so SQLite can restart it (see
/// [`checkpoint`]). ~16 MB at SQLite's 4 KiB default page size: high
/// enough that an ordinary burst drains for free at the end of it, low
/// enough that the log never becomes a liability for crash recovery or
/// for `VACUUM INTO` during a backup.
const CHECKPOINT_SOFT_LIMIT_PAGES: i64 = 4_000;

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
    /// The background checkpointer's own connection, so a checkpoint
    /// never queues behind `writer_lock` and no request ever waits for
    /// one. `None` when the WAL is not ours to manage (`:memory:`, or no
    /// runtime to spawn the task on).
    checkpointer: SharedConn,
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
        // Taking the automatic checkpoint off the request path is only
        // safe if we can actually run one ourselves, which needs a
        // runtime to spawn the task on. Outside one (a synchronous test,
        // a CLI subcommand that opens the database and exits) SQLite's
        // own automatic checkpoint stays in charge.
        let manage_wal = !memory && tokio::runtime::Handle::try_current().is_ok();
        let writer = open_connection(path, false, manage_wal)?;
        let checkpointer = if manage_wal {
            Some(open_connection(path, false, manage_wal)?)
        } else {
            None
        };
        let reader_count = if memory { 0 } else { readers };
        let mut reader_conns = Vec::with_capacity(reader_count);
        for _ in 0..reader_count {
            reader_conns.push(open_connection(path, true, manage_wal)?);
        }
        let engine = SqliteEngine {
            inner: Arc::new(Inner {
                path: path.to_string(),
                writer: Arc::new(Mutex::new(Some(writer))),
                writer_lock: Arc::new(tokio::sync::Mutex::new(())),
                readers: Mutex::new(reader_conns),
                checkpointer: Arc::new(Mutex::new(checkpointer)),
                reader_permits: Arc::new(Semaphore::new(reader_count)),
                reader_count,
                rewrites: Mutex::new(HashMap::new()),
                closed: AtomicBool::new(false),
            }),
        };
        if manage_wal {
            engine.spawn_checkpointer();
        }
        Ok(engine)
    }

    /// Start the background WAL checkpointer described in the module
    /// docs. It holds a `Weak` to the engine's state, so it stops on its
    /// own once the last [`SqliteEngine`] clone is dropped — a test that
    /// opens a hundred temporary databases leaks a hundred tasks for at
    /// most one tick each, not for the life of the process.
    fn spawn_checkpointer(&self) {
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(CHECKPOINT_INTERVAL);
            // The point of the exercise is to keep checkpoints off the
            // request path; catching up on missed ticks by running
            // several back to back would do the opposite.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let Some(inner) = weak.upgrade() else { return };
                if inner.closed.load(Ordering::Acquire) {
                    return;
                }
                // Sequential by construction: the next tick cannot start
                // until this checkpoint has finished.
                if let Err(e) = checkpoint(&inner).await {
                    tracing::warn!(error = %e, "WAL checkpoint failed");
                }
            }
        });
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
                let manage_wal = self.inner.manages_wal();
                tokio::task::spawn_blocking(move || open_connection(&path, true, manage_wal))
                    .await??
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

/// One `PRAGMA wal_checkpoint(<mode>)`, returning the size of the WAL in
/// pages (`-1` when another connection held the checkpoint lock and this
/// one backed off). See the module docs for why this is called from a
/// background task rather than left to SQLite.
fn wal_checkpoint(conn: &Connection, mode: &str) -> DbResult<i64> {
    conn.query_row(&format!("PRAGMA wal_checkpoint({mode})"), [], |row| {
        row.get::<_, i64>(1)
    })
    .map_err(map_err)
}

/// One pass of the background checkpointer.
///
/// Two steps, because copying the WAL into the database and *reclaiming*
/// the file are different things. A `PASSIVE` checkpoint on our own
/// connection does the copying and blocks nobody, but SQLite only
/// restarts the log — reusing it from byte zero instead of appending —
/// when a writer begins a transaction and finds the whole log already
/// checkpointed. A saturated writer appends new frames faster than a
/// checkpoint on another connection can catch up, so that condition is
/// never true and the file grows for as long as the write burst lasts.
///
/// That is fine, and deliberately so: a growing WAL is how the checkpoint
/// stays off the request path during a burst, and it drains for free the
/// moment the writes pause. Past [`CHECKPOINT_SOFT_LIMIT_PAGES`] the
/// second step runs the same passive checkpoint *while holding the writer
/// lock*, which stops the log moving underneath it and lets the next
/// write restart it. That does make writers wait — for the copy only,
/// never for a reader, which is what `TRUNCATE` would add — so it happens
/// only once the log is large enough to be worth it.
async fn checkpoint(inner: &Arc<Inner>) -> DbResult<i64> {
    let conn = inner.checkpointer.clone();
    let pages = tokio::task::spawn_blocking(move || match lock(&conn).as_ref() {
        Some(conn) => wal_checkpoint(conn, "PASSIVE"),
        None => Ok(0),
    })
    .await??;
    if pages < CHECKPOINT_SOFT_LIMIT_PAGES {
        return Ok(pages);
    }
    let writer = inner.writer.clone();
    let _guard = inner.writer_lock.lock().await;
    run_on_shared(&writer, |conn| wal_checkpoint(conn, "PASSIVE")).await
}

/// Open one connection and apply the per-connection PRAGMAs. Readers
/// additionally get `query_only=ON` so a stray write through the read
/// path fails loudly instead of silently taking the write lock.
///
/// `manage_wal` turns SQLite's inline automatic checkpoint off because
/// [`SqliteEngine::spawn_checkpointer`] does the job on a background
/// connection instead; do not re-enable it without re-reading the
/// measurement in the module docs. `journal_size_limit` still applies:
/// it caps the file whenever a checkpoint does reset the log, whoever
/// ran it.
fn open_connection(path: &str, reader: bool, manage_wal: bool) -> DbResult<Connection> {
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
    if manage_wal {
        conn.execute_batch("PRAGMA wal_autocheckpoint = 0;")?;
    }
    if reader {
        conn.execute_batch("PRAGMA query_only = ON;")?;
    }
    register_geo_distance(&conn)?;
    Ok(conn)
}

/// Register `geoDistance(lonA, latA, lonB, latB)`, the great-circle
/// distance in kilometres that `cratebase-filter` compiles geo
/// comparisons into on SQLite. Postgres needs no equivalent: the
/// compiler inlines the same haversine there.
///
/// `NULL` in, `NULL` out, so a row with an unset geo point simply fails
/// the comparison instead of aborting the query. The function is
/// deterministic, which lets SQLite use it in an index or a partial
/// index predicate.
fn register_geo_distance(conn: &Connection) -> DbResult<()> {
    use rusqlite::functions::FunctionFlags;

    conn.create_scalar_function(
        "geoDistance",
        4,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |ctx| {
            let mut args = [0f64; 4];
            for (i, slot) in args.iter_mut().enumerate() {
                // Coordinates reach us as REAL from a geoPoint column but
                // as INTEGER or TEXT from a bound literal, so the
                // conversion has to be lenient rather than `as_f64`.
                match to_f64(ctx.get_raw(i)) {
                    Some(v) => *slot = v,
                    None => return Ok(None),
                }
            }
            let [lon_a, lat_a, lon_b, lat_b] = args;
            Ok(Some(haversine_km(lon_a, lat_a, lon_b, lat_b)))
        },
    )
    .map_err(map_err)
}

/// Any SQLite value as a float; `None` for NULL and anything unparsable.
fn to_f64(value: ValueRef<'_>) -> Option<f64> {
    match value {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i as f64),
        ValueRef::Real(f) => Some(f),
        ValueRef::Text(t) => std::str::from_utf8(t).ok()?.trim().parse().ok(),
        ValueRef::Blob(_) => None,
    }
}

/// Great-circle distance in kilometres (R = 6371), clamped so floating
/// point noise can never push `acos` out of its domain.
fn haversine_km(lon_a: f64, lat_a: f64, lon_b: f64, lat_b: f64) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6371.0;
    let (lat_a, lat_b) = (lat_a.to_radians(), lat_b.to_radians());
    let delta_lon = (lon_b - lon_a).to_radians();
    let cos = lat_a.sin() * lat_b.sin() + lat_a.cos() * lat_b.cos() * delta_lon.cos();
    EARTH_RADIUS_KM * cos.clamp(-1.0, 1.0).acos()
}

impl Inner {
    /// Whether this engine runs its own WAL checkpoints. Decided once in
    /// [`SqliteEngine::open`]; the checkpointer connection's presence is
    /// the record of it, so a connection reopened later gets the same
    /// `wal_autocheckpoint` setting as the ones opened at startup.
    fn manages_wal(&self) -> bool {
        lock(&self.checkpointer).is_some()
    }

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
        // `VACUUM INTO` reads through the connection and so already sees
        // everything committed to the WAL, but folding the log back first
        // keeps the copy cheap and means a backup taken right after a
        // write burst is not paying to walk a 50 MB log.
        let _ = checkpoint(&self.inner).await;
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
        let checkpointer = self.inner.checkpointer.clone();
        let manages_wal = self.inner.manages_wal();
        tokio::task::spawn_blocking(move || {
            // Readers hold marks that stop a checkpoint from reclaiming
            // the log, so they go first. Then truncate: with the automatic
            // checkpoint off, whatever is left in the WAL at shutdown
            // would otherwise be replayed by the next process to open the
            // file, and a backup of the data directory would need it.
            // `TRUNCATE` blocks, which is exactly what is wanted here —
            // there is nothing left to block.
            drop(readers);
            if manages_wal {
                let guard = lock(&writer);
                if let Some(conn) = guard.as_ref() {
                    if let Err(e) = wal_checkpoint(conn, "TRUNCATE") {
                        tracing::warn!(error = %e, "final WAL checkpoint failed");
                    }
                }
            }
            lock(&checkpointer).take();
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
    async fn geo_distance_is_registered_and_matches_the_haversine() {
        let e = SqliteEngine::open_memory().unwrap();
        let rows = e
            .query(
                "SELECT geoDistance(0.0, 0.0, 0.0, 1.0), geoDistance(1, 1, 1, 1), \
                 geoDistance(NULL, 0.0, 0.0, 1.0), geoDistance('0', '0', '0.0', '1.0')",
                &[],
            )
            .await
            .unwrap();
        let one_degree = rows[0].values[0].as_f64().unwrap();
        assert!((one_degree - 111.19).abs() < 0.1, "{one_degree}");
        assert_eq!(rows[0].values[1].as_f64(), Some(0.0));
        assert!(rows[0].values[2].is_null());
        assert_eq!(rows[0].values[3].as_f64(), rows[0].values[0].as_f64());
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
