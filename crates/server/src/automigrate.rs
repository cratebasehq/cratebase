//! `--automigrate` / `CB_AUTOMIGRATE`: mirrors PocketBase's prebuilt-binary
//! behaviour of writing a `pb_migrations/*.js` file for every collection
//! change made through the dashboard/API, so a fresh environment can
//! replay the exact same schema history instead of the operator having to
//! remember or hand-write it.
//!
//! # Default and gating
//!
//! `automigrate` itself defaults to `true` (`Config::for_data_dir`), but a
//! file is only ever written when `pb_migrations/` already exists *or*
//! `--dev` is set (see [`should_write`]). Writing into a directory that
//! does not exist would conjure a new top-level `pb_migrations/` next to
//! every production data dir the moment a superuser edits a collection in
//! the dashboard — surprising for an operator who never opted into JS
//! migrations at all. Creating the directory once (`mkdir pb_migrations`,
//! or `cratebase migrate create`/`migrate collections`, which both do it
//! already) or running with `--dev` opts in.
//!
//! # What gets written
//!
//! `up()` applies the change (saves the new collection JSON, or deletes
//! it); `down()` reverts it (restores the previous JSON, or re-creates
//! it) — the same "before/after JSON" shape
//! [`crate::js_migrations::write_collections_snapshot`] uses. The file is
//! recorded in the ledger as already applied (see
//! `crate::js_migrations::DbMigrationLedger`) immediately, since the
//! change it describes has, by construction, already happened — without
//! this, the very next `serve` boot or `migrate up` would try to re-apply
//! it and fail (the collection already exists / no longer exists).

use chrono::Utc;
use serde_json::Value;

use crate::app::App;
use crate::events::CollectionRequestEvent;
use crate::hooks::{Event, Handler};

/// Register the three collection-lifecycle hooks that write migration
/// files. Called once from [`App::bootstrap`], unconditionally — the
/// hooks themselves check [`should_write`] on every fire, so toggling
/// `CB_AUTOMIGRATE` (or creating `pb_migrations/`) takes effect on the
/// next change without a restart.
pub fn bind_hooks(app: &App) {
    app.hooks().on_collection_create_request.bind(Handler::new(
        |e: &mut CollectionRequestEvent| {
            Box::pin(async move {
                e.next().await?;
                if let Some(created) = e.collection.clone() {
                    write_migration(
                        &e.app,
                        "created",
                        &created.name,
                        None,
                        Some(created.to_json()),
                    )
                    .await;
                }
                Ok(())
            })
        },
    ));
    app.hooks().on_collection_update_request.bind(Handler::new(
        |e: &mut CollectionRequestEvent| {
            Box::pin(async move {
                // Same timing note as `crate::audit::bind_collection_lifecycle`:
                // the store only picks up the pending write once this whole
                // chain returns, so this is still the pre-update row.
                let before = e
                    .collection
                    .as_ref()
                    .and_then(|c| e.app.db().collections.get_by_id(&c.id))
                    .map(|c| c.to_json());
                e.next().await?;
                if let (Some(before), Some(after)) = (before, e.collection.clone()) {
                    write_migration(
                        &e.app,
                        "updated",
                        &after.name,
                        Some(before),
                        Some(after.to_json()),
                    )
                    .await;
                }
                Ok(())
            })
        },
    ));
    app.hooks().on_collection_delete_request.bind(Handler::new(
        |e: &mut CollectionRequestEvent| {
            Box::pin(async move {
                let deleted = e.collection.clone();
                e.next().await?;
                if let Some(deleted) = deleted {
                    write_migration(
                        &e.app,
                        "deleted",
                        &deleted.name,
                        Some(deleted.to_json()),
                        None,
                    )
                    .await;
                }
                Ok(())
            })
        },
    ));
}

/// `true` when a collection change should be written to `pb_migrations/`
/// right now — see the module doc for why this is more than just
/// `config.automigrate`.
fn should_write(app: &App) -> bool {
    let config = app.config();
    config.automigrate && (config.dev || std::path::Path::new(&config.migrations_dir).is_dir())
}

/// Best-effort: a failure here must never fail the collection write itself
/// (the same "log and move on" stance `crate::audit`'s writer takes), so
/// this only logs and returns.
async fn write_migration(
    app: &App,
    action: &str,
    name: &str,
    before: Option<Value>,
    after: Option<Value>,
) {
    if !should_write(app) {
        return;
    }
    if let Err(e) = try_write_migration(app, action, name, before, after).await {
        tracing::warn!(error = %e, collection = %name, action, "automigrate: failed to write migration file");
    }
}

async fn try_write_migration(
    app: &App,
    action: &str,
    name: &str,
    before: Option<Value>,
    after: Option<Value>,
) -> Result<(), cratebase_core::AppError> {
    let dir = std::path::Path::new(&app.config().migrations_dir);
    std::fs::create_dir_all(dir).map_err(|e| {
        cratebase_core::AppError::internal(format!("cannot create {}: {e}", dir.display()))
    })?;

    let name_json = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into());
    let save = |json: &Value| format!("app.save(new Collection({}));", to_json_str(json));
    let delete_by_name =
        format!("const c = app.findCollectionByNameOrId({name_json}); if (c) app.delete(c);");
    let (up, down) = match (before.as_ref(), after.as_ref()) {
        (None, Some(after)) => (save(after), delete_by_name),
        (Some(before), Some(after)) => (save(after), save(before)),
        (Some(before), None) => (delete_by_name, save(before)),
        (None, None) => return Ok(()),
    };

    let slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = dir.join(format!("{}_{action}_{slug}.js", Utc::now().timestamp()));
    let body = format!(
        "/// <reference path=\"../pb_data/types.d.ts\" />\n\
         migrate((app) => {{\n  {up}\n}}, (app) => {{\n  {down}\n}});\n"
    );
    std::fs::write(&path, body).map_err(|e| {
        cratebase_core::AppError::internal(format!("cannot write {}: {e}", path.display()))
    })?;

    // Recorded as already applied: this file describes a change that has,
    // by construction, already happened through the API/dashboard write
    // this hook fired from — without this, the next boot's `js_migrations`
    // run would try to re-apply it (and fail: the collection already
    // exists, or no longer does).
    let file = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    cratebase_db::migrations::mark_applied(app.db(), &file)
        .await
        .map_err(cratebase_core::AppError::from)
}

fn to_json_str(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}
