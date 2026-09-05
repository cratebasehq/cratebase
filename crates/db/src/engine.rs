//! The storage engine contract. Everything above this trait (schema
//! sync, record queries, migrations, services) is backend-agnostic;
//! everything below it (`sqlite.rs`, `postgres.rs`) is not.
//!
//! Design notes:
//! - Placeholders are `$1..$n` in both dialects. The SQLite engine
//!   rewrites them to `?n` once per distinct statement string and caches
//!   the rewrite together with the prepared statement.
//! - Rows are decoded into [`Sql`] values with no intermediate copies,
//!   then mapped to JSON by the record layer using the collection's
//!   field types. TEXT columns come back as `Sql::Text`, INTEGER as
//!   `Sql::Int`, REAL as `Sql::Real`, NULL as `Sql::Null`.
//! - Writes are serialized: on SQLite there is exactly one writer
//!   connection, so our own statements never see `SQLITE_BUSY`.
//! - A [`Transaction`] is async-friendly: it holds the writer for its
//!   whole lifetime and callers may `await` other things (hooks, the JS
//!   runtime) between statements.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cratebase_filter::Dialect;

use crate::error::{DbError, DbResult};

/// A bound parameter or a decoded column value.
#[derive(Debug, Clone, PartialEq)]
pub enum Sql {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Sql {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Sql::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Sql::Int(i) => Some(*i),
            Sql::Real(f) => Some(*f as i64),
            Sql::Text(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Sql::Int(i) => Some(*i as f64),
            Sql::Real(f) => Some(*f),
            Sql::Text(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Sql::Null)
    }

    pub fn into_string(self) -> String {
        match self {
            Sql::Text(s) => s,
            Sql::Int(i) => i.to_string(),
            Sql::Real(f) => f.to_string(),
            Sql::Null => String::new(),
            Sql::Blob(b) => String::from_utf8_lossy(&b).into_owned(),
        }
    }
}

impl From<&str> for Sql {
    fn from(s: &str) -> Self {
        Sql::Text(s.to_owned())
    }
}
impl From<String> for Sql {
    fn from(s: String) -> Self {
        Sql::Text(s)
    }
}
impl From<i64> for Sql {
    fn from(i: i64) -> Self {
        Sql::Int(i)
    }
}
impl From<f64> for Sql {
    fn from(f: f64) -> Self {
        Sql::Real(f)
    }
}
impl From<bool> for Sql {
    fn from(b: bool) -> Self {
        Sql::Int(b as i64)
    }
}
impl<T: Into<Sql>> From<Option<T>> for Sql {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(v) => v.into(),
            None => Sql::Null,
        }
    }
}

/// One result row. `columns` is shared across every row of a result set.
#[derive(Debug, Clone)]
pub struct Row {
    pub columns: Arc<[String]>,
    pub values: Vec<Sql>,
}

impl Row {
    pub fn get(&self, column: &str) -> Option<&Sql> {
        self.columns
            .iter()
            .position(|c| c == column)
            .and_then(|i| self.values.get(i))
    }

    pub fn get_str(&self, column: &str) -> Option<&str> {
        self.get(column).and_then(Sql::as_str)
    }

    pub fn get_i64(&self, column: &str) -> Option<i64> {
        self.get(column).and_then(Sql::as_i64)
    }

    pub fn get_f64(&self, column: &str) -> Option<f64> {
        self.get(column).and_then(Sql::as_f64)
    }

    pub fn column_index(&self, column: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == column)
    }

    pub fn into_pairs(self) -> impl Iterator<Item = (String, Sql)> {
        let columns = self.columns.clone();
        self.values
            .into_iter()
            .enumerate()
            .map(move |(i, v)| (columns[i].clone(), v))
    }
}

/// Executes statements. Implemented by the SQLite and Postgres engines
/// and by [`Transaction`], so backend-agnostic code can be written once
/// against `&dyn Executor`.
#[async_trait]
pub trait Executor: Send + Sync {
    fn dialect(&self) -> Dialect;
    async fn query(&self, sql: &str, params: &[Sql]) -> DbResult<Vec<Row>>;
    async fn execute(&self, sql: &str, params: &[Sql]) -> DbResult<u64>;

    /// First row or `None`.
    async fn query_one(&self, sql: &str, params: &[Sql]) -> DbResult<Option<Row>> {
        Ok(self.query(sql, params).await?.into_iter().next())
    }

    /// First column of the first row as a string, for `SELECT COUNT(*)`
    /// and friends.
    async fn query_scalar(&self, sql: &str, params: &[Sql]) -> DbResult<Option<Sql>> {
        Ok(self
            .query_one(sql, params)
            .await?
            .and_then(|r| r.values.into_iter().next()))
    }

    /// [`query`](Executor::query), but a statement still running when
    /// `timeout` elapses is *interrupted*, not just abandoned, on every
    /// backend that can do so. `crates/server/src/routes/sql_console.rs`
    /// is the only caller today: an ad-hoc statement from a superuser is
    /// exactly the case where the caller-facing timeout in `query` was
    /// previously honest about not stopping the underlying work (see
    /// that module's own doc comment before this method existed).
    ///
    /// The default here — used by every [`Executor`] that has no
    /// cancellation primitive wired up at this layer (Postgres,
    /// [`Transaction`]) — falls back to exactly that old behavior: it
    /// only bounds the caller's *wait*, and the statement keeps running
    /// to completion (or its own driver-level timeout, if any) regardless
    /// of what this method returns. Only [`crate::sqlite::SqliteEngine`]
    /// overrides it with real interruption.
    async fn query_interruptible(
        &self,
        sql: &str,
        params: &[Sql],
        timeout: Duration,
    ) -> DbResult<Vec<Row>> {
        match tokio::time::timeout(timeout, self.query(sql, params)).await {
            Ok(result) => result,
            Err(_) => Err(DbError::Other(format!(
                "query timed out after {}s (not interrupted: this backend has no \
                 cancellation primitive wired up, see `Executor::query_interruptible`)",
                timeout.as_secs()
            ))),
        }
    }

    /// The write-statement counterpart of
    /// [`query_interruptible`](Executor::query_interruptible); same
    /// default, same caveat.
    async fn execute_interruptible(
        &self,
        sql: &str,
        params: &[Sql],
        timeout: Duration,
    ) -> DbResult<u64> {
        match tokio::time::timeout(timeout, self.execute(sql, params)).await {
            Ok(result) => result,
            Err(_) => Err(DbError::Other(format!(
                "query timed out after {}s (not interrupted: this backend has no \
                 cancellation primitive wired up, see `Executor::query_interruptible`)",
                timeout.as_secs()
            ))),
        }
    }
}

/// A backend. `Engine` is `Executor` plus transaction support and the
/// maintenance operations the services need.
#[async_trait]
pub trait Engine: Executor {
    /// Open a write transaction. Serialized on SQLite (single writer).
    async fn begin(&self) -> DbResult<Transaction>;

    /// Whether a table exists (used by schema sync and migrations).
    async fn table_exists(&self, table: &str) -> DbResult<bool>;

    /// Column names of a table or view, in order (used to derive a view
    /// collection's fields and to diff schemas).
    async fn table_columns(&self, table: &str) -> DbResult<Vec<String>>;

    /// Names of the indexes on a table.
    async fn table_indexes(&self, table: &str) -> DbResult<Vec<String>>;

    /// Backend-specific housekeeping (`PRAGMA optimize` / `VACUUM` on
    /// SQLite, `ANALYZE` on Postgres). Run by the `__pbDBOptimize__` cron.
    async fn optimize(&self) -> DbResult<()>;

    /// Produce a consistent snapshot of the database at `dest_path`
    /// (SQLite: `VACUUM INTO`; Postgres: not supported, returns an
    /// error the backup service reports).
    async fn snapshot_to(&self, dest_path: &str) -> DbResult<()>;

    /// Close every connection. Called before a backup restore swaps the
    /// data directory.
    async fn close(&self) -> DbResult<()>;

    /// Whether [`notify_realtime`](Engine::notify_realtime)/
    /// [`subscribe_realtime`](Engine::subscribe_realtime) do anything on
    /// this backend. `false` by default (SQLite: single-process, no
    /// cross-node channel to speak of); Postgres overrides this to
    /// `true`. Lets `crate::realtime::publish` skip the clone/spawn/
    /// serialize it would otherwise do on every write just to reach a
    /// no-op `await`.
    fn supports_cross_node(&self) -> bool {
        false
    }

    /// Best-effort cross-process notification for realtime fan-out (see
    /// `crates/server/src/realtime.rs`). Called once per committed write
    /// that might have subscribers *somewhere* — this instance has no
    /// way to know whether another process shares any given collection's
    /// watchers. The payload is deliberately small (record id,
    /// collection id, action, and only for a delete — where the row
    /// won't exist for anyone else to re-fetch — a snapshot of it),
    /// never blindly the record: Postgres caps a `NOTIFY` payload at
    /// 8000 bytes, and an implementation is expected to reject rather
    /// than silently truncate an oversized one. Every receiving process
    /// re-fetches the record and re-evaluates rules against its *own*
    /// current settings and rule text; this only ever says "something
    /// changed, go look", never "trust this payload".
    ///
    /// SQLite is single-node by construction: one file, one process, no
    /// second engine instance sharing it the way a Postgres connection
    /// pool is shared across app processes. The default here is a
    /// no-op — SQLite's synchronous in-process fan-out in
    /// `crate::realtime::publish` already reaches every subscriber
    /// there is, so there is nothing further to broadcast, and this
    /// method existing changes nothing about SQLite's behavior.
    async fn notify_realtime(&self, _payload: &str) -> DbResult<()> {
        Ok(())
    }

    /// Start a background subscription to every process's
    /// [`notify_realtime`](Engine::notify_realtime) calls against this
    /// same database — including this process's own, which the caller
    /// is expected to recognize and skip cheaply (its local subscribers
    /// were already reached synchronously, straight off the write) —
    /// invoking `on_notify` with each raw payload as it arrives. Meant
    /// to be called once, at startup; there is no unsubscribe and no
    /// readiness signal. Reconnects for the life of the process on
    /// connection loss, so one dropped connection never permanently
    /// kills cross-process realtime.
    ///
    /// SQLite has no cross-process channel to subscribe to (see
    /// [`notify_realtime`](Engine::notify_realtime)); the default here
    /// is a no-op that never calls `on_notify`, so a SQLite deployment's
    /// realtime behavior is exactly what it was before this method
    /// existed: whichever process handled a write is the only process
    /// that can tell its own SSE clients about it — true by definition,
    /// since SQLite deployments are single-process.
    fn subscribe_realtime(&self, _on_notify: Arc<dyn Fn(String) + Send + Sync>) {}
}

/// A write transaction. Dropping without `commit` rolls back.
pub struct Transaction {
    inner: Box<dyn TransactionImpl>,
}

#[async_trait]
pub trait TransactionImpl: Executor {
    async fn commit(self: Box<Self>) -> DbResult<()>;
    async fn rollback(self: Box<Self>) -> DbResult<()>;
}

impl Transaction {
    pub fn new(inner: Box<dyn TransactionImpl>) -> Self {
        Transaction { inner }
    }

    pub async fn commit(self) -> DbResult<()> {
        self.inner.commit().await
    }

    pub async fn rollback(self) -> DbResult<()> {
        self.inner.rollback().await
    }
}

#[async_trait]
impl Executor for Transaction {
    fn dialect(&self) -> Dialect {
        self.inner.dialect()
    }
    async fn query(&self, sql: &str, params: &[Sql]) -> DbResult<Vec<Row>> {
        self.inner.query(sql, params).await
    }
    async fn execute(&self, sql: &str, params: &[Sql]) -> DbResult<u64> {
        self.inner.execute(sql, params).await
    }
}

/// Quote an identifier for either dialect. Callers validate with
/// `cratebase_core::is_valid_identifier` first; this only wraps in
/// double quotes (PocketBase-style backticks in `indexes[]` are rewritten
/// by the schema layer).
pub fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}
