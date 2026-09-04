//! `/api/schema/apply` — "schema as code" for non-system collections.
//!
//! `PUT /api/collections/import` (see `routes::collections`) already
//! creates/updates/deletes whole collections from a JSON payload, but it
//! replaces each collection's `fields` wholesale: anything the payload
//! omits is gone, silently. That is fine for the dashboard's "import"
//! button (a human just edited the export and expects an exact mirror),
//! but wrong for a file checked into a repo and re-applied on every
//! deploy — a teammate who deletes a field locally without meaning to
//! ship that removal would take production data with them.
//!
//! This endpoint is diff-first instead of replace-first:
//!
//! 1. **plan** — for each non-system collection in the payload, compare
//!    it against the live snapshot and record which fields would be
//!    added, retyped or removed, and whether any rule/index changed;
//! 2. **report** — always returned, so `?dryRun=1` (or a CI check) can
//!    see the plan without touching the database;
//! 3. **apply** — added/changed fields are written unconditionally
//!    (they cannot lose data); a field *removal* is only written when
//!    the caller passes `?force=1`, otherwise it is left in place and
//!    reported with `pendingRemoval: true`. This is the confirmation
//!    flag the task calls for: no data-holding column is ever dropped
//!    by re-running a schema file that simply omits it.
//!
//! The payload is exactly what `GET /api/collections` returns (or its
//! `items` array, or a bare array of collection objects) — the same
//! shape a project checks into its repo as "schema as code" and the
//! same shape `routes::collections::import` already accepts under
//! `collections`.

use std::collections::HashSet;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use cratebase_core::{AppError, Collection, DateTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::App;
use crate::extract::{RequestInfo, RequireSuperuser};
use crate::http_error::{ApiError, ApiJson, ApiQuery, ApiResult};
use crate::routes::collections;

const APPLY_FAILED: &str = "Failed to apply schema.";
const BAD_SCHEMA_DOC: &str =
    "Expected a schema document: an array of collections, or an object with a `collections` or `items` array.";

pub fn router() -> Router<App> {
    Router::new().route("/schema/apply", post(apply))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplyQuery {
    /// Compute and return the plan without writing anything.
    dry_run: Option<String>,
    /// Actually drop fields the payload omits. Without this, a field
    /// removal is reported (`pendingRemoval: true`) but never applied.
    force: Option<String>,
}

/// Same truthy spellings `ListQuery::skip_total` accepts, so every
/// boolean query flag on `/api/collections*` behaves identically.
fn truthy(flag: &Option<String>) -> bool {
    matches!(flag.as_deref(), Some("1" | "true" | "TRUE" | "True"))
}

/// One collection's plan: what differs between the payload and the live
/// schema, and what happened (or would happen) about it.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionDiff {
    pub name: String,
    /// `create`, `update` or `unchanged`.
    pub action: &'static str,
    pub fields_added: Vec<String>,
    pub fields_removed: Vec<String>,
    pub fields_changed: Vec<String>,
    /// `fieldsRemoved` is non-empty and `force` was not set: the removal
    /// was left in place rather than applied.
    pub pending_removal: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDiff {
    pub collections: Vec<CollectionDiff>,
    /// Echoes the query flags, so a client can tell a dry-run report
    /// apart from one that was actually applied.
    pub applied: bool,
    pub force: bool,
}

/// `POST /api/schema/apply`. Superuser-only, like every other
/// `/api/collections*` route.
async fn apply(
    State(app): State<App>,
    su: RequireSuperuser,
    info: RequestInfo,
    ApiQuery(query): ApiQuery<ApplyQuery>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<SchemaDiff>> {
    let diff = plan_and_apply(
        &app,
        &body,
        truthy(&query.dry_run),
        truthy(&query.force),
        &info,
        Some(su.0),
    )
    .await?;
    Ok(Json(diff))
}

/// Diff `body` (the shapes `extract_collections` accepts) against the
/// live schema and, unless `dry_run`, write the difference. Shared by
/// the HTTP handler above and the `cratebase schema push` CLI command,
/// so the two never drift into different diffing rules.
pub async fn plan_and_apply(
    app: &App,
    body: &Value,
    dry_run: bool,
    force: bool,
    info: &RequestInfo,
    auth: Option<crate::extract::Auth>,
) -> ApiResult<SchemaDiff> {
    let mut diffs = Vec::new();
    for value in extract_collections(body)? {
        let incoming = collections::deserialize(value)?;
        if incoming.system {
            // Schema-as-code only ever describes app-owned collections;
            // a checked-in doc that happens to include a system
            // collection (e.g. round-tripped from `GET /api/collections`
            // verbatim) is silently skipped rather than rejected.
            continue;
        }
        let existing = app
            .db()
            .collections
            .get_by_id(&incoming.id)
            .or_else(|| app.db().collections.get_by_name(&incoming.name));

        let (diff, plan) = plan_collection(app, existing.as_deref(), incoming, force)?;
        diffs.push(diff);

        if !dry_run {
            if let Some(plan) = plan {
                let (next, previous, change) = match plan {
                    Plan::Create(next) => (next, None, collections::Change::Create),
                    Plan::Update(next, previous) => {
                        (next, Some(*previous), collections::Change::Update)
                    }
                };
                collections::apply(app, next, previous, change, info, auth.clone()).await?;
            }
        }
    }

    Ok(SchemaDiff {
        collections: diffs,
        applied: !dry_run,
        force,
    })
}

/// A pending write, already validated and ready for
/// `collections::apply`.
enum Plan {
    Create(Collection),
    Update(Collection, Box<Collection>),
}

/// Compare `incoming` against `existing` (`None` for a brand-new
/// collection), returning the report and, when there is anything to
/// write, the plan to write it.
fn plan_collection(
    app: &App,
    existing: Option<&Collection>,
    incoming: Collection,
    force: bool,
) -> ApiResult<(CollectionDiff, Option<Plan>)> {
    let Some(existing) = existing else {
        let mut next = incoming;
        collections::prepare_new(&mut next);
        if next.is_view() {
            next.fields = collections::view_fields(app, &next, APPLY_FAILED)?;
        }
        collections::validate(app, &next, None, APPLY_FAILED)?;
        let fields_added = next
            .fields
            .iter()
            .filter(|f| !f.system)
            .map(|f| f.name.clone())
            .collect();
        let diff = CollectionDiff {
            name: next.name.clone(),
            action: "create",
            fields_added,
            ..Default::default()
        };
        return Ok((diff, Some(Plan::Create(next))));
    };

    let incoming_names: HashSet<String> = incoming.fields.iter().map(|f| f.name.clone()).collect();

    let mut fields_added = Vec::new();
    let mut fields_changed = Vec::new();
    for field in &incoming.fields {
        if field.system {
            continue;
        }
        match existing.fields.iter().find(|f| f.name == field.name) {
            None => fields_added.push(field.name.clone()),
            Some(current) if current.kind != field.kind || current.required != field.required => {
                fields_changed.push(field.name.clone())
            }
            Some(_) => {}
        }
    }
    let fields_removed: Vec<String> = existing
        .fields
        .iter()
        .filter(|f| !f.system && !incoming_names.contains(f.name.as_str()))
        .map(|f| f.name.clone())
        .collect();

    let rules_changed = existing.list_rule != incoming.list_rule
        || existing.view_rule != incoming.view_rule
        || existing.create_rule != incoming.create_rule
        || existing.update_rule != incoming.update_rule
        || existing.delete_rule != incoming.delete_rule
        || existing.indexes != incoming.indexes
        || existing.view_query != incoming.view_query;

    let pending_removal = !fields_removed.is_empty() && !force;
    let removal_applies = !fields_removed.is_empty() && force;
    let unchanged =
        fields_added.is_empty() && fields_changed.is_empty() && !removal_applies && !rules_changed;

    if unchanged {
        let diff = CollectionDiff {
            name: existing.name.clone(),
            action: "unchanged",
            fields_removed,
            pending_removal,
            ..Default::default()
        };
        return Ok((diff, None));
    }

    let mut next = incoming;
    next.id = existing.id.clone();
    next.system = existing.system;
    next.created = existing.created;
    next.updated = DateTime::now();
    // Token secrets are never serialized in `to_json`/exported, so a
    // schema doc round-tripped from `GET /api/collections` never carries
    // them; carry them over exactly like `routes::collections::update`
    // does, or every apply would invalidate every session on an auth
    // collection.
    next.auth.auth_token.secret = existing.auth.auth_token.secret.clone();
    next.auth.file_token.secret = existing.auth.file_token.secret.clone();
    next.auth.verification_token.secret = existing.auth.verification_token.secret.clone();
    next.auth.password_reset_token.secret = existing.auth.password_reset_token.secret.clone();
    next.auth.email_change_token.secret = existing.auth.email_change_token.secret.clone();

    if !force {
        // Put back every field the payload omitted: an omission is a
        // *proposed* removal, not an instruction, unless `force` says
        // otherwise.
        for field in &existing.fields {
            if !field.system && !incoming_names.contains(field.name.as_str()) {
                next.fields.push(field.clone());
            }
        }
    }
    collections::dedupe_fields(&mut next);
    next.ensure_system_fields();
    next.assign_field_ids();
    if next.is_view() {
        next.fields = collections::view_fields(app, &next, APPLY_FAILED)?;
    }
    collections::validate(app, &next, Some(existing), APPLY_FAILED)?;

    let diff = CollectionDiff {
        name: next.name.clone(),
        action: "update",
        fields_added,
        fields_removed,
        fields_changed,
        pending_removal,
    };
    Ok((diff, Some(Plan::Update(next, Box::new(existing.clone())))))
}

/// Accept the payload shapes described in the module doc: a bare array,
/// `{"collections": [...]}` (matches `routes::collections::import`) or
/// `{"items": [...]}` (matches `GET /api/collections}` verbatim).
fn extract_collections(body: &Value) -> ApiResult<Vec<Value>> {
    match body {
        Value::Array(items) => Ok(items.clone()),
        Value::Object(map) => map
            .get("collections")
            .or_else(|| map.get("items"))
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| ApiError::bad_request(BAD_SCHEMA_DOC)),
        _ => Err(ApiError(AppError::bad_request(BAD_SCHEMA_DOC))),
    }
}

#[cfg(test)]
mod tests {
    use cratebase_core::{CollectionType, Field, FieldKind};
    use serde_json::json;

    use super::*;
    use crate::config::Config;

    /// A bootstrapped app over an in-memory database, mirroring
    /// `tests/api.rs`'s harness but local to this `--lib` test module.
    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    async fn create_collection(app: &App, name: &str, fields: Vec<Field>) -> Collection {
        let mut next = Collection::new(name, CollectionType::Base);
        next.fields = fields;
        collections::prepare_new(&mut next);
        collections::validate(app, &next, None, "test").unwrap();
        let info = RequestInfo::default();
        (*collections::apply(app, next, None, collections::Change::Create, &info, None)
            .await
            .unwrap())
        .clone()
    }

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

    #[tokio::test]
    async fn unchanged_schema_is_a_noop() {
        let (app, _dir) = test_app().await;
        let created = create_collection(&app, "widgets", vec![text_field("title")]).await;

        let doc = json!({ "collections": [created.to_json()] });
        let extracted = extract_collections(&doc).unwrap();
        let incoming = collections::deserialize(extracted[0].clone()).unwrap();
        let existing = app.db().collections.get_by_id(&created.id).unwrap();
        let (diff, plan) = plan_collection(&app, Some(&existing), incoming, false).unwrap();

        assert_eq!(diff.action, "unchanged");
        assert!(plan.is_none());
    }

    #[tokio::test]
    async fn added_field_applies() {
        let (app, _dir) = test_app().await;
        let created = create_collection(&app, "gizmos", vec![text_field("title")]).await;

        let mut doc = created.to_json();
        doc["fields"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(text_field("subtitle")).unwrap());
        let incoming = collections::deserialize(doc).unwrap();
        let existing = app.db().collections.get_by_id(&created.id).unwrap();
        let (diff, plan) = plan_collection(&app, Some(&existing), incoming, false).unwrap();

        assert_eq!(diff.action, "update");
        assert_eq!(diff.fields_added, vec!["subtitle".to_string()]);
        let Some(Plan::Update(next, previous)) = plan else {
            panic!("expected an update plan");
        };
        let info = RequestInfo::default();
        collections::apply(
            &app,
            next,
            Some(*previous),
            collections::Change::Update,
            &info,
            None,
        )
        .await
        .unwrap();

        let reloaded = app.db().collections.get_by_id(&created.id).unwrap();
        assert!(reloaded.fields.iter().any(|f| f.name == "subtitle"));
    }

    #[tokio::test]
    async fn removed_field_requires_force() {
        let (app, _dir) = test_app().await;
        let created = create_collection(
            &app,
            "sprockets",
            vec![text_field("title"), text_field("subtitle")],
        )
        .await;

        // The payload omits `subtitle`.
        let mut doc = created.to_json();
        doc["fields"] = json!(created
            .fields
            .iter()
            .filter(|f| f.name != "subtitle")
            .cloned()
            .collect::<Vec<_>>());

        // Without `force`: reported, not applied.
        let incoming = collections::deserialize(doc.clone()).unwrap();
        let existing = app.db().collections.get_by_id(&created.id).unwrap();
        let (diff, plan) = plan_collection(&app, Some(&existing), incoming, false).unwrap();
        assert_eq!(diff.fields_removed, vec!["subtitle".to_string()]);
        assert!(diff.pending_removal);
        assert!(plan.is_none());
        let still_there = app.db().collections.get_by_id(&created.id).unwrap();
        assert!(still_there.fields.iter().any(|f| f.name == "subtitle"));

        // With `force`: applied.
        let incoming = collections::deserialize(doc).unwrap();
        let existing = app.db().collections.get_by_id(&created.id).unwrap();
        let (diff, plan) = plan_collection(&app, Some(&existing), incoming, true).unwrap();
        assert_eq!(diff.fields_removed, vec!["subtitle".to_string()]);
        assert!(!diff.pending_removal);
        let Some(Plan::Update(next, previous)) = plan else {
            panic!("expected an update plan");
        };
        let info = RequestInfo::default();
        collections::apply(
            &app,
            next,
            Some(*previous),
            collections::Change::Update,
            &info,
            None,
        )
        .await
        .unwrap();
        let reloaded = app.db().collections.get_by_id(&created.id).unwrap();
        assert!(!reloaded.fields.iter().any(|f| f.name == "subtitle"));
    }

    #[test]
    fn extract_collections_accepts_every_shape() {
        let bare = json!([{"name": "a"}]);
        assert_eq!(extract_collections(&bare).unwrap().len(), 1);
        let wrapped = json!({"collections": [{"name": "a"}]});
        assert_eq!(extract_collections(&wrapped).unwrap().len(), 1);
        let items = json!({"items": [{"name": "a"}]});
        assert_eq!(extract_collections(&items).unwrap().len(), 1);
        assert!(extract_collections(&json!("nope")).is_err());
    }
}
