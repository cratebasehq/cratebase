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

use cratebase_core::{Collection, Field, FieldKind};
use cratebase_mailer::{cta_button, info_box, otp_code_block};

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
        // `_emailTemplates`/`_mailLog` follow the same story as every
        // migration above, plus a one-time seed of seven default
        // `_emailTemplates` rows (the five auth templates, magic-link,
        // and a `welcome` example) — see `add_email_platform_up`'s doc.
        r.register(Migration::new(
            ADD_EMAIL_PLATFORM,
            Box::new(|db| Box::pin(add_email_platform_up(db))),
            Box::new(|db| Box::pin(add_email_platform_down(db))),
        ));
        // `_emailTemplates.sendRule`/`.design`/`.editor` were added after
        // `ADD_EMAIL_PLATFORM` shipped, same story as `ADD_SUPERUSER_ROLE`
        // above — an existing database's `_emailTemplates` table
        // predates them and needs them added; a fresh database already
        // has them from `default_system_collections()` and this
        // migration is a no-op there. No backfill: every existing row's
        // physical zero default for the new `Json` columns is SQL
        // `NULL`, which is exactly `sendRule`'s "superuser only" reading
        // — so every template customized before this feature existed
        // keeps behaving exactly as before (superuser/API-key-only
        // sends), rather than silently becoming publicly sendable.
        r.register(Migration::new(
            ADD_EMAIL_SEND_RULES,
            Box::new(|db| Box::pin(add_email_send_rules_up(db))),
            Box::new(|db| Box::pin(add_email_send_rules_down(db))),
        ));
        // `_emailTriggers` follows the same story as `_emailTemplates`/
        // `_mailLog` (`ADD_EMAIL_PLATFORM` above): added to
        // `default_system_collections()` after that migration shipped,
        // so an existing database needs this follow-up migration to
        // retroactively get the table. A fresh database already has it
        // and this migration is a no-op there.
        r.register(Migration::new(
            ADD_EMAIL_TRIGGERS,
            Box::new(|db| Box::pin(add_email_triggers_up(db))),
            Box::new(|db| Box::pin(add_email_triggers_down(db))),
        ));
        // `_rpc` follows the same story as `_bans` above.
        r.register(Migration::new(
            ADD_RPC,
            Box::new(|db| Box::pin(add_rpc_up(db))),
            Box::new(|db| Box::pin(add_rpc_down(db))),
        ));
        // `_totps` follows the same story as `_rpc` immediately above.
        r.register(Migration::new(
            ADD_TOTPS,
            Box::new(|db| Box::pin(add_totps_up(db))),
            Box::new(|db| Box::pin(add_totps_down(db))),
        ));
        // Adds `_emailAssets` (same story as `_rpc` above) and refreshes
        // every untouched `_emailTemplates` seed row to the redesigned
        // copy/layout — see `refresh_default_email_templates_up`'s doc.
        r.register(Migration::new(
            REFRESH_EMAIL_TEMPLATES,
            Box::new(|db| Box::pin(refresh_default_email_templates_up(db))),
            Box::new(|db| Box::pin(refresh_default_email_templates_down(db))),
        ));
        // `_pendingUploads` follows the same story as `_totps`/`_rpc`
        // above: added to `default_system_collections()` after
        // `REFRESH_EMAIL_TEMPLATES` shipped, so an existing database
        // needs this follow-up migration to retroactively get the table.
        // A fresh database already has it and this migration is a no-op
        // there. Named `19_...`: the next core migration number after
        // `REFRESH_EMAIL_TEMPLATES` (`18_...`).
        r.register(Migration::new(
            ADD_PENDING_UPLOADS,
            Box::new(|db| Box::pin(add_pending_uploads_up(db))),
            Box::new(|db| Box::pin(add_pending_uploads_down(db))),
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

pub const ADD_RPC: &str = "16_add_rpc.rs";

/// `_rpc` follows the same story as `_bans`: added to
/// `default_system_collections()` after `INIT_SYSTEM` shipped, so an
/// existing database needs this follow-up migration to get the table. A
/// fresh database already has it and this migration is a no-op there.
async fn add_rpc_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_rpc").is_some() {
        return Ok(());
    }
    let collection = Collection::default_system_collections()
        .into_iter()
        .find(|c| c.name == "_rpc")
        .expect("_rpc is a default system collection");
    db.collections.insert(&*db.engine, &collection).await?;
    Ok(())
}

async fn add_rpc_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_rpc").is_some() {
        db.collections.delete(&*db.engine, "_rpc").await?;
    }
    Ok(())
}

pub const ADD_TOTPS: &str = "17_add_totps.rs";

/// `_totps` (TOTP 2FA state — `crate::routes::totp` in the server crate)
/// follows the same story as `_rpc`/`_bans` above: added to
/// `default_system_collections()` after `INIT_SYSTEM` shipped, so an
/// existing database needs this follow-up migration to get the table. A
/// fresh database already has it and this migration is a no-op there.
async fn add_totps_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_totps").is_some() {
        return Ok(());
    }
    let collection = Collection::default_system_collections()
        .into_iter()
        .find(|c| c.name == "_totps")
        .expect("_totps is a default system collection");
    db.collections.insert(&*db.engine, &collection).await?;
    Ok(())
}

async fn add_totps_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_totps").is_some() {
        db.collections.delete(&*db.engine, "_totps").await?;
    }
    Ok(())
}

pub const ADD_EMAIL_PLATFORM: &str = "13_add_email_platform.rs";
pub const ADD_EMAIL_SEND_RULES: &str = "14_add_email_send_rules.rs";
pub const ADD_EMAIL_TRIGGERS: &str = "15_add_email_triggers.rs";

/// `_emailTemplates`/`_mailLog`/`_magicLinks` follow the same story as
/// every migration above: added to `default_system_collections()` after
/// `INIT_SYSTEM` shipped, so an existing database needs this follow-up
/// migration to retroactively get all three tables. A fresh database
/// already has them from `INIT_SYSTEM`, so those inserts are no-ops there
/// — but the seed step below always runs (once, per the migration
/// ledger), on a fresh database and an upgraded one alike, since seeding
/// rows is not something `default_system_collections()`/`INIT_SYSTEM` do
/// for *any* collection.
async fn add_email_platform_up(db: &Db) -> DbResult<()> {
    for name in ["_emailTemplates", "_mailLog", "_magicLinks"] {
        if db.collections.get_by_name(name).is_none() {
            let collection = Collection::default_system_collections()
                .into_iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("{name} is a default system collection"));
            db.collections.insert(&*db.engine, &collection).await?;
        }
    }
    seed_default_email_templates(db).await?;
    Ok(())
}

async fn add_email_platform_down(db: &Db) -> DbResult<()> {
    for name in ["_magicLinks", "_mailLog", "_emailTemplates"] {
        if db.collections.get_by_name(name).is_some() {
            db.collections.delete(&*db.engine, name).await?;
        }
    }
    Ok(())
}

async fn add_email_send_rules_up(db: &Db) -> DbResult<()> {
    let Some(previous) = db.collections.get_by_name("_emailTemplates") else {
        return Ok(());
    };
    if previous.fields.iter().any(|f| f.name == "sendRule") {
        return Ok(());
    }
    let mut next = (*previous).clone();
    let pos = next.fields.len() - 2;
    next.fields
        .insert(pos, Field::new("sendRule", FieldKind::Json { max_size: 0 }));
    next.fields.insert(
        pos + 1,
        Field::new("design", FieldKind::Json { max_size: 0 }),
    );
    next.fields.insert(
        pos + 2,
        Field::new(
            "editor",
            FieldKind::Select {
                values: vec!["visual".into(), "html".into()],
                max_select: 1,
            },
        ),
    );
    db.collections.update(&*db.engine, &next).await?;
    Ok(())
}

async fn add_email_send_rules_down(db: &Db) -> DbResult<()> {
    let Some(previous) = db.collections.get_by_name("_emailTemplates") else {
        return Ok(());
    };
    if !previous.fields.iter().any(|f| f.name == "sendRule") {
        return Ok(());
    }
    let mut next = (*previous).clone();
    next.fields
        .retain(|f| !["sendRule", "design", "editor"].contains(&f.name.as_str()));
    db.collections.update(&*db.engine, &next).await?;
    Ok(())
}

async fn add_email_triggers_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_emailTriggers").is_none() {
        let collection = Collection::default_system_collections()
            .into_iter()
            .find(|c| c.name == "_emailTriggers")
            .expect("_emailTriggers is a default system collection");
        db.collections.insert(&*db.engine, &collection).await?;
    }
    Ok(())
}

async fn add_email_triggers_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_emailTriggers").is_some() {
        db.collections.delete(&*db.engine, "_emailTriggers").await?;
    }
    Ok(())
}

/// One `_emailTemplates` seed row. `subject`/`html` are owned `String`s
/// (built from [`cratebase_mailer::template`]'s `cta_button`/
/// `otp_code_block`/`info_box` helpers, see [`seed_email_templates`])
/// rather than `&'static str` literals, unlike [`OldSeedTemplate`] —
/// `key`/`name`/`description` stay plain metadata literals. Rendered via
/// `cratebase_mailer::template::render_email_template`, never the legacy
/// `render_template`.
struct SeedTemplate {
    key: &'static str,
    name: &'static str,
    subject: String,
    html: String,
    description: &'static str,
}

/// A small heading matching [`cratebase_mailer::template::render_layout`]'s
/// `.cb-heading` dark-mode override.
fn heading(text: &str) -> String {
    format!(
        r#"<h1 class="cb-heading" style="margin:0 0 16px;font-size:22px;line-height:28px;font-weight:700;color:#111827;">{text}</h1>"#
    )
}

/// An ordinary body paragraph — plain `<p>`, since the layout's own
/// `.cb-text` wrapper already sets the base color/size every seed
/// template's content sits inside.
fn body_p(text: &str) -> String {
    format!(r#"<p style="margin:0 0 16px;line-height:24px;">{text}</p>"#)
}

/// A de-emphasized closing note (the "didn't request this?" line),
/// matching `.cb-muted`.
fn muted_p(text: &str) -> String {
    format!(
        r#"<p class="cb-muted" style="margin:24px 0 0;font-size:13px;line-height:20px;color:#8a8a90;">{text}</p>"#
    )
}

/// The seven default `_emailTemplates` rows: the five auth templates,
/// magic-link, and a `welcome` example — Linear/Vercel/Stripe-style
/// copy (short, direct, one clear action) over
/// [`cratebase_mailer::template::render_layout`]'s centered card, with
/// a bulletproof [`cta_button`] for every call to action and
/// [`otp_code_block`]/[`info_box`] for the two templates that need
/// something other than a button.
fn seed_email_templates() -> Vec<SeedTemplate> {
    vec![
        SeedTemplate {
            key: "auth.verification",
            name: "Verification email",
            subject: "Verify your email for {{appName}}".into(),
            html: format!(
                "{}{}{}{}",
                heading("Verify your email"),
                body_p("Welcome to {{appName}}. Click the button below to verify your email address and get started."),
                cta_button("Verify email", "{{appUrl}}/_/#/auth/confirm-verification/{{token}}"),
                muted_p("If you didn't create an account with {{appName}}, you can safely ignore this email."),
            ),
            description: "Editable copy of the built-in email-verification template. A collection's own authOptions.verificationTemplate, if customized away from its default, still takes priority over this row.",
        },
        SeedTemplate {
            key: "auth.passwordReset",
            name: "Password reset email",
            subject: "Reset your {{appName}} password".into(),
            html: format!(
                "{}{}{}{}",
                heading("Reset your password"),
                body_p("We received a request to reset the password for your {{appName}} account. Click the button below to choose a new one."),
                cta_button("Reset password", "{{appUrl}}/_/#/auth/confirm-password-reset/{{token}}"),
                muted_p("This link will expire soon. If you didn't request a password reset, your password won't change — you can safely ignore this email."),
            ),
            description: "Editable copy of the built-in password-reset template. A collection's own authOptions.resetPasswordTemplate, if customized away from its default, still takes priority over this row.",
        },
        SeedTemplate {
            key: "auth.emailChange",
            name: "Confirm new email address",
            subject: "Confirm your {{appName}} new email address".into(),
            html: format!(
                "{}{}{}{}",
                heading("Confirm your new email"),
                body_p("Click the button below to confirm this is your new email address for {{appName}}."),
                cta_button("Confirm new email", "{{appUrl}}/_/#/auth/confirm-email-change/{{token}}"),
                muted_p("If you didn't request this change, you can safely ignore this email — your address won't change until confirmed."),
            ),
            description: "Editable copy of the built-in email-change confirmation template. A collection's own authOptions.confirmEmailChangeTemplate, if customized away from its default, still takes priority over this row.",
        },
        SeedTemplate {
            key: "auth.otp",
            name: "One-time password",
            subject: "Your {{appName}} verification code".into(),
            html: format!(
                "{}{}{}{}",
                heading("Your verification code"),
                body_p("Enter this code to continue signing in to {{appName}}."),
                otp_code_block("{{otp}}"),
                muted_p("This code will expire shortly. If you didn't request it, you can safely ignore this email."),
            ),
            description: "Editable copy of the built-in OTP template. A collection's own authOptions.otp.emailTemplate, if customized away from its default, still takes priority over this row.",
        },
        SeedTemplate {
            key: "auth.loginAlert",
            name: "New-location login alert",
            subject: "New sign-in to your {{appName}} account".into(),
            html: format!(
                "{}{}{}{}",
                heading("New sign-in detected"),
                body_p("We noticed a new sign-in to your {{appName}} account:"),
                info_box("{{alertInfo}}"),
                body_p("<strong>If this was you, no action is needed.</strong> If you don't recognize this activity, we recommend changing your password right away."),
            ),
            description: "Editable copy of the built-in new-location login alert. A collection's own authOptions.authAlert.emailTemplate, if customized away from its default, still takes priority over this row.",
        },
        SeedTemplate {
            key: "auth.magic-link",
            name: "Magic link sign-in",
            subject: "Your sign-in link for {{appName}}".into(),
            html: format!(
                "{}{}{}{}",
                heading("Sign in to {{appName}}"),
                body_p("Click the button below to sign in — no password needed."),
                cta_button("Sign in", "{{magicLink}}"),
                muted_p("This link will expire shortly and can only be used once. If you didn't request it, you can safely ignore this email."),
            ),
            description: "Editable copy of the built-in magic-link sign-in template. A collection's own authOptions.magicLink.emailTemplate, if customized away from its default, still takes priority over this row.",
        },
        SeedTemplate {
            key: "welcome",
            name: "Welcome email",
            subject: "Welcome to {{appName}}".into(),
            html: format!(
                "{}{}{}{}",
                heading("Welcome, {{user.name}}"),
                body_p("We're glad you're here. {{appName}} is ready whenever you are — click below to get started."),
                cta_button("Get started", "{{appUrl}}"),
                muted_p("Questions? Just reply to this email — we're happy to help."),
            ),
            description: "Example template, not wired to any built-in flow. Send it with $mails.send({ to, template: \"welcome\", data: { user: { name } } }) or POST /api/mails/send.",
        },
    ]
}

/// [`SeedTemplate`], but for [`OLD_SEED_EMAIL_TEMPLATES`] — the exact
/// content `add_email_platform_up` (`13_add_email_platform.rs`)
/// originally inserted, frozen forever. Only `key`/`subject`/`html`
/// (never `name`/`description`, which
/// `refresh_default_email_templates_up` never compares against) matter
/// here, since this exists purely to detect "this row is still exactly
/// what we first seeded".
struct OldSeedTemplate {
    key: &'static str,
    subject: &'static str,
    html: &'static str,
}

const OLD_SEED_EMAIL_TEMPLATES: &[OldSeedTemplate] = &[
    OldSeedTemplate {
        key: "auth.verification",
        subject: "Verify your {{appName}} email",
        html: "<p>Hello,</p>\n<p>Thank you for joining us at {{appName}}.</p>\n<p>Click on the button below to verify your email address.</p>\n<p>\n  <a class=\"btn\" href=\"{{appUrl}}/_/#/auth/confirm-verification/{{token}}\" target=\"_blank\" rel=\"noopener\">Verify</a>\n</p>\n<p><i>If you didn't recently register, please ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {{appName}} team\n</p>",
    },
    OldSeedTemplate {
        key: "auth.passwordReset",
        subject: "Reset your {{appName}} password",
        html: "<p>Hello,</p>\n<p>Click on the button below to reset your password.</p>\n<p>\n  <a class=\"btn\" href=\"{{appUrl}}/_/#/auth/confirm-password-reset/{{token}}\" target=\"_blank\" rel=\"noopener\">Reset password</a>\n</p>\n<p><i>If you didn't ask to reset your password, please ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {{appName}} team\n</p>",
    },
    OldSeedTemplate {
        key: "auth.emailChange",
        subject: "Confirm your {{appName}} new email address",
        html: "<p>Hello,</p>\n<p>Click on the button below to confirm your new email address.</p>\n<p>\n  <a class=\"btn\" href=\"{{appUrl}}/_/#/auth/confirm-email-change/{{token}}\" target=\"_blank\" rel=\"noopener\">Confirm new email</a>\n</p>\n<p><i>If you didn't ask to change your email address, please ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {{appName}} team\n</p>",
    },
    OldSeedTemplate {
        key: "auth.otp",
        subject: "OTP for {{appName}}",
        html: "<p>Hello,</p>\n<p>Your one-time password is: <strong>{{otp}}</strong></p>\n<p><i>If you didn't ask for the one-time password, you can ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {{appName}} team\n</p>",
    },
    OldSeedTemplate {
        key: "auth.loginAlert",
        subject: "Login from a new location",
        html: "<p>Hello,</p>\n<p>We noticed a login to your {{appName}} account from a new location:</p>\n<p><em>{{alertInfo}}</em></p>\n<p><strong>If this wasn't you, you should immediately change your {{appName}} account password to revoke access from all other locations.</strong></p>\n<p>If this was you, you may disregard this email.</p>\n<p>\n  Thanks,<br/>\n  {{appName}} team\n</p>",
    },
    OldSeedTemplate {
        key: "auth.magic-link",
        subject: "Sign in to {{appName}}",
        html: "<p>Hello,</p>\n<p>Click on the button below to sign in to {{appName}}.</p>\n<p>\n  <a class=\"btn\" href=\"{{magicLink}}\" target=\"_blank\" rel=\"noopener\">Sign in</a>\n</p>\n<p><i>If you didn't ask to sign in, you can ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {{appName}} team\n</p>",
    },
    OldSeedTemplate {
        key: "welcome",
        subject: "Welcome to {{appName}}!",
        html: "<p>Hi {{user.name}},</p>\n<p>Welcome to {{appName}} — we're glad to have you.</p>\n<p>\n  <a class=\"btn\" href=\"{{appUrl}}\" target=\"_blank\" rel=\"noopener\">Get started</a>\n</p>\n<p>\n  Thanks,<br/>\n  {{appName}} team\n</p>",
    },
];

/// Inserts [`seed_email_templates`] into `_emailTemplates`, skipping any
/// `key` that already has a `locale: ""` row — so re-running this
/// (safe, since the migration ledger only calls it once, but `seed::run`
/// or a hand-written script could call it again) never clobbers an
/// admin's edits.
async fn seed_default_email_templates(db: &Db) -> DbResult<()> {
    let Some(collection) = db.collections.get_by_name("_emailTemplates") else {
        return Ok(());
    };
    for tpl in seed_email_templates() {
        let exists = db
            .query_scalar(
                r#"SELECT 1 FROM "_emailTemplates" WHERE "key" = $1 AND "locale" = ''"#,
                &[Sql::Text(tpl.key.to_string())],
            )
            .await?
            .is_some();
        if exists {
            continue;
        }
        let mut record = cratebase_core::Record::new(collection.clone());
        record.set("key", serde_json::Value::String(tpl.key.to_string()));
        record.set("name", serde_json::Value::String(tpl.name.to_string()));
        record.set("subject", serde_json::Value::String(tpl.subject));
        record.set("html", serde_json::Value::String(tpl.html));
        record.set("text", serde_json::Value::String(String::new()));
        record.set("locale", serde_json::Value::String(String::new()));
        record.set("layout", serde_json::Value::Bool(true));
        record.set(
            "description",
            serde_json::Value::String(tpl.description.to_string()),
        );
        crate::records::create(db, &db.collections, &mut record).await?;
    }
    Ok(())
}

pub const REFRESH_EMAIL_TEMPLATES: &str = "18_refresh_default_email_templates.rs";

/// Adds `_emailAssets` (see `default_system_collections`'s doc comment
/// on it) and refreshes every `_emailTemplates` seed row whose
/// `subject`+`html` still exactly match [`OLD_SEED_EMAIL_TEMPLATES`] —
/// i.e. an admin never edited it — to [`seed_email_templates`]'s new
/// copy and layout. A row that differs at all from its old snapshot
/// (whether because an admin customized it, or because a previous run
/// of this very migration already refreshed it) is left alone; this
/// makes the migration idempotent as well as safe to run on a database
/// with real edits in it.
async fn refresh_default_email_templates_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_emailAssets").is_none() {
        let collection = Collection::default_system_collections()
            .into_iter()
            .find(|c| c.name == "_emailAssets")
            .expect("_emailAssets is a default system collection");
        db.collections.insert(&*db.engine, &collection).await?;
    }

    let Some(collection) = db.collections.get_by_name("_emailTemplates") else {
        return Ok(());
    };
    for tpl in seed_email_templates() {
        let Some(old) = OLD_SEED_EMAIL_TEMPLATES.iter().find(|o| o.key == tpl.key) else {
            continue;
        };
        let Some(id) = db
            .query_scalar(
                r#"SELECT "id" FROM "_emailTemplates" WHERE "key" = $1 AND "locale" = ''"#,
                &[Sql::Text(tpl.key.to_string())],
            )
            .await?
            .and_then(|v| v.as_str().map(str::to_string))
        else {
            // No seeded row for this key at all (e.g. an admin deleted
            // it) — this migration only refreshes existing rows, never
            // re-inserts one `seed_default_email_templates` itself
            // wouldn't.
            continue;
        };
        let mut record = crate::records::find_by_id_raw(db, &collection, &id).await?;
        let unchanged = record.get("subject").and_then(serde_json::Value::as_str)
            == Some(old.subject)
            && record.get("html").and_then(serde_json::Value::as_str) == Some(old.html);
        if !unchanged {
            continue;
        }
        record.set("subject", serde_json::Value::String(tpl.subject));
        record.set("html", serde_json::Value::String(tpl.html));
        record.set(
            "description",
            serde_json::Value::String(tpl.description.to_string()),
        );
        crate::records::update(db, &db.collections, &mut record).await?;
    }
    Ok(())
}

/// Best-effort: drops `_emailAssets` (added by the `up` side above).
/// Refreshed template content is not reverted — this migration never
/// records which rows it touched, and rewinding wording that may
/// already have been sent/seen is not a meaningful "down" the way a
/// schema change is.
async fn refresh_default_email_templates_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_emailAssets").is_some() {
        db.collections.delete(&*db.engine, "_emailAssets").await?;
    }
    Ok(())
}

pub const ADD_PENDING_UPLOADS: &str = "19_add_pending_uploads.rs";

/// See `_pendingUploads`'s doc comment on
/// [`cratebase_core::Collection::default_system_collections`] for what
/// the table is for.
async fn add_pending_uploads_up(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_pendingUploads").is_some() {
        return Ok(());
    }
    let collection = Collection::default_system_collections()
        .into_iter()
        .find(|c| c.name == "_pendingUploads")
        .expect("_pendingUploads is a default system collection");
    db.collections.insert(&*db.engine, &collection).await?;
    Ok(())
}

async fn add_pending_uploads_down(db: &Db) -> DbResult<()> {
    if db.collections.get_by_name("_pendingUploads").is_some() {
        db.collections
            .delete(&*db.engine, "_pendingUploads")
            .await?;
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
                ADD_EMAIL_PLATFORM.to_string(),
                ADD_EMAIL_SEND_RULES.to_string(),
                ADD_EMAIL_TRIGGERS.to_string(),
                ADD_RPC.to_string(),
                ADD_TOTPS.to_string(),
                REFRESH_EMAIL_TEMPLATES.to_string(),
                ADD_PENDING_UPLOADS.to_string(),
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
        assert!(db.collections.get("_emailTemplates").is_some());
        assert!(db.collections.get("_mailLog").is_some());
        assert!(db.collections.get("_magicLinks").is_some());
        assert!(db.collections.get("_emailTriggers").is_some());
        assert!(db.collections.get("_totps").is_some());
        assert!(db.collections.get("_emailAssets").is_some());
        assert!(db
            .collections
            .get("_emailTemplates")
            .unwrap()
            .fields
            .iter()
            .any(|f| f.name == "sendRule"));
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
            "_emailTemplates",
            "_mailLog",
            "_magicLinks",
            "_emailTriggers",
            "_totps",
            "_emailAssets",
            "_pendingUploads",
        ] {
            assert!(db.engine.table_exists(t).await.unwrap(), "{t}");
        }
        // A fresh database's `_emailTemplates` already has the seed rows.
        let seeded: i64 = db
            .query_scalar(r#"SELECT COUNT(*) FROM "_emailTemplates""#, &[])
            .await
            .unwrap()
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        assert_eq!(seeded, seed_email_templates().len() as i64);
        // Second run is a no-op.
        assert!(Runner::core().up(&db).await.unwrap().is_empty());
        assert!(is_applied(&db, INIT_SYSTEM).await.unwrap());

        let reverted = Runner::core().down(&db, 19).await.unwrap();
        assert_eq!(
            reverted,
            vec![
                ADD_PENDING_UPLOADS.to_string(),
                REFRESH_EMAIL_TEMPLATES.to_string(),
                ADD_TOTPS.to_string(),
                ADD_RPC.to_string(),
                ADD_EMAIL_TRIGGERS.to_string(),
                ADD_EMAIL_SEND_RULES.to_string(),
                ADD_EMAIL_PLATFORM.to_string(),
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

    /// Reads one `_emailTemplates` row's `html` by key, for the refresh
    /// migration tests below.
    async fn html_for(db: &Db, key: &str) -> String {
        db.query_scalar(
            r#"SELECT "html" FROM "_emailTemplates" WHERE "key" = $1"#,
            &[Sql::Text(key.to_string())],
        )
        .await
        .unwrap()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap()
    }

    #[tokio::test]
    async fn refresh_default_email_templates_updates_rows_unchanged_since_the_old_seed() {
        let db = fresh().await;
        Runner::core().up(&db).await.unwrap();
        // Roll the row back to the exact content `13_add_email_platform.rs`
        // used to seed, simulating a database that bootstrapped before
        // this migration existed.
        let old = OLD_SEED_EMAIL_TEMPLATES
            .iter()
            .find(|o| o.key == "auth.verification")
            .unwrap();
        db.execute(
            r#"UPDATE "_emailTemplates" SET "subject" = $1, "html" = $2 WHERE "key" = 'auth.verification'"#,
            &[
                Sql::Text(old.subject.to_string()),
                Sql::Text(old.html.to_string()),
            ],
        )
        .await
        .unwrap();
        assert_eq!(html_for(&db, "auth.verification").await, old.html);

        refresh_default_email_templates_up(&db).await.unwrap();

        let html = html_for(&db, "auth.verification").await;
        assert!(html.contains("Verify your email"), "new heading: {html}");
        assert!(
            !html.contains("Thank you for joining us"),
            "old copy replaced: {html}"
        );
    }

    #[tokio::test]
    async fn refresh_default_email_templates_skips_rows_an_admin_customized() {
        let db = fresh().await;
        Runner::core().up(&db).await.unwrap();
        db.execute(
            r#"UPDATE "_emailTemplates" SET "html" = $1 WHERE "key" = 'auth.verification'"#,
            &[Sql::Text("<p>My custom copy</p>".into())],
        )
        .await
        .unwrap();

        refresh_default_email_templates_up(&db).await.unwrap();

        assert_eq!(
            html_for(&db, "auth.verification").await,
            "<p>My custom copy</p>",
            "an edited row is left exactly as the admin left it"
        );
    }

    #[tokio::test]
    async fn refresh_default_email_templates_is_a_no_op_on_a_fresh_database() {
        // `seed_default_email_templates` (`ADD_EMAIL_PLATFORM`) already
        // inserts `seed_email_templates()`'s new copy directly, so on a
        // fresh bootstrap the refresh migration that runs right after it
        // has nothing to do — every row already looks nothing like
        // `OLD_SEED_EMAIL_TEMPLATES`.
        let db = fresh().await;
        Runner::core().up(&db).await.unwrap();
        let before = html_for(&db, "welcome").await;
        refresh_default_email_templates_up(&db).await.unwrap();
        assert_eq!(html_for(&db, "welcome").await, before);
    }

    /// Not run by default (`cargo test -- --ignored` to run it) — a
    /// developer convenience, not a regression test: renders every
    /// [`seed_email_templates`] row through the real
    /// `render_email_template`/`render_layout` pipeline with
    /// representative sample data for each `{{var}}`, and writes the
    /// result to a temp dir for a human (or a screenshot tool) to look
    /// at after touching the copy or the branded layout.
    #[test]
    #[ignore]
    fn render_check_writes_every_default_template_to_a_temp_dir() {
        use cratebase_core::settings::Meta;
        use cratebase_mailer::{render_email_template, TemplateDoc};
        use serde_json::json;

        let sample_data: &[(&str, serde_json::Value)] = &[
            ("auth.verification", json!({ "token": "tok_9f8a1c2e" })),
            ("auth.passwordReset", json!({ "token": "tok_9f8a1c2e" })),
            ("auth.emailChange", json!({ "token": "tok_9f8a1c2e" })),
            ("auth.otp", json!({ "otp": "482913" })),
            (
                "auth.loginAlert",
                json!({ "alertInfo": "Chrome on macOS · San Francisco, CA · Sep 28, 2026" }),
            ),
            (
                "auth.magic-link",
                json!({ "magicLink": "https://acme.test/_/#/auth/magic/tok_9f8a1c2e" }),
            ),
            ("welcome", json!({ "user": { "name": "Ada" } })),
        ];

        let dir = std::env::temp_dir().join("cratebase-email-render-check");
        std::fs::create_dir_all(&dir).unwrap();

        let meta = Meta {
            app_name: "Acme".into(),
            app_url: "https://acme.test".into(),
            ..Meta::default()
        };

        let mut written = Vec::new();
        for tpl in seed_email_templates() {
            let (_, data) = sample_data
                .iter()
                .find(|(key, _)| *key == tpl.key)
                .unwrap_or_else(|| panic!("no sample data for {}", tpl.key));
            let doc = TemplateDoc {
                subject: &tpl.subject,
                html: &tpl.html,
                text: "",
                layout: true,
            };
            let (_, html, _) = render_email_template(&doc, data, &meta);
            let path = dir.join(format!("{}.html", tpl.key.replace(['.', '-'], "_")));
            std::fs::write(&path, &html).unwrap();
            written.push(path);
        }

        eprintln!(
            "render-check wrote {} files to {}:",
            written.len(),
            dir.display()
        );
        for path in &written {
            eprintln!("  {}", path.display());
        }
        assert_eq!(written.len(), 7);
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

    #[tokio::test]
    async fn add_email_send_rules_up_adds_columns_to_a_pre_existing_table() {
        let db = fresh().await;
        // `_emailTemplates` as it looked before `sendRule`/`design`/
        // `editor` existed — the real shape an upgrading database has.
        let mut without_send_rule = Collection::default_system_collections()
            .into_iter()
            .find(|c| c.name == "_emailTemplates")
            .unwrap();
        without_send_rule
            .fields
            .retain(|f| !["sendRule", "design", "editor"].contains(&f.name.as_str()));
        db.collections
            .insert(&*db.engine, &without_send_rule)
            .await
            .unwrap();
        assert!(db
            .collections
            .get_by_name("_emailTemplates")
            .unwrap()
            .fields
            .iter()
            .all(|f| f.name != "sendRule"));

        add_email_send_rules_up(&db).await.unwrap();

        let updated = db.collections.get_by_name("_emailTemplates").unwrap();
        for name in ["sendRule", "design", "editor"] {
            assert!(
                updated.fields.iter().any(|f| f.name == name),
                "missing {name}"
            );
        }
        // Re-running is a no-op, not an error.
        add_email_send_rules_up(&db).await.unwrap();

        // A row inserted straight through SQL (as an old row that
        // predates the column would have been) never wrote `sendRule`,
        // so its new column defaults to real SQL `NULL` (superuser-only)
        // — not `''` (public). This is the whole point of using a
        // `Json`-kind column for it (see `add_email_send_rules_up`'s doc).
        db.execute(
            r#"INSERT INTO "_emailTemplates" ("id", "key") VALUES ('tpl00000000000', 'x')"#,
            &[],
        )
        .await
        .unwrap();
        let value = crate::records::find_by_id_raw(&db, &updated, "tpl00000000000")
            .await
            .unwrap();
        assert_eq!(
            value.get("sendRule").cloned(),
            Some(serde_json::Value::Null)
        );

        add_email_send_rules_down(&db).await.unwrap();
        assert!(db
            .collections
            .get_by_name("_emailTemplates")
            .unwrap()
            .fields
            .iter()
            .all(|f| f.name != "sendRule"));
    }

    #[tokio::test]
    async fn add_email_triggers_up_is_idempotent() {
        let db = fresh().await;
        assert!(db.collections.get_by_name("_emailTriggers").is_none());
        add_email_triggers_up(&db).await.unwrap();
        assert!(db.collections.get_by_name("_emailTriggers").is_some());
        // Re-running is a no-op, not an error.
        add_email_triggers_up(&db).await.unwrap();
        add_email_triggers_down(&db).await.unwrap();
        assert!(db.collections.get_by_name("_emailTriggers").is_none());
    }

    #[tokio::test]
    async fn add_pending_uploads_up_is_idempotent() {
        let db = fresh().await;
        assert!(db.collections.get_by_name("_pendingUploads").is_none());
        add_pending_uploads_up(&db).await.unwrap();
        assert!(db.collections.get_by_name("_pendingUploads").is_some());
        // Re-running is a no-op, not an error.
        add_pending_uploads_up(&db).await.unwrap();
        add_pending_uploads_down(&db).await.unwrap();
        assert!(db.collections.get_by_name("_pendingUploads").is_none());
    }
}
