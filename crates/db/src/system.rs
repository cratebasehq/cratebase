use std::sync::atomic::{AtomicU64, Ordering};

use cratebase_core::{new_id, now, AuthOptions, Collection, CollectionType};
use sqlx::any::AnyRow;
use sqlx::Row;

use crate::collections;
use crate::error::{DbError, DbResult};
use crate::pool::Db;

/// Most recent request-log rows kept around. Pruned back to this count
/// periodically (see [`insert_request_log`]) rather than left to grow
/// unbounded - a request log is a debugging aid, not a permanent audit
/// trail, so trimming lossy history is the right trade-off (mirrors
/// PocketBase's own capped `_requests` table).
const REQUEST_LOG_CAP: i64 = 5000;

/// How many inserts between prune passes. The prune query is a full
/// table scan (`... WHERE id NOT IN (SELECT ... ORDER BY created DESC
/// LIMIT N)`) plus a second write statement holding SQLite's one
/// writer lock right after the insert - cheap once, but running it on
/// *every* request logged doubled the write-lock hold time (and wall
/// clock cost) of every single API call, measured as a genuine
/// throughput regression in `benchmarks/`. The table can grow up to
/// `REQUEST_LOG_CAP + PRUNE_EVERY` rows between passes, which is a
/// fine trade for a debugging aid.
const PRUNE_EVERY: u64 = 256;

static INSERT_COUNT: AtomicU64 = AtomicU64::new(0);

/// Create the two fixed system tables Cratebase needs before any
/// user-defined collection exists: collection metadata and admin accounts.
/// Both are plain `IF NOT EXISTS` DDL so this is safe to run on every boot.
pub async fn ensure_system_tables(db: &Db) -> DbResult<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _collections (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            type TEXT NOT NULL,
            schema TEXT NOT NULL,
            list_rule TEXT,
            view_rule TEXT,
            create_rule TEXT,
            update_rule TEXT,
            delete_rule TEXT,
            auth_options TEXT NOT NULL,
            view_query TEXT,
            created TEXT NOT NULL,
            updated TEXT NOT NULL
        )",
    )
    .execute(&db.pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _admins (
            id TEXT PRIMARY KEY,
            email TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            created TEXT NOT NULL,
            updated TEXT NOT NULL
        )",
    )
    .execute(&db.pool)
    .await?;

    // Links an OAuth2 identity (provider + that provider's own user id) to
    // one auth-collection record. A record can have several linked
    // providers; a given (collection, provider, provider_user_id) maps to
    // at most one record.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _external_auths (
            id TEXT PRIMARY KEY,
            collection_id TEXT NOT NULL,
            record_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            provider_user_id TEXT NOT NULL,
            created TEXT NOT NULL
        )",
    )
    .execute(&db.pool)
    .await?;
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_external_auths_identity
         ON _external_auths (collection_id, provider, provider_user_id)",
    )
    .execute(&db.pool)
    .await?;

    // Hashed one-time codes for the OTP-login and MFA-second-factor flows
    // (`crates/db/src/otp.rs`). Keyed by `(collection_id, record_id)` so a
    // record can only have one live batch of codes at a time; short-lived
    // and single-use, unlike `_external_auths` this table is meant to
    // empty out again as codes expire or get consumed.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _otp_codes (
            id TEXT PRIMARY KEY,
            collection_id TEXT NOT NULL,
            record_id TEXT NOT NULL,
            code_hash TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            created TEXT NOT NULL
        )",
    )
    .execute(&db.pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_otp_codes_record
         ON _otp_codes (collection_id, record_id)",
    )
    .execute(&db.pool)
    .await?;

    Ok(())
}

/// One HTTP request/response pair, captured by the logging middleware in
/// `cratebase-server` after the handler has produced a response.
pub struct RequestLogEntry {
    pub method: String,
    pub path: String,
    pub status: i64,
    pub duration_ms: i64,
    /// Caller identity, if the request carried a valid bearer token.
    /// `None` for both fields on an anonymous request; `auth_collection_id`
    /// is `None` for a superuser (admins aren't records in a collection).
    pub auth_id: Option<String>,
    pub auth_collection_id: Option<String>,
}

pub struct RequestLogPage {
    pub items: Vec<serde_json::Value>,
    pub page: i64,
    pub per_page: i64,
    pub total_items: i64,
    pub total_pages: i64,
}

/// Bounded log of recent API requests, written by the logging middleware
/// and read by the superuser-only `GET /api/logs` dashboard page. Kept
/// separate from `_collections`-backed tables since it's server-internal
/// bookkeeping, not user schema.
pub async fn ensure_request_logs_table(db: &Db) -> DbResult<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _request_logs (
            id TEXT PRIMARY KEY,
            method TEXT NOT NULL,
            path TEXT NOT NULL,
            status INTEGER NOT NULL,
            duration_ms INTEGER NOT NULL,
            auth_id TEXT,
            auth_collection_id TEXT,
            created TEXT NOT NULL
        )",
    )
    .execute(&db.pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_request_logs_created ON _request_logs (created DESC)")
        .execute(&db.pool)
        .await?;
    Ok(())
}

/// Records one request, pruning the table back down to
/// [`REQUEST_LOG_CAP`] roughly every [`PRUNE_EVERY`] inserts (see its
/// doc comment for why not on every insert).
pub async fn insert_request_log(db: &Db, entry: RequestLogEntry) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO _request_logs
            (id, method, path, status, duration_ms, auth_id, auth_collection_id, created)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(new_id())
    .bind(entry.method)
    .bind(entry.path)
    .bind(entry.status)
    .bind(entry.duration_ms)
    .bind(entry.auth_id)
    .bind(entry.auth_collection_id)
    .bind(now())
    .execute(&db.pool)
    .await?;

    if INSERT_COUNT.fetch_add(1, Ordering::Relaxed) % PRUNE_EVERY == 0 {
        sqlx::query(
            "DELETE FROM _request_logs WHERE id NOT IN (
                SELECT id FROM _request_logs ORDER BY created DESC LIMIT $1
            )",
        )
        .bind(REQUEST_LOG_CAP)
        .execute(&db.pool)
        .await?;
    }

    Ok(())
}

fn row_to_request_log(row: &AnyRow) -> DbResult<serde_json::Value> {
    Ok(serde_json::json!({
        "id": row.try_get::<String, _>("id")?,
        "method": row.try_get::<String, _>("method")?,
        "path": row.try_get::<String, _>("path")?,
        "status": row.try_get::<i64, _>("status")?,
        "durationMs": row.try_get::<i64, _>("duration_ms")?,
        "authId": row.try_get::<Option<String>, _>("auth_id")?,
        "authCollectionId": row.try_get::<Option<String>, _>("auth_collection_id")?,
        "created": row.try_get::<String, _>("created")?,
    }))
}

/// Lists recent request-log rows, newest first. `filter` is a plain
/// substring match against `path` — the full `cratebase-filter` grammar
/// resolves against a `Collection`'s schema, which this system table
/// deliberately doesn't have, so a simple `LIKE` covers "find requests to
/// /api/collections/foo" without building a second resolver just for this
/// one table.
pub async fn list_request_logs(
    db: &Db,
    page: i64,
    per_page: i64,
    filter: Option<&str>,
) -> DbResult<RequestLogPage> {
    let page = page.max(1);
    let per_page = per_page.clamp(1, 500);
    let offset = (page - 1) * per_page;

    let where_clause = if filter.is_some() { " WHERE path LIKE $1" } else { "" };
    let like_pattern = filter.map(|f| format!("%{f}%"));

    let count_sql = format!("SELECT COUNT(*) FROM _request_logs{where_clause}");
    let total_items: i64 = if let Some(p) = &like_pattern {
        sqlx::query_scalar(&count_sql).bind(p).fetch_one(&db.pool).await?
    } else {
        sqlx::query_scalar(&count_sql).fetch_one(&db.pool).await?
    };

    let list_sql = if filter.is_some() {
        format!("SELECT * FROM _request_logs{where_clause} ORDER BY created DESC LIMIT $2 OFFSET $3")
    } else {
        format!("SELECT * FROM _request_logs{where_clause} ORDER BY created DESC LIMIT $1 OFFSET $2")
    };
    let rows = if let Some(p) = &like_pattern {
        sqlx::query(&list_sql)
            .bind(p)
            .bind(per_page)
            .bind(offset)
            .fetch_all(&db.pool)
            .await?
    } else {
        sqlx::query(&list_sql)
            .bind(per_page)
            .bind(offset)
            .fetch_all(&db.pool)
            .await?
    };
    let items = rows
        .iter()
        .map(row_to_request_log)
        .collect::<DbResult<Vec<_>>>()?;

    let total_pages = if total_items == 0 {
        0
    } else {
        (total_items + per_page - 1) / per_page
    };

    Ok(RequestLogPage {
        items,
        page,
        per_page,
        total_items,
        total_pages,
    })
}

/// Seed the `users` auth collection on first boot: every project gets a
/// working sign-in table without an operator ever choosing "base or auth"
/// for it themselves. No-op once it exists.
pub async fn ensure_default_collections(db: &Db) -> DbResult<()> {
    match collections::get_collection_by_name(db, "users").await {
        Ok(_) => Ok(()),
        Err(DbError::NotFound) => {
            let now = now();
            let users = Collection {
                id: new_id(),
                name: "users".into(),
                collection_type: CollectionType::Auth,
                schema: vec![],
                list_rule: Some("id = @request.auth.id".into()),
                view_rule: Some("id = @request.auth.id".into()),
                create_rule: Some(String::new()),
                update_rule: Some("id = @request.auth.id".into()),
                delete_rule: Some("id = @request.auth.id".into()),
                auth_options: AuthOptions::default(),
                view_query: None,
                created: now.clone(),
                updated: now,
            };
            collections::create_collection(db, &users).await
        }
        Err(e) => Err(e),
    }
}

/// Runs SQLite's `VACUUM INTO` to produce a consistent snapshot of the
/// whole database at `dest_path`, used by the backups feature. Lives here
/// (rather than as a raw query in `cratebase-server`) so callers outside
/// this crate never need `sqlx` as a direct dependency just to issue one
/// statement. `dest_path` is always a server-generated temp path, never
/// user input — see `crate::routes::backups`'s call site for why that
/// makes plain string interpolation safe here.
pub async fn vacuum_into(db: &Db, dest_path: &str) -> DbResult<()> {
    let escaped = dest_path.replace('\'', "''");
    sqlx::query(&format!("VACUUM INTO '{escaped}'"))
        .execute(&db.pool)
        .await?;
    Ok(())
}
