use sqlx::any::{AnyConnectOptions, AnyPoolOptions};
use sqlx::AnyPool;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::backend::Backend;
use crate::collections::CollectionCache;
use crate::error::DbResult;

/// A connected database: the runtime-polymorphic sqlx pool plus which
/// concrete backend it talks to (needed for DDL/type generation that sqlx's
/// `Any` driver deliberately doesn't abstract over).
#[derive(Clone)]
pub struct Db {
    pub pool: AnyPool,
    /// Where `_request_logs` lives. On a file-backed SQLite database this
    /// is a *separate* database file (`<main>.logs.db`) so that logging a
    /// request never takes the main database's single writer lock — the
    /// request log is written on every API call, reads included, and
    /// sharing the primary file turned every read-only endpoint into a
    /// write transaction (measured in `benchmarks/` as the largest single
    /// contributor to the `search` gap vs PocketBase, which keeps its
    /// logs in `auxiliary.db` for exactly this reason). On Postgres and on
    /// `:memory:` SQLite it is the same pool as `pool`.
    pub logs: AnyPool,
    pub backend: Backend,
    /// Process-wide cache of collection metadata; see
    /// [`crate::collections::CollectionCache`].
    pub collections: Arc<CollectionCache>,
}

/// Size of the SQLite connection pool for a file-backed database: one
/// connection per available core, bounded to a sane range, unless
/// `DATABASE_MAX_CONNECTIONS` says otherwise.
fn sqlite_pool_size() -> u32 {
    if let Some(n) = std::env::var("DATABASE_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|n| *n > 0)
    {
        return n;
    }
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4)
        .clamp(4, 16)
}

/// Rewrite a `sqlite:` URL so it points at a sibling file with the given
/// suffix appended to the database filename (`foo.db` -> `foo.db.logs.db`),
/// preserving any query string. Returns `None` for `:memory:`.
fn sqlite_sibling_url(database_url: &str, suffix: &str) -> Option<String> {
    let (prefix, rest) = if let Some(r) = database_url.strip_prefix("sqlite://") {
        ("sqlite://", r)
    } else {
        ("sqlite:", database_url.strip_prefix("sqlite:")?)
    };
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };
    if path == ":memory:" || path.is_empty() {
        return None;
    }
    let mut url = format!("{prefix}{path}{suffix}");
    if let Some(q) = query {
        url.push('?');
        url.push_str(q);
    }
    Some(url)
}

async fn open_pool(
    backend: Backend,
    database_url: &str,
    max_connections: u32,
) -> DbResult<AnyPool> {
    let opts = AnyConnectOptions::from_str(database_url)?;
    let is_sqlite = backend == Backend::Sqlite;
    let pool = AnyPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(10))
        // sqlx's default pings every connection on every acquire. For a
        // local SQLite file that is a cross-thread channel round trip to
        // the connection's worker thread that can never fail in a way the
        // next statement wouldn't also surface — pure overhead, paid
        // several times per request. Postgres keeps the ping: a dropped
        // TCP connection is a real thing there.
        .test_before_acquire(!is_sqlite)
        .after_connect(move |conn, _meta| {
            Box::pin(async move {
                if !is_sqlite {
                    return Ok(());
                }
                // `after_connect` fires once per *physical* connection
                // the pool opens, unlike a one-off
                // `sqlx::query(...).execute(&pool)` after `connect_with`
                // returns — that only configures whichever single
                // connection happened to serve that query.
                use sqlx::Executor;
                conn.execute("PRAGMA journal_mode = WAL").await?;
                conn.execute("PRAGMA foreign_keys = ON").await?;
                conn.execute("PRAGMA busy_timeout = 5000").await?;
                // NORMAL is safe under WAL (no corruption on power loss,
                // at most the last transactions are rolled back) and
                // avoids an fsync per write.
                conn.execute("PRAGMA synchronous = NORMAL").await?;
                // 16 MiB page cache per connection (default 2 MiB).
                conn.execute("PRAGMA cache_size = -16000").await?;
                // Sort/temp b-trees in memory: every default list query
                // sorts by `created`, so the sorter is on the hot path.
                conn.execute("PRAGMA temp_store = MEMORY").await?;
                // Memory-map up to 256 MiB of the file so page reads skip
                // the read() syscall.
                conn.execute("PRAGMA mmap_size = 268435456").await?;
                conn.execute("PRAGMA journal_size_limit = 200000000")
                    .await?;
                Ok(())
            })
        })
        .connect_with(opts)
        .await?;
    Ok(pool)
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

        // sqlx pools each connection independently; a `:memory:` SQLite
        // connection is a fresh, isolated database per connection unless
        // using a shared-cache URI, so a pool size above 1 for `:memory:`
        // would silently scatter data across disconnected in-memory DBs
        // (this is exactly what the test harness relies on staying at 1).
        // A real file-backed database has no such constraint: WAL mode
        // lets many readers run alongside one writer.
        let is_sqlite_memory = backend == Backend::Sqlite && database_url.contains(":memory:");
        let max_connections = match backend {
            Backend::Sqlite if is_sqlite_memory => 1,
            Backend::Sqlite => sqlite_pool_size(),
            _ => 10,
        };

        let pool = open_pool(backend, &database_url, max_connections).await?;

        let logs = match backend {
            Backend::Sqlite if !is_sqlite_memory => {
                match sqlite_sibling_url(&database_url, ".logs.db") {
                    // Logging is a single background writer plus the
                    // occasional dashboard read; two connections is
                    // plenty.
                    Some(url) => open_pool(backend, &url, 2).await?,
                    None => pool.clone(),
                }
            }
            _ => pool.clone(),
        };

        Ok(Db {
            pool,
            logs,
            backend,
            collections: Arc::new(CollectionCache::default()),
        })
    }

    pub fn dialect(&self) -> cratebase_filter::Dialect {
        self.backend.dialect()
    }
}
