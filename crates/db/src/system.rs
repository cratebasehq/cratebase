//! The fixed system tables. Every statement is `IF NOT EXISTS`, so both
//! functions are safe to run on every boot (and are).
//!
//! Column names that PocketBase spells in camelCase (`listRule`, ...)
//! are quoted in DDL and in every query so Postgres keeps their case.

use crate::engine::Executor;
use crate::error::DbResult;

/// `_collections`, `_params` and `_migrations` on the main database.
pub async fn ensure_system_tables(ex: &dyn Executor) -> DbResult<()> {
    ex.execute(
        r#"CREATE TABLE IF NOT EXISTS "_collections" (
            "id" TEXT PRIMARY KEY NOT NULL,
            "name" TEXT NOT NULL UNIQUE,
            "type" TEXT NOT NULL DEFAULT 'base',
            "system" INTEGER NOT NULL DEFAULT 0,
            "fields" TEXT NOT NULL DEFAULT '[]',
            "indexes" TEXT NOT NULL DEFAULT '[]',
            "listRule" TEXT,
            "viewRule" TEXT,
            "createRule" TEXT,
            "updateRule" TEXT,
            "deleteRule" TEXT,
            "options" TEXT NOT NULL DEFAULT '{}',
            "created" TEXT NOT NULL DEFAULT '',
            "updated" TEXT NOT NULL DEFAULT ''
        )"#,
        &[],
    )
    .await?;
    ex.execute(
        r#"CREATE TABLE IF NOT EXISTS "_params" (
            "id" TEXT PRIMARY KEY NOT NULL,
            "key" TEXT NOT NULL UNIQUE,
            "value" TEXT,
            "created" TEXT NOT NULL DEFAULT '',
            "updated" TEXT NOT NULL DEFAULT ''
        )"#,
        &[],
    )
    .await?;
    ex.execute(
        r#"CREATE TABLE IF NOT EXISTS "_migrations" (
            "file" TEXT PRIMARY KEY NOT NULL,
            "applied" BIGINT NOT NULL DEFAULT 0
        )"#,
        &[],
    )
    .await?;
    Ok(())
}

/// `_logs` on the auxiliary (logs) database, with the two indexes the
/// dashboard's list (`-created`) and level filter need.
pub async fn ensure_logs_tables(ex: &dyn Executor) -> DbResult<()> {
    ex.execute(
        r#"CREATE TABLE IF NOT EXISTS "_logs" (
            "id" TEXT PRIMARY KEY NOT NULL,
            "level" INTEGER NOT NULL DEFAULT 0,
            "message" TEXT NOT NULL DEFAULT '',
            "data" TEXT NOT NULL DEFAULT '{}',
            "created" TEXT NOT NULL DEFAULT '',
            "updated" TEXT NOT NULL DEFAULT ''
        )"#,
        &[],
    )
    .await?;
    ex.execute(
        r#"CREATE INDEX IF NOT EXISTS "_logs_created_idx" ON "_logs" ("created")"#,
        &[],
    )
    .await?;
    ex.execute(
        r#"CREATE INDEX IF NOT EXISTS "_logs_level_idx" ON "_logs" ("level")"#,
        &[],
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::sqlite::SqliteEngine;

    #[tokio::test]
    async fn system_tables_are_idempotent() {
        let e = SqliteEngine::open_memory().unwrap();
        ensure_system_tables(&e).await.unwrap();
        ensure_system_tables(&e).await.unwrap();
        ensure_logs_tables(&e).await.unwrap();
        ensure_logs_tables(&e).await.unwrap();
        for t in ["_collections", "_params", "_migrations", "_logs"] {
            assert!(e.table_exists(t).await.unwrap(), "{t}");
        }
        let cols = e.table_columns("_collections").await.unwrap();
        assert!(cols.iter().any(|c| c == "listRule"));
        let idx = e.table_indexes("_logs").await.unwrap();
        assert!(idx.contains(&"_logs_created_idx".to_string()));
        assert!(idx.contains(&"_logs_level_idx".to_string()));
    }
}
