use sqlx::any::{AnyConnectOptions, AnyPoolOptions};
use sqlx::AnyPool;
use std::str::FromStr;
use std::time::Duration;

use crate::backend::Backend;
use crate::error::DbResult;

/// A connected database: the runtime-polymorphic sqlx pool plus which
/// concrete backend it talks to (needed for DDL/type generation that sqlx's
/// `Any` driver deliberately doesn't abstract over).
#[derive(Clone)]
pub struct Db {
    pub pool: AnyPool,
    pub backend: Backend,
}

impl Db {
    /// Connect using a `sqlite:./data/cratebase.db` or
    /// `postgres://user:pass@host/db` URL. Creates the sqlite file (and its
    /// parent directory) if missing.
    pub async fn connect(database_url: &str) -> DbResult<Self> {
        sqlx::any::install_default_drivers();
        let backend = Backend::from_url(database_url)?;

        let mut database_url = database_url.to_string();
        if backend == Backend::Sqlite {
            if let Some(path) = database_url
                .strip_prefix("sqlite://")
                .or_else(|| database_url.strip_prefix("sqlite:"))
            {
                let path = path.split('?').next().unwrap_or(path).to_string();
                if path != ":memory:" && !path.is_empty() {
                    if let Some(parent) = std::path::Path::new(&path).parent() {
                        if !parent.as_os_str().is_empty() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                    }
                }
            }
            // Auto-create the database file on first run unless the caller
            // already specified an explicit sqlite `mode`.
            if !database_url.contains("mode=") {
                let sep = if database_url.contains('?') { "&" } else { "?" };
                database_url = format!("{database_url}{sep}mode=rwc");
            }
        }

        let opts = AnyConnectOptions::from_str(&database_url)?;

        // sqlx pools each connection independently; a `:memory:` SQLite
        // connection is a fresh, isolated database per connection unless
        // using a shared-cache URI, so a pool size above 1 for `:memory:`
        // would silently scatter data across disconnected in-memory DBs
        // (this is exactly what the test harness relies on staying at 1).
        // A real file-backed database has no such constraint: WAL mode
        // (enabled below) lets many readers run alongside one writer, so
        // capping it at 1 the same way serializes every request on a
        // single connection for no reason — measured in `benchmarks/` as
        // throughput that doesn't scale with concurrency at all.
        let is_sqlite_memory = backend == Backend::Sqlite && database_url.contains(":memory:");
        let max_connections = match backend {
            Backend::Sqlite if is_sqlite_memory => 1,
            Backend::Sqlite => 5,
            _ => 10,
        };

        let pool = AnyPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(opts)
            .await?;

        if backend == Backend::Sqlite {
            sqlx::query("PRAGMA journal_mode = WAL")
                .execute(&pool)
                .await
                .ok();
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&pool)
                .await
                .ok();
            // With more than one connection now able to attempt a write
            // (e.g. two concurrent record updates), a brief lock
            // conflict should block-and-retry rather than fail
            // immediately with SQLITE_BUSY.
            sqlx::query("PRAGMA busy_timeout = 5000")
                .execute(&pool)
                .await
                .ok();
        }

        Ok(Db { pool, backend })
    }

    pub fn dialect(&self) -> cratebase_filter::Dialect {
        self.backend.dialect()
    }
}
