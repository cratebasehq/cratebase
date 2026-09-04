//! The [`Db`] handle: the main engine, the logs engine, which backend
//! the main engine is, and the process-wide collection store.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use cratebase_filter::Dialect;

use crate::backend::Backend;
use crate::collections::CollectionStore;
use crate::engine::{Engine, Executor, Row, Sql, Transaction};
use crate::error::DbResult;
use crate::postgres::PostgresEngine;
use crate::sqlite::{self, SqliteEngine};
use crate::{migrations, system};

/// File name of the logs database inside the data directory (PocketBase's
/// name, so a data directory is recognisable).
pub const AUXILIARY_DB: &str = "auxiliary.db";

/// Reader connections for the logs database: a single background writer
/// plus the occasional dashboard read.
const LOGS_READERS: usize = 2;

#[derive(Clone)]
pub struct Db {
    pub engine: Arc<dyn Engine>,
    /// Request/application logs. Always SQLite (`<data_dir>/auxiliary.db`),
    /// even when the main database is Postgres, so logging a request
    /// never contends with user traffic for the main database.
    pub logs: Arc<dyn Engine>,
    pub backend: Backend,
    pub collections: CollectionStore,
}

/// `sqlite:path`, `sqlite://path`, `sqlite::memory:` → the path without
/// scheme or query string (`":memory:"` for in-memory).
pub fn sqlite_path(database_url: &str) -> String {
    let rest = database_url
        .strip_prefix("sqlite://")
        .or_else(|| database_url.strip_prefix("sqlite:"))
        .unwrap_or(database_url);
    let path = rest.split('?').next().unwrap_or(rest);
    if path.is_empty() || path == ":memory:" {
        ":memory:".to_string()
    } else {
        path.to_string()
    }
}

impl Db {
    /// Connect to `database_url` and open the logs database under
    /// `data_dir`. A `sqlite::memory:` main database gets an in-memory
    /// logs database too (a separate engine instance), so tests never
    /// touch the disk.
    pub async fn connect(database_url: &str, data_dir: &str) -> DbResult<Db> {
        let backend = Backend::from_url(database_url)?;
        let (engine, logs): (Arc<dyn Engine>, Arc<dyn Engine>) = match backend {
            Backend::Sqlite => {
                let path = sqlite_path(database_url);
                if path == ":memory:" {
                    let main = open_sqlite(":memory:", 0).await?;
                    let logs = open_sqlite(":memory:", 0).await?;
                    (Arc::new(main), Arc::new(logs))
                } else {
                    let main = open_sqlite(&path, sqlite::default_readers()).await?;
                    let logs = open_logs(data_dir).await?;
                    (Arc::new(main), Arc::new(logs))
                }
            }
            Backend::Postgres => {
                let main =
                    PostgresEngine::connect(database_url, crate::postgres::DEFAULT_POOL_SIZE)
                        .await?;
                let logs = open_logs(data_dir).await?;
                (Arc::new(main), Arc::new(logs))
            }
        };
        Ok(Db::from_engines(engine, logs, backend))
    }

    /// Assemble a handle from already-open engines (tests, embedders).
    pub fn from_engines(engine: Arc<dyn Engine>, logs: Arc<dyn Engine>, backend: Backend) -> Db {
        Db {
            engine,
            logs,
            backend,
            collections: CollectionStore::new(),
        }
    }

    /// An in-memory SQLite database, bootstrapped. For tests.
    pub async fn memory() -> DbResult<Db> {
        let db = Db::connect("sqlite::memory:", "").await?;
        db.bootstrap().await?;
        Ok(db)
    }

    /// Create the system tables on both engines, run the core migrations
    /// (which seed `_superusers`, `users` and the `_*` system
    /// collections) and load the collection store.
    pub async fn bootstrap(&self) -> DbResult<()> {
        system::ensure_system_tables(&*self.engine).await?;
        system::ensure_logs_tables(&*self.logs).await?;
        self.collections.load(&*self.engine).await?;
        migrations::Runner::core().up(self).await?;
        self.collections.load(&*self.engine).await?;
        Ok(())
    }

    pub fn dialect(&self) -> Dialect {
        self.backend.dialect()
    }

    /// A write transaction on the main engine.
    pub async fn begin(&self) -> DbResult<Transaction> {
        self.engine.begin().await
    }

    /// Close both engines (before a backup restore swaps the data dir).
    pub async fn close(&self) -> DbResult<()> {
        self.engine.close().await?;
        self.logs.close().await
    }
}

/// `Db` is itself an executor on the main engine, so
/// `db.query(..)` reads like `tx.query(..)`.
#[async_trait]
impl Executor for Db {
    fn dialect(&self) -> Dialect {
        self.backend.dialect()
    }

    async fn query(&self, sql: &str, params: &[Sql]) -> DbResult<Vec<Row>> {
        self.engine.query(sql, params).await
    }

    async fn execute(&self, sql: &str, params: &[Sql]) -> DbResult<u64> {
        self.engine.execute(sql, params).await
    }
}

async fn open_sqlite(path: &str, readers: usize) -> DbResult<SqliteEngine> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || SqliteEngine::open(&path, readers)).await?
}

async fn open_logs(data_dir: &str) -> DbResult<SqliteEngine> {
    let path = Path::new(data_dir).join(AUXILIARY_DB);
    open_sqlite(&path.to_string_lossy(), LOGS_READERS).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sqlite_urls() {
        assert_eq!(sqlite_path("sqlite::memory:"), ":memory:");
        assert_eq!(sqlite_path("sqlite://:memory:"), ":memory:");
        assert_eq!(sqlite_path("sqlite:"), ":memory:");
        assert_eq!(sqlite_path("sqlite:data/x.db?mode=rwc"), "data/x.db");
        assert_eq!(sqlite_path("sqlite:///tmp/x.db"), "/tmp/x.db");
    }

    #[tokio::test]
    async fn memory_db_bootstraps_with_system_collections() {
        let db = Db::memory().await.unwrap();
        assert_eq!(db.backend, Backend::Sqlite);
        let snap = db.collections.all();
        let names: Vec<&str> = snap.all.iter().map(|c| c.name.as_str()).collect();
        for n in [
            "_superusers",
            "users",
            "_externalAuths",
            "_mfas",
            "_otps",
            "_authOrigins",
            "_cron_jobs",
            "_webhooks",
            "_llm_usage",
        ] {
            assert!(names.contains(&n), "{n} missing from {names:?}");
        }
        assert!(db.engine.table_exists("users").await.unwrap());
        assert!(db.logs.table_exists("_logs").await.unwrap());
        // Bootstrapping twice is a no-op.
        db.bootstrap().await.unwrap();
        assert_eq!(db.collections.all().all.len(), snap.all.len());
    }

    #[tokio::test]
    async fn file_db_creates_auxiliary_db_in_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().to_string_lossy().to_string();
        let url = format!("sqlite:{}/data.db", data);
        let db = Db::connect(&url, &data).await.unwrap();
        db.bootstrap().await.unwrap();
        assert!(dir.path().join("data.db").exists());
        assert!(dir.path().join(AUXILIARY_DB).exists());
        db.close().await.unwrap();
    }
}
