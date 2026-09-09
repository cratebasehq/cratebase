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

use cratebase_core::{Collection, Field};

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
        // `_cron_jobs` was added to `default_system_collections()` after
        // `INIT_SYSTEM` had already shipped and run on real databases —
        // that function only inserts collections `INIT_SYSTEM` itself
        // doesn't already know about, so a database bootstrapped before
        // this change would otherwise never get the new table. A fresh
        // database already has `_cron_jobs` from `INIT_SYSTEM` and this
        // migration is a no-op there; an existing one gets it added here.
        r.register(Migration::new(
            ADD_CRON_JOBS,
            Box::new(|db| Box::pin(add_cron_jobs_up(db))),
            Box::new(|db| Box::pin(add_cron_jobs_down(db))),
        ));
        // `_webhooks` follows the same story as `_cron_jobs` immediately
        // above: it was added to `default_system_collections()` after
        // `INIT_SYSTEM` shipped, so an existing database needs this
        // follow-up migration to retroactively get the table. A fresh
        // database already has `_webhooks` from `INIT_SYSTEM` and this
        // migration is a no-op there.
        r.register(Migration::new(
            ADD_WEBHOOKS,
            Box::new(|db| Box::pin(add_webhooks_up(db))),
            Box::new(|db| Box::pin(add_webhooks_down(db))),
        ));
        // `_teams`/`_team_members` follow the same story as `_cron_jobs`
        // and `_webhooks` immediately above: added to
        // `default_system_collections()` after `INIT_SYSTEM` shipped, so
        // an existing database needs this follow-up migration to
        // retroactively get both tables. A fresh database already has
        // them from `INIT_SYSTEM` and this migration is a no-op there.
        r.register(Migration::new(
            ADD_TEAMS,
            Box::new(|db| Box::pin(add_teams_up(db))),
            Box::new(|db| Box::pin(add_teams_down(db))),
        ));
        // `_llm_usage` follows the same story as `_cron_jobs`,
        // `_webhooks` and `_teams` immediately above: added to
        // `default_system_collections()` after `INIT_SYSTEM` shipped, so
        // an existing database needs this follow-up migration to
        // retroactively get the table. A fresh database already has it
        // from `INIT_SYSTEM` and this migration is a no-op there.
        r.register(Migration::new(
            ADD_LLM_USAGE,
            Box::new(|db| Box::pin(add_llm_usage_up(db))),
            Box::new(|db| Box::pin(add_llm_usage_down(db))),
        ));
        // Same story again for `_api_keys` and `_push_subscriptions`.
        r.register(Migration::new(
            ADD_API_KEYS,
            Box::new(|db| Box::pin(add_api_keys_up(db))),
            Box::new(|db| Box::pin(add_api_keys_down(db))),
        ));
        r.register(Migration::new(
            ADD_PUSH_SUBSCRIPTIONS,
            Box::new(|db| Box::pin(add_push_subscriptions_up(db))),
            Box::new(|db| Box::pin(add_push_subscriptions_down(db))),
        ));
        // `_superusers.role` was added after `INIT_SYSTEM` shipped, same
        // story as every migration above — except this one alters an
        // *existing* system collection's schema instead of adding a
        // whole new one, so a fresh database (whose `default_superusers`
        // already has the field) is a no-op here, while an existing
        // database needs the column added *and* every pre-existing row
        // backfilled to `"owner"` — the column's own zero default
        // (`''`) is not a valid role, and defaulting to anything less
        // than full access would silently downgrade every current
        // superuser's own account on upgrade.
        r.register(Migration::new(
            ADD_SUPERUSER_ROLE,
            Box::new(|db| Box::pin(add_superuser_role_up(db))),
            Box::new(|db| Box::pin(add_superuser_role_down(db))),
        ));
        // `_audit_log` follows the same story as every migration above:
        // added to `default_system_collections()` after `INIT_SYSTEM`
        // shipped, so an existing database needs this follow-up
        // migration to retroactively get the table. A fresh database
        // already has it from `INIT_SYSTEM` and this migration is a
        // no-op there.
        r.register(Migration::new(
            ADD_AUDIT_LOG,
            Box::new(|db| Box::pin(add_audit_log_up(db))),
            Box::new(|db| Box::pin(add_audit_log_down(db))),
        ));
        // `_api_keys.actsAsCollection`/`.actsAsRecord` were added after
        // `INIT_SYSTEM` shipped, same story as `ADD_SUPERUSER_ROLE`
        // above — an existing database's `_api_keys` table predates the
        // pair and needs them added; a fresh database already has them
        // from `INIT_SYSTEM`'s `default_system_collections()` and this
        // migration is a no-op there. No backfill is needed: the
        // physical zero default for a new text column is `''`, which is
        // already the "unscoped, superuser" reading `crate::api_keys`
        // (server crate) gives an absent pair, so every pre-existing key
        // keeps behaving exactly as before.
        r.register(Migration::new(
            ADD_API_KEY_SCOPING,
            Box::new(|db| Box::pin(add_api_key_scoping_up(db))),
            Box::new(|db| Box::pin(add_api_key_scoping_down(db))),
        ));
        // `_sessions` follows the same story as every migration above:
        // added to `default_system_collections()` after `INIT_SYSTEM`
        // shipped, so an existing database needs this follow-up
        // migration to retroactively get the table. A fresh database
        // already has it from `INIT_SYSTEM` and this migration is a
        // no-op there.
        r.register(Migration::new(
            ADD_SESSIONS,
            Box::new(|db| Box::pin(add_sessions_up(db))),
            Box::new(|db| Box::pin(add_sessions_down(db))),
        ));
        // `_bans` follows the same story as `_sessions` immediately
        // above.
        r.register(Migration::new(
            ADD_BANS,
            Box::new(|db| Box::pin(add_bans_up(db))),
            Box::new(|db| Box::pin(add_bans_down(db))),
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
    ///
    /// Ties on `applied` (migrations run in the same `up()` call share a
    /// timestamp, second resolution) break on the numeric prefix of the
    /// filename, not a plain string compare — `"10_..."` sorts as file
    /// number 10, not lexicographically before `"9_..."` (`'1' < '9'` as
    /// characters), which would revert a same-second batch out of
    /// registration order the moment a migration count crosses into
    /// double digits.
    pub async fn down(&self, db: &Db, n: usize) -> DbResult<Vec<String>> {
        let mut done = applied(db).await?;
        done.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| migration_number(&b.0).cmp(&migration_number(&a.0)))
        });
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

/// The leading integer of a `"{n}_description.rs"` migration filename —
/// see [`Runner::down`]'s doc comment for why this exists instead of
/// comparing filenames as strings. An unparseable prefix (never
/// registered by this module, but the ledger can in principle hold rows
/// [`Runner`] doesn't know about) sorts as `0`, i.e. oldest.
fn migration_number(file: &str) -> u32 {
    file.split('_')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
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

pub const ADD_CRON_JOBS: &str = "2_add_cron_jobs.rs";

async fn add_cron_jobs_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_cron_jobs").is_some() {
        return Ok(());
    }
    let collection = Collection::default_system_collections()
        .into_iter()
        .find(|c| c.name == "_cron_jobs")
        .expect("_cron_jobs is a default system collection");
    db.collections.insert(&*db.engine, &collection).await?;
    Ok(())
}

async fn add_cron_jobs_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_cron_jobs").is_some() {
        db.collections.delete(&*db.engine, "_cron_jobs").await?;
    }
    Ok(())
}

pub const ADD_WEBHOOKS: &str = "3_add_webhooks.rs";

async fn add_webhooks_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_webhooks").is_some() {
        return Ok(());
    }
    let collection = Collection::default_system_collections()
        .into_iter()
        .find(|c| c.name == "_webhooks")
        .expect("_webhooks is a default system collection");
    db.collections.insert(&*db.engine, &collection).await?;
    Ok(())
}

async fn add_webhooks_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_webhooks").is_some() {
        db.collections.delete(&*db.engine, "_webhooks").await?;
    }
    Ok(())
}

pub const ADD_TEAMS: &str = "4_add_teams.rs";

async fn add_teams_up(db: &Db) -> DbResult<()> {
    for name in ["_teams", "_team_members"] {
        if db.collections.get_by_name(name).is_some() {
            continue;
        }
        let collection = Collection::default_system_collections()
            .into_iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} is a default system collection"));
        db.collections.insert(&*db.engine, &collection).await?;
    }
    Ok(())
}

async fn add_teams_down(db: &Db) -> DbResult<()> {
    // `_team_members` before `_teams`: it holds a relation field pointing
    // at `_teams`, same ordering constraint `init_system_down` follows by
    // reverting `seed_collections()` in reverse.
    for name in ["_team_members", "_teams"] {
        if db.collections.get_by_name(name).is_some() {
            db.collections.delete(&*db.engine, name).await?;
        }
    }
    Ok(())
}

pub const ADD_LLM_USAGE: &str = "5_add_llm_usage.rs";

async fn add_llm_usage_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_llm_usage").is_some() {
        return Ok(());
    }
    let collection = Collection::default_system_collections()
        .into_iter()
        .find(|c| c.name == "_llm_usage")
        .expect("_llm_usage is a default system collection");
    db.collections.insert(&*db.engine, &collection).await?;
    Ok(())
}

async fn add_llm_usage_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_llm_usage").is_some() {
        db.collections.delete(&*db.engine, "_llm_usage").await?;
    }
    Ok(())
}

pub const ADD_API_KEYS: &str = "6_add_api_keys.rs";

async fn add_api_keys_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_api_keys").is_some() {
        return Ok(());
    }
    let collection = Collection::default_system_collections()
        .into_iter()
        .find(|c| c.name == "_api_keys")
        .expect("_api_keys is a default system collection");
    db.collections.insert(&*db.engine, &collection).await?;
    Ok(())
}

async fn add_api_keys_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_api_keys").is_some() {
        db.collections.delete(&*db.engine, "_api_keys").await?;
    }
    Ok(())
}

pub const ADD_PUSH_SUBSCRIPTIONS: &str = "7_add_push_subscriptions.rs";

async fn add_push_subscriptions_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_push_subscriptions").is_some() {
        return Ok(());
    }
    let collection = Collection::default_system_collections()
        .into_iter()
        .find(|c| c.name == "_push_subscriptions")
        .expect("_push_subscriptions is a default system collection");
    db.collections.insert(&*db.engine, &collection).await?;
    Ok(())
}

async fn add_push_subscriptions_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_push_subscriptions").is_some() {
        db.collections
            .delete(&*db.engine, "_push_subscriptions")
            .await?;
    }
    Ok(())
}

pub const ADD_SUPERUSER_ROLE: &str = "8_add_superuser_role.rs";

/// Adds `_superusers.role` (see `Field::role_field`) to a database that
/// predates it, and backfills every pre-existing row to `"owner"` — the
/// column's own physical zero default (`''`) is not a valid role, and
/// leaving it there would silently strip every current superuser of the
/// full access they had before this migration ran.
async fn add_superuser_role_up(db: &Db) -> DbResult<()> {
    let Some(previous) = db
        .collections
        .get_by_name(cratebase_core::SUPERUSERS_COLLECTION)
    else {
        return Ok(());
    };
    if !previous.fields.iter().any(|f| f.name == "role") {
        let mut next = (*previous).clone();
        // Same insertion point `Collection::default_superusers` uses: right
        // before `created`/`updated`.
        let pos = next.fields.len() - 2;
        next.fields.insert(pos, Field::role_field());
        db.collections.update(&*db.engine, &next).await?;
    }
    // Always run, even when the column already existed: the ALTER and
    // this backfill are two separate commits, so a process that died
    // between them leaves a database where `role` already exists but
    // every row still has the zero default `''`. Skipping this step in
    // that case would mark the migration applied while permanently
    // locking every existing superuser out (no valid role, no recovery
    // path). Idempotent via the `WHERE` clause, so re-running it against
    // an already-backfilled table is a no-op.
    db.execute(
        &format!(
            r#"UPDATE "{}" SET "role" = '{}' WHERE "role" = '' OR "role" IS NULL"#,
            cratebase_core::SUPERUSERS_COLLECTION,
            cratebase_core::SUPERUSER_ROLE_OWNER,
        ),
        &[],
    )
    .await?;
    Ok(())
}

async fn add_superuser_role_down(db: &Db) -> DbResult<()> {
    let Some(previous) = db
        .collections
        .get_by_name(cratebase_core::SUPERUSERS_COLLECTION)
    else {
        return Ok(());
    };
    if !previous.fields.iter().any(|f| f.name == "role") {
        return Ok(());
    }
    let mut next = (*previous).clone();
    next.fields.retain(|f| f.name != "role");
    db.collections.update(&*db.engine, &next).await?;
    Ok(())
}

pub const ADD_AUDIT_LOG: &str = "9_add_audit_log.rs";

async fn add_audit_log_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_audit_log").is_some() {
        return Ok(());
    }
    let collection = Collection::default_system_collections()
        .into_iter()
        .find(|c| c.name == "_audit_log")
        .expect("_audit_log is a default system collection");
    db.collections.insert(&*db.engine, &collection).await?;
    Ok(())
}

async fn add_audit_log_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_audit_log").is_some() {
        db.collections.delete(&*db.engine, "_audit_log").await?;
    }
    Ok(())
}

pub const ADD_API_KEY_SCOPING: &str = "10_add_api_key_scoping.rs";

/// A record-scoped text field, matching the shape `Collection::default_system_collections`
/// gives `_api_keys.actsAsCollection`/`.actsAsRecord` on a fresh database — see
/// `ADD_API_KEY_SCOPING`'s registration comment for why an existing database
/// needs this migration and why no backfill is needed.
fn acts_as_field(name: &str) -> Field {
    let mut f = Field::new(
        name,
        cratebase_core::FieldKind::Text {
            min: 0,
            max: 0,
            pattern: String::new(),
            autogenerate_pattern: String::new(),
            primary_key: false,
        },
    );
    f.system = true;
    f.required = false;
    f
}

async fn add_api_key_scoping_up(db: &Db) -> DbResult<()> {
    let Some(previous) = db.collections.get_by_name("_api_keys") else {
        return Ok(());
    };
    if previous.fields.iter().any(|f| f.name == "actsAsCollection") {
        return Ok(());
    }
    let mut next = (*previous).clone();
    // Same insertion point `default_system_collections` uses: right
    // before `created`/`updated`.
    let pos = next.fields.len() - 2;
    next.fields.insert(pos, acts_as_field("actsAsCollection"));
    next.fields.insert(pos + 1, acts_as_field("actsAsRecord"));
    db.collections.update(&*db.engine, &next).await?;
    Ok(())
}

async fn add_api_key_scoping_down(db: &Db) -> DbResult<()> {
    let Some(previous) = db.collections.get_by_name("_api_keys") else {
        return Ok(());
    };
    if !previous.fields.iter().any(|f| f.name == "actsAsCollection") {
        return Ok(());
    }
    let mut next = (*previous).clone();
    next.fields
        .retain(|f| f.name != "actsAsCollection" && f.name != "actsAsRecord");
    db.collections.update(&*db.engine, &next).await?;
    Ok(())
}

pub const ADD_SESSIONS: &str = "11_add_sessions.rs";

/// `_sessions` follows the same story as every migration above: added to
/// `default_system_collections()` after `INIT_SYSTEM` shipped, so an
/// existing database needs this follow-up migration to retroactively get
/// the table. A fresh database already has it from `INIT_SYSTEM` and this
/// migration is a no-op there.
async fn add_sessions_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_sessions").is_some() {
        return Ok(());
    }
    let collection = Collection::default_system_collections()
        .into_iter()
        .find(|c| c.name == "_sessions")
        .expect("_sessions is a default system collection");
    db.collections.insert(&*db.engine, &collection).await?;
    Ok(())
}

async fn add_sessions_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_sessions").is_some() {
        db.collections.delete(&*db.engine, "_sessions").await?;
    }
    Ok(())
}

pub const ADD_BANS: &str = "12_add_bans.rs";

/// `_bans` follows the same story as every migration above: added to
/// `default_system_collections()` after `INIT_SYSTEM` shipped, so an
/// existing database needs this follow-up migration to retroactively get
/// the table. A fresh database already has it from `INIT_SYSTEM` and this
/// migration is a no-op there.
async fn add_bans_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_bans").is_some() {
        return Ok(());
    }
    let collection = Collection::default_system_collections()
        .into_iter()
        .find(|c| c.name == "_bans")
        .expect("_bans is a default system collection");
    db.collections.insert(&*db.engine, &collection).await?;
    Ok(())
}

async fn add_bans_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_bans").is_some() {
        db.collections.delete(&*db.engine, "_bans").await?;
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
        assert_eq!(
            ran,
            vec![
                INIT_SYSTEM.to_string(),
                ADD_CRON_JOBS.to_string(),
                ADD_WEBHOOKS.to_string(),
                ADD_TEAMS.to_string(),
                ADD_LLM_USAGE.to_string(),
                ADD_API_KEYS.to_string(),
                ADD_PUSH_SUBSCRIPTIONS.to_string(),
                ADD_SUPERUSER_ROLE.to_string(),
                ADD_AUDIT_LOG.to_string(),
                ADD_API_KEY_SCOPING.to_string(),
                ADD_SESSIONS.to_string(),
                ADD_BANS.to_string(),
            ]
        );
        assert_eq!(
            db.collections.get("_superusers").unwrap().id,
            "pbc_3142635823"
        );
        assert_eq!(db.collections.get("users").unwrap().id, "_pb_users_auth_");
        assert_eq!(db.collections.get("_mfas").unwrap().id, "pbc_2279338944");
        assert_eq!(db.collections.get("_otps").unwrap().id, "pbc_1638494021");
        assert!(db.collections.get("_externalAuths").is_some());
        assert!(db.collections.get("_authOrigins").is_some());
        assert!(db.collections.get("_sessions").is_some());
        assert!(db.collections.get("_bans").is_some());
        assert!(db.collections.get("_cron_jobs").is_some());
        assert!(db.collections.get("_webhooks").is_some());
        assert!(db.collections.get("_teams").is_some());
        assert!(db.collections.get("_team_members").is_some());
        assert!(db.collections.get("_llm_usage").is_some());
        assert!(db.collections.get("_api_keys").is_some());
        assert!(db.collections.get("_push_subscriptions").is_some());
        assert!(db.collections.get("_audit_log").is_some());
        assert!(db.collections.get("_superusers").unwrap().system);
        for t in [
            "_superusers",
            "users",
            "_mfas",
            "_otps",
            "_externalAuths",
            "_authOrigins",
            "_sessions",
            "_bans",
            "_cron_jobs",
            "_webhooks",
            "_teams",
            "_team_members",
            "_llm_usage",
            "_api_keys",
            "_push_subscriptions",
            "_audit_log",
        ] {
            assert!(db.engine.table_exists(t).await.unwrap(), "{t}");
        }
        // Second run is a no-op.
        assert!(Runner::core().up(&db).await.unwrap().is_empty());
        assert!(is_applied(&db, INIT_SYSTEM).await.unwrap());

        let reverted = Runner::core().down(&db, 12).await.unwrap();
        assert_eq!(
            reverted,
            vec![
                ADD_BANS.to_string(),
                ADD_SESSIONS.to_string(),
                ADD_API_KEY_SCOPING.to_string(),
                ADD_AUDIT_LOG.to_string(),
                ADD_SUPERUSER_ROLE.to_string(),
                ADD_PUSH_SUBSCRIPTIONS.to_string(),
                ADD_API_KEYS.to_string(),
                ADD_LLM_USAGE.to_string(),
                ADD_TEAMS.to_string(),
                ADD_WEBHOOKS.to_string(),
                ADD_CRON_JOBS.to_string(),
                INIT_SYSTEM.to_string(),
            ]
        );
        assert!(db.collections.is_empty());
        assert!(!db.engine.table_exists("users").await.unwrap());
        assert!(!is_applied(&db, INIT_SYSTEM).await.unwrap());
    }

    #[tokio::test]
    async fn add_superuser_role_backfills_existing_rows_to_owner() {
        let db = fresh().await;
        // Seed `_superusers` in the exact shape it had before this
        // migration existed — `default_superusers()` itself now always
        // includes `role`, so a real "predates this feature" table is
        // built by stripping it back off rather than by running the
        // in-code seed collections (which would already have it).
        let mut without_role = Collection::default_superusers();
        without_role.fields.retain(|f| f.name != "role");
        db.collections
            .insert(&*db.engine, &without_role)
            .await
            .unwrap();
        assert!(db
            .collections
            .get_by_name("_superusers")
            .unwrap()
            .fields
            .iter()
            .all(|f| f.name != "role"));

        // A superuser row created before the `role` column existed —
        // exactly the shape a real installation's table has going into
        // the upgrade.
        db.execute(
            r#"INSERT INTO "_superusers"
               ("id", "email", "password", "tokenKey", "emailVisibility", "verified", "created", "updated")
               VALUES ('sup00000000000', 'a@b.co', 'hash', 'tok0000000000000000000000000000', 0, 1,
                       '2024-01-01 00:00:00.000Z', '2024-01-01 00:00:00.000Z')"#,
            &[],
        )
        .await
        .unwrap();

        add_superuser_role_up(&db).await.unwrap();

        assert!(db
            .collections
            .get_by_name("_superusers")
            .unwrap()
            .fields
            .iter()
            .any(|f| f.name == "role"));
        let role = db
            .query_scalar(
                r#"SELECT "role" FROM "_superusers" WHERE "id" = 'sup00000000000'"#,
                &[],
            )
            .await
            .unwrap()
            .and_then(|v| v.as_str().map(str::to_string));
        assert_eq!(role.as_deref(), Some("owner"));

        // Re-running on an already-migrated collection is a no-op, not
        // an error (idempotent, like every other migration here).
        add_superuser_role_up(&db).await.unwrap();
    }

    #[tokio::test]
    async fn add_superuser_role_up_backfills_on_a_partially_migrated_rerun() {
        // Reproduces a process dying between the ALTER (schema update)
        // and the backfill UPDATE the first time this migration ran: the
        // column already exists, but every row still carries its zero
        // default. The next boot must still complete the backfill
        // instead of returning early because `role` is already present.
        let db = fresh().await;
        db.collections
            .insert(&*db.engine, &Collection::default_superusers())
            .await
            .unwrap();
        assert!(db
            .collections
            .get_by_name("_superusers")
            .unwrap()
            .fields
            .iter()
            .any(|f| f.name == "role"));

        // A row that already has the column (physical zero default),
        // simulating the schema-updated-but-not-backfilled state.
        db.execute(
            r#"INSERT INTO "_superusers"
               ("id", "email", "password", "tokenKey", "role", "emailVisibility", "verified", "created", "updated")
               VALUES ('sup00000000001', 'c@d.co', 'hash', 'tok0000000000000000000000000001', '', 0, 1,
                       '2024-01-01 00:00:00.000Z', '2024-01-01 00:00:00.000Z')"#,
            &[],
        )
        .await
        .unwrap();

        add_superuser_role_up(&db).await.unwrap();

        let role = db
            .query_scalar(
                r#"SELECT "role" FROM "_superusers" WHERE "id" = 'sup00000000001'"#,
                &[],
            )
            .await
            .unwrap()
            .and_then(|v| v.as_str().map(str::to_string));
        assert_eq!(
            role.as_deref(),
            Some("owner"),
            "re-running the migration against an ALTER'd-but-not-backfilled table must still \
             backfill, not skip out early because the column already exists"
        );
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
