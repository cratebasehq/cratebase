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

        let pool = AnyPoolOptions::new()
            .max_connections(if backend == Backend::Sqlite { 1 } else { 10 })
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
        }

        Ok(Db { pool, backend })
    }

    pub fn dialect(&self) -> cratebase_filter::Dialect {
        self.backend.dialect()
    }
}
