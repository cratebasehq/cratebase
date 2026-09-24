//! Wires `cratebase_jsvm`'s JS migration runner (`pb_migrations/*.js`) to
//! the server: a [`cratebase_jsvm::MigrationLedger`] backed by the same
//! `_migrations` table the core (Rust) migration runner uses (see
//! `cratebase_db::migrations`), and the entry points [`App::serve`] and
//! `cratebase migrate up`/`down` call.
//!
//! JS migrations share `_migrations` with the core runner rather than
//! getting a table of their own — `migrate history-sync` and any future
//! listing need one merged view of "what has been applied" — and stay
//! distinguishable by filename: core migrations are registered as
//! `"<n>_description.rs"` (see `cratebase_db::migrations::Runner::core`),
//! JS migrations are `"<unix_ts>_name.js"` (see [`write_migration_stub`]
//! in `main.rs` and [`write_collections_snapshot`] below), so filtering on
//! the `.js` suffix is enough to tell them apart.
//!
//! # Transactionality
//!
//! A migration's `up`/`down` function already runs inside its own
//! `$app.runInTransaction` (`prelude.js`'s `__cb.runMigration`), so a
//! throwing migration rolls back every `$app.save`/`$app.delete` it made
//! before the ledger is ever touched: [`cratebase_jsvm::Runtime::run_migrations_up`]
//! only calls [`MigrationLedger::mark_applied`] *after* the migration
//! function returns without error, so a rollback and a missing ledger
//! entry always happen together.

use std::path::PathBuf;

use async_trait::async_trait;
use cratebase_core::AppError;
use cratebase_jsvm::MigrationLedger;
use serde_json::Value;

use crate::app::App;

/// [`MigrationLedger`] for JS migrations, backed by `_migrations` — see
/// the module doc for why core and JS migrations share the table.
pub struct DbMigrationLedger(App);

impl DbMigrationLedger {
    pub fn new(app: App) -> Self {
        DbMigrationLedger(app)
    }
}

#[async_trait]
impl MigrationLedger for DbMigrationLedger {
    async fn applied(&self) -> Result<Vec<String>, AppError> {
        let rows = cratebase_db::migrations::applied(self.0.db())
            .await
            .map_err(AppError::from)?;
        Ok(rows
            .into_iter()
            .map(|(file, _)| file)
            .filter(|file| file.ends_with(".js"))
            .collect())
    }

    async fn mark_applied(&self, file: &str) -> Result<(), AppError> {
        cratebase_db::migrations::mark_applied(self.0.db(), file)
            .await
            .map_err(AppError::from)
    }

    async fn mark_reverted(&self, file: &str) -> Result<(), AppError> {
        cratebase_db::migrations::mark_reverted(self.0.db(), file)
            .await
            .map_err(AppError::from)
    }
}

/// Apply every pending JS migration, in file-name order. A no-op — not an
/// error — when the JS runtime never started (`jsvm_host::maybe_start`
/// starts it for a non-empty `pb_migrations/` even without any
/// `pb_hooks/*.pb.js` file; see its doc comment), matching PocketBase's
/// "an absent `pb_migrations` costs nothing" behaviour.
pub async fn run_up(app: &App) -> Result<Vec<String>, AppError> {
    let Some(runtime) = app.jsvm() else {
        return Ok(vec![]);
    };
    runtime
        .run_migrations_up(&DbMigrationLedger::new(app.clone()))
        .await
}

/// Revert the last `n` applied JS migrations. See [`run_up`] for the
/// no-runtime case.
pub async fn run_down(app: &App, n: usize) -> Result<Vec<String>, AppError> {
    let Some(runtime) = app.jsvm() else {
        return Ok(vec![]);
    };
    runtime
        .run_migrations_down(&DbMigrationLedger::new(app.clone()), n)
        .await
}

/// Every `*.js` migration file name currently on disk, known or not to the
/// ledger — used by `migrate history-sync` to decide which ledger rows are
/// stale. Independent of whether the JS runtime happens to be running, so
/// it stays correct even when `pb_migrations/` is momentarily the only
/// non-empty JS directory (a plain `Runtime` isn't needed just to list
/// files).
pub fn known_migration_files(app: &App) -> Result<Vec<String>, AppError> {
    let dir = std::path::Path::new(&app.config().migrations_dir);
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => {
            return Err(AppError::internal(format!(
                "cannot read {}: {e}",
                dir.display()
            )))
        }
    };
    let mut files: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".js"))
        .collect();
    files.sort();
    Ok(files)
}

/// `cratebase migrate collections`: write a migration snapshotting every
/// current non-system collection — `up()` (re)creates each one from its
/// exported JSON, `down()` deletes them again in reverse order (the same
/// convention `cratebase_db::migrations::add_teams_down` follows for a
/// relation between two collections created together). Mirrors
/// `SchemaAction::Pull`'s `!c.system` filter: system collections already
/// come from the core migrations, so re-snapshotting them would only make
/// this migration fail on a fresh database that has not run those yet.
pub fn write_collections_snapshot(app: &App) -> Result<PathBuf, AppError> {
    let dir = std::path::Path::new(&app.config().migrations_dir);
    std::fs::create_dir_all(dir)
        .map_err(|e| AppError::internal(format!("cannot create {}: {e}", dir.display())))?;

    let snapshot = app.db().collections.all();
    let items: Vec<Value> = snapshot
        .all
        .iter()
        .filter(|c| !c.system)
        .map(|c| c.to_json())
        .collect();
    let names: Vec<&str> = snapshot
        .all
        .iter()
        .filter(|c| !c.system)
        .map(|c| c.name.as_str())
        .collect();

    let path = dir.join(format!(
        "{}_collections_snapshot.js",
        chrono::Utc::now().timestamp()
    ));
    let body = format!(
        "/// <reference path=\"../pb_data/types.d.ts\" />\n\
         migrate((app) => {{\n\
         \x20 const snapshot = {};\n\
         \x20 for (const data of snapshot) {{\n\
         \x20   app.save(new Collection(data));\n\
         \x20 }}\n\
         }}, (app) => {{\n\
         \x20 const names = {};\n\
         \x20 for (const name of names.slice().reverse()) {{\n\
         \x20   const c = app.findCollectionByNameOrId(name);\n\
         \x20   if (c) app.delete(c);\n\
         \x20 }}\n\
         }});\n",
        serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into()),
        serde_json::to_string(&names).unwrap_or_else(|_| "[]".into()),
    );
    std::fs::write(&path, body)
        .map_err(|e| AppError::internal(format!("cannot write {}: {e}", path.display())))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use cratebase_core::{Collection, CollectionType, Field, FieldKind};
    use cratebase_db::Executor;

    fn text_field(name: &str) -> Field {
        Field::new(
            name,
            FieldKind::Text {
                min: 0,
                max: 0,
                pattern: String::new(),
                autogenerate_pattern: String::new(),
                primary_key: false,
            },
        )
    }

    /// A fully-formed `widgets` collection JSON, exactly like the JSON a
    /// real PocketBase-style migration embeds (id, timestamps and every
    /// system field already assigned) — `$app.save` on a JS `Collection`
    /// does not run the HTTP route's `prepare_new` normalization
    /// (id/system-field assignment), so a hand-written migration has to
    /// supply a complete collection the same way `migrate collections`
    /// (`write_collections_snapshot`) does.
    fn widgets_json() -> Value {
        let mut c = Collection::new("widgets", CollectionType::Base);
        c.fields.push(text_field("title"));
        c.assign_field_ids();
        c.to_json()
    }

    const MIGRATION_FILE: &str = "1700000000_create_widgets.js";

    fn create_widgets_migration() -> String {
        format!(
            "migrate((app) => {{\n  app.save(new Collection({}));\n}}, (app) => {{\n  const c = app.findCollectionByNameOrId(\"widgets\");\n  if (c) app.delete(c);\n}});\n",
            serde_json::to_string(&widgets_json()).expect("serialize collection"),
        )
    }

    /// A temp-dir app with `js` written as the single pending migration.
    /// Mirrors `jsvm_host`'s own `test_app_with_hook` helper, but for
    /// `pb_migrations/` with no `pb_hooks/` file at all — the runtime must
    /// still start (see `jsvm_host::maybe_start`).
    async fn app_with_migration(js: &str) -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let cfg = Config::memory(dir.path().join("pb_data"));
        std::fs::create_dir_all(&cfg.migrations_dir).expect("create migrations dir");
        std::fs::write(
            std::path::Path::new(&cfg.migrations_dir).join(MIGRATION_FILE),
            js,
        )
        .expect("write migration file");
        let app = App::new(cfg);
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    #[tokio::test]
    async fn pending_migration_is_applied_and_recorded() {
        let (app, _dir) = app_with_migration(&create_widgets_migration()).await;
        assert!(
            app.jsvm().is_some(),
            "the JS runtime must start for a pending migration even with no pb_hooks file"
        );

        let applied = run_up(&app).await.expect("run_up");
        assert_eq!(applied, vec![MIGRATION_FILE.to_string()]);
        assert!(app.db().collections.get("widgets").is_some());
        assert!(cratebase_db::migrations::is_applied(app.db(), MIGRATION_FILE)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn running_up_again_is_a_no_op() {
        let (app, _dir) = app_with_migration(&create_widgets_migration()).await;
        run_up(&app).await.expect("first run_up");
        let applied = run_up(&app).await.expect("second run_up");
        assert!(applied.is_empty());
    }

    #[tokio::test]
    async fn migrate_down_reverts_the_last_migration() {
        let (app, _dir) = app_with_migration(&create_widgets_migration()).await;
        run_up(&app).await.expect("run_up");
        assert!(app.db().collections.get("widgets").is_some());

        let reverted = run_down(&app, 1).await.expect("run_down");
        assert_eq!(reverted, vec![MIGRATION_FILE.to_string()]);
        assert!(app.db().collections.get("widgets").is_none());
        assert!(!cratebase_db::migrations::is_applied(app.db(), MIGRATION_FILE)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn a_throwing_migration_rolls_back_and_is_not_recorded() {
        // `$app.save`/`$app.delete` on a *record* join the migration's own
        // `$app.runInTransaction` (`prelude.js`'s `runMigration`); a
        // collection save does not (`JsvmHost::save_collection` always
        // writes straight through the engine, matching every core Rust
        // migration's own `db.collections.insert` calls), so the
        // collection this test asserts on has to already exist for the
        // rollback under test to be the record write, not a schema change.
        let js = "migrate((app) => {\n  const r = new Record(app.findCollectionByNameOrId(\"widgets\"));\n  r.set(\"title\", \"one\");\n  app.save(r);\n  throw new Error(\"boom\");\n}, (app) => {});\n";
        let (app, _dir) = app_with_migration(js).await;
        let mut widgets = Collection::new("widgets", CollectionType::Base);
        widgets.fields.push(text_field("title"));
        widgets.assign_field_ids();
        app.db()
            .collections
            .insert(&*app.db().engine, &widgets)
            .await
            .expect("seed widgets collection");

        let err = run_up(&app).await.expect_err("the migration throws");
        assert!(err.to_string().contains("boom"), "{err}");

        let count = app
            .db()
            .query_scalar(r#"SELECT COUNT(*) FROM "widgets""#, &[])
            .await
            .unwrap()
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        assert_eq!(count, 0, "the record save must have rolled back");
        assert!(!cratebase_db::migrations::is_applied(app.db(), MIGRATION_FILE)
            .await
            .unwrap());
    }
}
