//! The migrations ledger (`_migrations(file, applied)`) and the runner
//! for core (Rust) migrations.
//!
//! Core migrations are registered in order on a [`Runner`]; JS
//! migrations (`pb_migrations/*.js`) are executed by the JS runtime and
//! recorded through the same ledger via [`mark_applied`] /
//! [`mark_reverted`], so `migrate history-sync` and the dashboard see
//! one list.
//!
//! A migration receives the whole [`Db`] because the seed migration
//! goes through [`crate::CollectionStore`], which manages its own
//! transactions; a migration that wants atomicity opens one itself with
//! `db.begin()`.

use chrono::Utc;
use futures::future::BoxFuture;

use cratebase_core::Collection;

use crate::db::Db;
use crate::engine::{Executor, Sql};
use crate::error::{DbError, DbResult};

pub type MigrationFn = Box<dyn for<'a> Fn(&'a Db) -> BoxFuture<'a, DbResult<()>> + Send + Sync>;

pub struct Migration {
    pub file: String,
    pub up: MigrationFn,
    pub down: MigrationFn,
}

impl Migration {
    pub fn new(file: impl Into<String>, up: MigrationFn, down: MigrationFn) -> Self {
        Migration {
            file: file.into(),
            up,
            down,
        }
    }
}

/// Registered migrations, applied in registration order.
#[derive(Default)]
pub struct Runner {
    migrations: Vec<Migration>,
}

impl Runner {
    pub fn new() -> Self {
        Self::default()
    }

    /// The built-in migrations every database gets.
    pub fn core() -> Self {
        let mut r = Runner::new();
        r.register(Migration::new(
            INIT_SYSTEM,
            Box::new(|db| Box::pin(init_system_up(db))),
            Box::new(|db| Box::pin(init_system_down(db))),
        ));
        r
    }

    pub fn register(&mut self, migration: Migration) {
        self.migrations.push(migration);
    }

    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.migrations.iter().map(|m| m.file.as_str())
    }

    /// Apply every registered migration not yet in the ledger. Returns
    /// the files applied.
    pub async fn up(&self, db: &Db) -> DbResult<Vec<String>> {
        let done = applied(db).await?;
        let mut ran = Vec::new();
        for m in &self.migrations {
            if done.iter().any(|(f, _)| f == &m.file) {
                continue;
            }
            tracing::info!(file = %m.file, "applying migration");
            (m.up)(db).await?;
            mark_applied(db, &m.file).await?;
            ran.push(m.file.clone());
        }
        Ok(ran)
    }

    /// Revert the last `n` applied migrations (most recent first).
    /// Every reverted file must be registered; an unknown one is an
    /// error rather than a silent skip.
    pub async fn down(&self, db: &Db, n: usize) -> DbResult<Vec<String>> {
        let mut done = applied(db).await?;
        done.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
        let mut reverted = Vec::new();
        for (file, _) in done.into_iter().take(n) {
            let m = self
                .migrations
                .iter()
                .find(|m| m.file == file)
                .ok_or_else(|| DbError::Other(format!("migration {file} is not registered")))?;
            tracing::info!(file = %file, "reverting migration");
            (m.down)(db).await?;
            mark_reverted(db, &file).await?;
            reverted.push(file);
        }
        Ok(reverted)
    }

    /// Registered migrations plus any ledger-only rows, with their
    /// applied timestamp (unix seconds) when applied.
    pub async fn list(&self, db: &Db) -> DbResult<Vec<(String, Option<i64>)>> {
        let done = applied(db).await?;
        let mut out: Vec<(String, Option<i64>)> = self
            .migrations
            .iter()
            .map(|m| {
                let at = done.iter().find(|(f, _)| f == &m.file).map(|(_, t)| *t);
                (m.file.clone(), at)
            })
            .collect();
        for (file, at) in done {
            if !out.iter().any(|(f, _)| f == &file) {
                out.push((file, Some(at)));
            }
        }
        Ok(out)
    }
}

/// Every ledger row as `(file, applied unix seconds)`, in file order.
pub async fn applied(ex: &dyn Executor) -> DbResult<Vec<(String, i64)>> {
    let rows = ex
        .query(
            r#"SELECT "file", "applied" FROM "_migrations" ORDER BY "file""#,
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get_str("file").unwrap_or("").to_string(),
                r.get_i64("applied").unwrap_or(0),
            )
        })
        .collect())
}

pub async fn is_applied(ex: &dyn Executor, file: &str) -> DbResult<bool> {
    Ok(ex
        .query_one(
            r#"SELECT 1 FROM "_migrations" WHERE "file" = $1"#,
            &[Sql::from(file)],
        )
        .await?
        .is_some())
}

/// Record `file` as applied now. Idempotent.
pub async fn mark_applied(ex: &dyn Executor, file: &str) -> DbResult<()> {
    ex.execute(
        r#"INSERT INTO "_migrations" ("file", "applied") VALUES ($1, $2)
           ON CONFLICT ("file") DO UPDATE SET "applied" = excluded."applied""#,
        &[Sql::from(file), Sql::Int(Utc::now().timestamp())],
    )
    .await?;
    Ok(())
}

/// Remove `file` from the ledger.
pub async fn mark_reverted(ex: &dyn Executor, file: &str) -> DbResult<()> {
    ex.execute(
        r#"DELETE FROM "_migrations" WHERE "file" = $1"#,
        &[Sql::from(file)],
    )
    .await?;
    Ok(())
}

/// Drop ledger rows whose file is no longer known (deleted from
/// `pb_migrations/` or unregistered). Returns the removed files.
pub async fn history_sync(ex: &dyn Executor, known_files: &[String]) -> DbResult<Vec<String>> {
    let mut removed = Vec::new();
    for (file, _) in applied(ex).await? {
        if !known_files.contains(&file) {
            mark_reverted(ex, &file).await?;
            removed.push(file);
        }
    }
    Ok(removed)
}

// --- core migrations -----------------------------------------------------

pub const INIT_SYSTEM: &str = "1_init_system.rs";

/// The collections PocketBase seeds on first run, in creation order.
fn seed_collections() -> Vec<Collection> {
    let mut all = vec![
        Collection::default_superusers(),
        Collection::default_users(),
    ];
    all.extend(Collection::default_system_collections());
    all
}

async fn init_system_up(db: &Db) -> DbResult<()> {
    for c in seed_collections() {
        if db.collections.get_by_name(&c.name).is_some() {
            continue;
        }
        db.collections.insert(&*db.engine, &c).await?;
    }
    Ok(())
}

async fn init_system_down(db: &Db) -> DbResult<()> {
    for c in seed_collections().iter().rev() {
        if db.collections.get_by_name(&c.name).is_some() {
            db.collections.delete(&*db.engine, &c.name).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    async fn fresh() -> Db {
        let db = Db::connect("sqlite::memory:", "").await.unwrap();
        crate::system::ensure_system_tables(&*db.engine)
            .await
            .unwrap();
        db
    }

    #[tokio::test]
    async fn init_seeds_system_collections_with_fixture_ids() {
        let db = fresh().await;
        let ran = Runner::core().up(&db).await.unwrap();
        assert_eq!(ran, vec![INIT_SYSTEM.to_string()]);
        assert_eq!(
            db.collections.get("_superusers").unwrap().id,
            "pbc_3142635823"
        );
        assert_eq!(db.collections.get("users").unwrap().id, "_pb_users_auth_");
        assert_eq!(db.collections.get("_mfas").unwrap().id, "pbc_2279338944");
        assert_eq!(db.collections.get("_otps").unwrap().id, "pbc_1638494021");
        assert!(db.collections.get("_externalAuths").is_some());
        assert!(db.collections.get("_authOrigins").is_some());
        assert!(db.collections.get("_superusers").unwrap().system);
        for t in [
            "_superusers",
            "users",
            "_mfas",
            "_otps",
            "_externalAuths",
            "_authOrigins",
        ] {
            assert!(db.engine.table_exists(t).await.unwrap(), "{t}");
        }
        // Second run is a no-op.
        assert!(Runner::core().up(&db).await.unwrap().is_empty());
        assert!(is_applied(&db, INIT_SYSTEM).await.unwrap());

        let reverted = Runner::core().down(&db, 1).await.unwrap();
        assert_eq!(reverted, vec![INIT_SYSTEM.to_string()]);
        assert!(db.collections.is_empty());
        assert!(!db.engine.table_exists("users").await.unwrap());
        assert!(!is_applied(&db, INIT_SYSTEM).await.unwrap());
    }

    #[tokio::test]
    async fn ledger_up_down_list_and_history_sync() {
        let db = fresh().await;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut runner = Runner::new();
        for name in ["1_a.js", "2_b.js"] {
            let up = counter.clone();
            let down = counter.clone();
            runner.register(Migration::new(
                name,
                Box::new(move |_| {
                    up.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async { Ok(()) })
                }),
                Box::new(move |_| {
                    down.fetch_sub(1, Ordering::SeqCst);
                    Box::pin(async { Ok(()) })
                }),
            ));
        }
        assert_eq!(runner.up(&db).await.unwrap(), vec!["1_a.js", "2_b.js"]);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
        let listed = runner.list(&db).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|(_, at)| at.is_some()));

        assert_eq!(runner.down(&db, 1).await.unwrap(), vec!["2_b.js"]);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert_eq!(runner.list(&db).await.unwrap()[1].1, None);

        // A JS migration recorded by another runtime shows in the list
        // and is pruned by history_sync when its file is gone.
        mark_applied(&db, "3_js.js").await.unwrap();
        let listed = runner.list(&db).await.unwrap();
        assert_eq!(listed.len(), 3);
        let err = runner.down(&db, 1).await.unwrap_err();
        assert!(matches!(err, DbError::Other(_)));
        let removed = history_sync(&db, &["1_a.js".to_string()]).await.unwrap();
        assert_eq!(removed, vec!["3_js.js"]);
        assert_eq!(applied(&db).await.unwrap().len(), 1);
        mark_reverted(&db, "1_a.js").await.unwrap();
        assert!(applied(&db).await.unwrap().is_empty());
    }
}
