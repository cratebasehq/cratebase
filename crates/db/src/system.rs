use crate::error::DbResult;
use crate::pool::Db;

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

    Ok(())
}
