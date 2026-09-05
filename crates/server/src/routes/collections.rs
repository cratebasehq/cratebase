//! `/api/collections` — the schema API, superuser only.
//!
//! Every mutation follows the same three steps:
//!
//! 1. **merge** — `PATCH` is a partial update, so the body is merged onto
//!    the stored collection's JSON and the result is re-deserialized.
//!    That is what lets a client send `{indexes: []}` without also
//!    resending the field list;
//! 2. **validate** — identifier rules, case-insensitive name uniqueness,
//!    system-collection protection, `type` immutability, rule expressions
//!    (compiled against the collection *being saved*, not the stored one)
//!    and index statements;
//! 3. **apply** — `cratebase_db::collections` writes the `_collections`
//!    row and runs the DDL in one transaction, after which the in-memory
//!    snapshot is reloaded.
//!
//! Errors here are shaped differently from the record API: list-typed
//! properties report per-index (`data.indexes["0"]`), and an unknown
//! field `type` is a *body-format* error rather than a field error
//! (KNOWN_DIVERGENCES §6, §7).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete as delete_method, get, put};
use axum::{Json, Router};
use cratebase_core::{
    codes, AppError, Collection, CollectionType, DateTime, Field, FieldError, FieldKind, FieldType,
};
use cratebase_db::collections as store_ops;
use cratebase_db::engine::quote_ident;
use cratebase_db::{schema, DbError};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::events::{collection_tags, CollectionEvent, CollectionRequestEvent};
use crate::extract::{RequestInfo, RequireSuperuser};
use crate::http_error::{ApiError, ApiJson, ApiQuery, ApiResult};

const CREATE_FAILED: &str = "Failed to create collection.";
const UPDATE_FAILED: &str = "Failed to update collection.";
const DELETE_FAILED: &str = "Failed to delete collection.";
const IMPORT_FAILED: &str = "Failed to import collections.";
/// PocketBase's message for a body it could not even shape into a
/// collection (an unknown field `type`, say).
const BAD_PAYLOAD: &str = "Failed to load the submitted data due to invalid formatting.";

/// `oauth2.providers[].clientSecret` is stored (`Collection::to_json`
/// now serializes it — see `cratebase_core::OAuth2Provider`'s doc) but
/// must never round-trip back out to a caller, same as any other
/// write-only secret. Every response this module hands back goes
/// through this first; the merge base in [`update`] does not, since
/// that one has to keep the real value to preserve it across a PATCH.
fn redact_oauth2_secrets(mut value: Value) -> Value {
    if let Some(providers) = value
        .get_mut("oauth2")
        .and_then(|o| o.get_mut("providers"))
        .and_then(Value::as_array_mut)
    {
        for provider in providers {
            if let Some(obj) = provider.as_object_mut() {
                obj.insert("clientSecret".into(), Value::String(String::new()));
            }
        }
    }
    value
}

pub fn router() -> Router<App> {
    Router::new()
        .route("/collections", get(list).post(create))
        .route("/collections/import", put(import))
        .route("/collections/meta/scaffolds", get(scaffolds))
        .route(
            "/collections/{idOrName}",
            get(view).patch(update).delete(remove),
        )
        .route("/collections/{idOrName}/truncate", delete_method(truncate))
}

// --------------------------------------------------------------------- list

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    page: Option<i64>,
    per_page: Option<i64>,
    filter: Option<String>,
    sort: Option<String>,
    fields: Option<String>,
    skip_total: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct Page {
    page: i64,
    #[serde(rename = "perPage")]
    per_page: i64,
    #[serde(rename = "totalItems")]
    total_items: i64,
    #[serde(rename = "totalPages")]
    total_pages: i64,
    items: Value,
}

/// `GET /api/collections`. Filtering and sorting run against the
/// in-memory snapshot — `_collections` is never queried on a request
/// path, and the whole schema is a handful of kilobytes.
async fn list(
    State(app): State<App>,
    su: RequireSuperuser,
    ApiQuery(query): ApiQuery<ListQuery>,
    info: RequestInfo,
) -> ApiResult<Json<Page>> {
    let mut event = CollectionRequestEvent::new(app.clone(), info, Some(su.0), None, Vec::new());
    app.hooks()
        .on_collections_list_request
        .trigger_bare(&mut event)
        .await
        .map_err(ApiError)?;

    let snapshot = app.db().collections.all();
    let mut rows: Vec<Value> = snapshot
        .all
        .iter()
        .map(|c| redact_oauth2_secrets(c.to_json()))
        .collect();

    if let Some(expr) = query
        .filter
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let ast = cratebase_filter::parse_cached(expr)?;
        let resolver = MetaResolver::shared();
        let mut kept = Vec::with_capacity(rows.len());
        for row in rows {
            let object = row.as_object().cloned().unwrap_or_default();
            if cratebase_filter::evaluate(&ast, &object, resolver)
                .map_err(|_| ApiError(AppError::bad_request("")))?
            {
                kept.push(row);
            }
        }
        rows = kept;
    }

    if let Some(sort) = query
        .sort
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        sort_rows(&mut rows, sort);
    }

    let total_items = rows.len() as i64;
    let per_page = query.per_page.unwrap_or(500).clamp(1, 1000);
    let page = query.page.unwrap_or(1).max(1);
    let offset = ((page - 1) * per_page).max(0) as usize;
    let items: Vec<Value> = rows
        .into_iter()
        .skip(offset)
        .take(per_page as usize)
        .collect();
    let skip_total = matches!(
        query.skip_total.as_deref(),
        Some("1" | "true" | "TRUE" | "True")
    );

    let mut items = Value::Array(items);
    crate::routes::common::project(&mut items, query.fields.as_deref());
    Ok(Json(Page {
        page,
        per_page,
        total_items: if skip_total { -1 } else { total_items },
        total_pages: if skip_total {
            -1
        } else {
            (total_items + per_page - 1) / per_page
        },
        items,
    }))
}

/// A tiny multi-key sort over the serialized collections.
fn sort_rows(rows: &mut [Value], sort: &str) {
    let keys: Vec<(String, bool)> = sort
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|token| match token.strip_prefix('-') {
            Some(rest) => (rest.trim().to_string(), true),
            None => (
                token.strip_prefix('+').unwrap_or(token).trim().to_string(),
                false,
            ),
        })
        .collect();
    rows.sort_by(|a, b| {
        for (key, descending) in &keys {
            let ordering = compare(a.get(key), b.get(key));
            let ordering = if *descending {
                ordering.reverse()
            } else {
                ordering
            };
            if ordering != std::cmp::Ordering::Equal {
                return ordering;
            }
        }
        std::cmp::Ordering::Equal
    });
}

fn compare(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(Value::String(x)), Some(Value::String(y))) => x.cmp(y),
        (Some(Value::Number(x)), Some(Value::Number(y))) => x
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&y.as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal),
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x.cmp(y),
        (Some(x), Some(y)) => x.to_string().cmp(&y.to_string()),
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

/// The virtual collection a `?filter=` on `/api/collections` compiles
/// against. Evaluation is in-process, so only the field *types* matter.
struct MetaResolver {
    collection: Collection,
}

impl MetaResolver {
    /// Built once for the process: the virtual schema never changes, so
    /// rebuilding it per request would allocate a whole `Collection` just
    /// to look up a couple of field types.
    fn shared() -> &'static MetaResolver {
        static SHARED: std::sync::OnceLock<MetaResolver> = std::sync::OnceLock::new();
        SHARED.get_or_init(MetaResolver::new)
    }

    fn new() -> Self {
        let mut collection = Collection::new("_collections", CollectionType::Base);
        let text = |name: &str| Field::new(name, FieldKind::default_for(FieldType::Text));
        let extra = vec![
            text("name"),
            text("type"),
            Field::new("system", FieldKind::Bool {}),
            text("listRule"),
            text("viewRule"),
            text("createRule"),
            text("updateRule"),
            text("deleteRule"),
        ];
        let position = collection.fields.len().saturating_sub(2);
        collection.fields.splice(position..position, extra);
        collection.assign_field_ids();
        MetaResolver { collection }
    }
}

impl cratebase_filter::Resolver for MetaResolver {
    fn root(&self) -> &Collection {
        &self.collection
    }
    fn collection(&self, _name_or_id: &str) -> Option<Arc<Collection>> {
        None
    }
    fn request_value(&self, _path: &cratebase_filter::RequestPath) -> Value {
        Value::Null
    }
    fn body_has(&self, _key: &str) -> bool {
        false
    }
    fn dialect(&self) -> cratebase_filter::Dialect {
        cratebase_filter::Dialect::Sqlite
    }
}

// --------------------------------------------------------------------- view

async fn view(
    State(app): State<App>,
    su: RequireSuperuser,
    Path(id_or_name): Path<String>,
    info: RequestInfo,
) -> ApiResult<Json<Value>> {
    let collection = app
        .db()
        .collections
        .get(&id_or_name)
        .ok_or_else(|| ApiError::not_found(""))?;
    let mut event = CollectionRequestEvent::new(
        app.clone(),
        info,
        Some(su.0),
        Some((*collection).clone()),
        collection_tags(&collection),
    );
    app.hooks()
        .on_collection_view_request
        .trigger_bare(&mut event)
        .await
        .map_err(ApiError)?;
    Ok(Json(redact_oauth2_secrets(
        event
            .collection
            .map(|c| c.to_json())
            .unwrap_or_else(|| collection.to_json()),
    )))
}

// ------------------------------------------------------------------- create

async fn create(
    State(app): State<App>,
    su: RequireSuperuser,
    info: RequestInfo,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let body = object(body)?;
    let mut next = deserialize(Value::Object(body.clone()))?;
    prepare_new(&mut next);
    if next.is_view() {
        next.fields = view_fields(&app, &next, CREATE_FAILED)?;
    }
    validate(&app, &next, None, CREATE_FAILED)?;

    let info = info.with_body(body);
    let saved = apply(&app, next, None, Change::Create, &info, Some(su.0)).await?;
    Ok(Json(redact_oauth2_secrets(saved.to_json())))
}

// ------------------------------------------------------------------- update

async fn update(
    State(app): State<App>,
    su: RequireSuperuser,
    Path(id_or_name): Path<String>,
    info: RequestInfo,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let existing = app
        .db()
        .collections
        .get(&id_or_name)
        .ok_or_else(|| ApiError::not_found(""))?;
    let patch = object(body)?;

    // `PATCH` is partial: merge onto the stored JSON, then re-read the
    // whole thing so defaults and flattened auth options stay consistent.
    let mut merged = match existing.to_json() {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    for (key, value) in &patch {
        merged.insert(key.clone(), value.clone());
    }
    let mut next = deserialize(Value::Object(merged))?;
    // Token secrets are never serialized, so the merge above would have
    // silently cleared them.
    next.id = existing.id.clone();
    next.auth.auth_token.secret = existing.auth.auth_token.secret.clone();
    next.auth.file_token.secret = existing.auth.file_token.secret.clone();
    next.auth.verification_token.secret = existing.auth.verification_token.secret.clone();
    next.auth.password_reset_token.secret = existing.auth.password_reset_token.secret.clone();
    next.auth.email_change_token.secret = existing.auth.email_change_token.secret.clone();
    // `oauth2.providers[].clientSecret` is stripped from every response
    // (see `redact_oauth2_secrets`), so a PATCH built by resubmitting a
    // provider entry read back from a GET/view response would otherwise
    // silently blank a secret that was actually already set. Preserve
    // each existing provider's secret, matched by name, whenever the
    // submitted entry doesn't carry a non-empty one of its own.
    for provider in &mut next.auth.oauth2.providers {
        if provider.client_secret.is_empty() {
            if let Some(prev) = existing
                .auth
                .oauth2
                .providers
                .iter()
                .find(|p| p.name == provider.name)
            {
                provider.client_secret = prev.client_secret.clone();
            }
        }
    }
    next.system = existing.system;
    next.created = existing.created;
    next.updated = DateTime::now();
    dedupe_fields(&mut next);
    next.ensure_system_fields();
    next.assign_field_ids();
    if next.is_view() {
        next.fields = view_fields(&app, &next, UPDATE_FAILED)?;
    }
    validate(&app, &next, Some(&existing), UPDATE_FAILED)?;

    let info = info.with_body(patch);
    let saved = apply(
        &app,
        next,
        Some((*existing).clone()),
        Change::Update,
        &info,
        Some(su.0),
    )
    .await?;
    Ok(Json(redact_oauth2_secrets(saved.to_json())))
}

// ------------------------------------------------------------------- delete

async fn remove(
    State(app): State<App>,
    su: RequireSuperuser,
    Path(id_or_name): Path<String>,
    info: RequestInfo,
) -> ApiResult<StatusCode> {
    let existing = app
        .db()
        .collections
        .get(&id_or_name)
        .ok_or_else(|| ApiError::not_found(""))?;
    if existing.system {
        return Err(ApiError::bad_request(DELETE_FAILED));
    }
    if let Some(referrer) = referencing_collection(&app, &existing) {
        return Err(ApiError::bad_request(format!(
            "Failed to delete collection probably due to existing reference in {referrer}."
        )));
    }

    let info = info.with_body(Map::new());
    apply(
        &app,
        (*existing).clone(),
        Some((*existing).clone()),
        Change::Delete,
        &info,
        Some(su.0),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The name of another collection holding a relation field pointing at
/// `target`, if any. PocketBase names it in the delete error.
fn referencing_collection(app: &App, target: &Collection) -> Option<String> {
    app.db()
        .collections
        .all()
        .all
        .iter()
        .filter(|c| c.id != target.id)
        .find(|c| {
            c.fields_of_type(FieldType::Relation)
                .any(|f| f.relation_collection_id() == Some(target.id.as_str()))
        })
        .map(|c| c.name.clone())
}

// ----------------------------------------------------------------- truncate

async fn truncate(
    State(app): State<App>,
    su: RequireSuperuser,
    Path(id_or_name): Path<String>,
) -> ApiResult<StatusCode> {
    let collection = app
        .db()
        .collections
        .get(&id_or_name)
        .ok_or_else(|| ApiError::not_found(""))?;
    if collection.is_view() {
        return Err(ApiError::bad_request("Unsupported collection type."));
    }
    // `_audit_log` has to stay append-only end to end (see
    // `crate::audit`'s module doc) — no legitimate reason for even a
    // superuser to bulk-erase it, so a truncate is refused outright.
    // `_superusers` needs the same owner-only gate its ordinary record
    // writes do (see `routes::records`'s `_superusers` guard doc): a
    // truncate wipes every account including every owner, which a
    // merely-`admin` superuser must not be able to do.
    if collection.is_superusers() {
        crate::extract::RequireOwner::holds(&su.0)
            .then_some(())
            .ok_or_else(|| ApiError::forbidden("Only an owner can truncate _superusers."))?;
    } else if collection.name == crate::audit::COLLECTION {
        return Err(ApiError::bad_request("_audit_log cannot be truncated."));
    }
    let sql = format!("DELETE FROM {}", quote_ident(collection.table_name()));
    app.db()
        .engine
        .execute(&sql, &[])
        .await
        .map_err(|e| ApiError(e.into()))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- scaffolds

async fn scaffolds(State(app): State<App>, _su: RequireSuperuser) -> ApiResult<Json<Value>> {
    let _ = &app;
    let mut out = Map::new();
    for (key, kind) in [
        ("base", CollectionType::Base),
        ("auth", CollectionType::Auth),
        ("view", CollectionType::View),
    ] {
        let mut scaffold = Collection::scaffold(kind);
        // `Collection::new` seeds `created`/`updated` autodate fields for
        // convenience; PocketBase's scaffolds do not carry them
        // (KNOWN_DIVERGENCES §4), so the template is exactly the system
        // fields for the type.
        scaffold
            .fields
            .retain(|f| f.field_type() != FieldType::Autodate);
        if kind == CollectionType::Auth {
            // The two default auth indexes, with the empty table name
            // PocketBase emits for a nameless scaffold.
            scaffold.indexes = default_auth_indexes("", "");
        }
        out.insert(key.to_string(), scaffold.to_json());
    }
    Ok(Json(Value::Object(out)))
}

// ------------------------------------------------------------------- import

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportBody {
    #[serde(default)]
    collections: Vec<Value>,
    #[serde(default)]
    delete_missing: bool,
}

/// `PUT /api/collections/import`: create or update every collection in
/// the payload and, with `deleteMissing`, drop the ones it omits. The
/// whole thing runs in one transaction, so a single invalid collection
/// leaves the schema exactly as it was.
async fn import(
    State(app): State<App>,
    su: RequireSuperuser,
    info: RequestInfo,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<StatusCode> {
    let raw: ImportBody =
        serde_json::from_value(body.clone()).map_err(|_| ApiError::bad_request(BAD_PAYLOAD))?;

    let mut event = CollectionRequestEvent::new(
        app.clone(),
        info.with_body(body.as_object().cloned().unwrap_or_default()),
        Some(su.0),
        None,
        Vec::new(),
    );
    app.hooks()
        .on_collections_import_request
        .trigger_bare(&mut event)
        .await
        .map_err(ApiError)?;

    let mut planned: Vec<(Collection, Option<Arc<Collection>>)> = Vec::new();
    for value in raw.collections {
        let incoming = deserialize(value).map_err(import_error)?;
        let existing = app
            .db()
            .collections
            .get_by_id(&incoming.id)
            .or_else(|| app.db().collections.get_by_name(&incoming.name));
        let mut next = incoming;
        match &existing {
            Some(previous) => {
                next.id = previous.id.clone();
                next.system = previous.system;
                next.created = previous.created;
                next.updated = DateTime::now();
                dedupe_fields(&mut next);
                next.ensure_system_fields();
                next.assign_field_ids();
            }
            None => prepare_new(&mut next),
        }
        if next.is_view() {
            next.fields = view_fields(&app, &next, IMPORT_FAILED).map_err(import_error)?;
        }
        validate(&app, &next, existing.as_deref(), IMPORT_FAILED).map_err(import_error)?;
        planned.push((next, existing));
    }

    let keep: std::collections::HashSet<String> =
        planned.iter().map(|(c, _)| c.id.clone()).collect();
    let doomed: Vec<Arc<Collection>> = if raw.delete_missing {
        app.db()
            .collections
            .all()
            .all
            .iter()
            .filter(|c| !c.system && !keep.contains(&c.id))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    let backend = app.db().backend;
    app.run_in_transaction(move |tx| async move {
        for collection in &doomed {
            store_ops::delete_in(&tx, collection).await?;
        }
        for (next, existing) in &planned {
            match existing {
                Some(previous) => store_ops::update_in(&tx, backend, previous, next).await?,
                None => store_ops::insert_in(&tx, backend, next).await?,
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| import_error(ApiError(e)))?;

    app.db()
        .collections
        .load(&*app.db().engine)
        .await
        .map_err(|e| ApiError(e.into()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// PocketBase reports every import failure under `data.collections`.
fn import_error(error: ApiError) -> ApiError {
    let mut data = Map::new();
    let detail = error.error.body();
    data.insert(
        "collections".into(),
        serde_json::json!({
            "code": "validation_collections_import_failure",
            "message": detail.message,
            "params": detail.data,
        }),
    );
    ApiError::nested_validation(IMPORT_FAILED, data)
}

// ------------------------------------------------------------------- shared

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    Create,
    Update,
    Delete,
}

/// Fire the request hook, then the `on_collection_*` chain around the
/// actual `_collections` write and DDL, all in one transaction, and
/// refresh the in-memory snapshot afterwards.
pub(crate) async fn apply(
    app: &App,
    next: Collection,
    previous: Option<Collection>,
    change: Change,
    info: &RequestInfo,
    auth: Option<crate::extract::Auth>,
) -> ApiResult<Arc<Collection>> {
    let tags = collection_tags(&next);
    let id = next.id.clone();
    let mut request = CollectionRequestEvent::new(
        app.clone(),
        info.clone(),
        auth,
        Some(next.clone()),
        tags.clone(),
    );

    let outcome = {
        let inner_app = app.clone();
        let hook = match change {
            Change::Create => &app.hooks().on_collection_create_request,
            Change::Update => &app.hooks().on_collection_update_request,
            Change::Delete => &app.hooks().on_collection_delete_request,
        };
        hook.trigger(&mut request, move |e| {
            let next = e.collection.clone().unwrap_or_else(|| next.clone());
            let previous = previous.clone();
            let tags = tags.clone();
            let app = inner_app.clone();
            Box::pin(async move { write(app, next, previous, change, tags).await })
        })
        .await
    };
    outcome.map_err(ApiError)?;

    app.db()
        .collections
        .load(&*app.db().engine)
        .await
        .map_err(|e| ApiError(e.into()))?;

    match change {
        Change::Delete => Ok(Arc::new(Collection::default())),
        _ => app
            .db()
            .collections
            .get_by_id(&id)
            .ok_or_else(|| ApiError::internal("the saved collection vanished")),
    }
}

async fn write(
    app: App,
    next: Collection,
    previous: Option<Collection>,
    change: Change,
    tags: Vec<String>,
) -> Result<(), AppError> {
    let backend = app.db().backend;
    app.run_in_transaction(move |tx| async move {
        let mut event = CollectionEvent::new(tx.clone(), next, previous, tags.clone());
        let hooks = tx.hooks();
        let outer = match change {
            Change::Create => &hooks.on_collection_create,
            Change::Update => &hooks.on_collection_update,
            Change::Delete => &hooks.on_collection_delete,
        };
        let inner_tags = tags.clone();
        let result = outer
            .trigger(&mut event, move |e| {
                let app = e.app.clone();
                let collection = e.collection.clone();
                let previous = e.previous.clone();
                Box::pin(async move {
                    let mut inner =
                        CollectionEvent::new(app.clone(), collection, previous, inner_tags);
                    let hooks = app.hooks();
                    let execute = match change {
                        Change::Create => &hooks.on_collection_create_execute,
                        Change::Update => &hooks.on_collection_update_execute,
                        Change::Delete => &hooks.on_collection_delete_execute,
                    };
                    execute
                        .trigger(&mut inner, move |ie| {
                            Box::pin(async move {
                                let result = match change {
                                    Change::Create => {
                                        store_ops::insert_in(&ie.app, backend, &ie.collection).await
                                    }
                                    Change::Update => {
                                        let previous =
                                            ie.previous.as_ref().unwrap_or(&ie.collection);
                                        store_ops::update_in(
                                            &ie.app,
                                            backend,
                                            previous,
                                            &ie.collection,
                                        )
                                        .await
                                    }
                                    Change::Delete => {
                                        store_ops::delete_in(&ie.app, &ie.collection).await
                                    }
                                };
                                result.map_err(|e| ddl_error(e, change))
                            })
                        })
                        .await
                })
            })
            .await;

        let hooks = tx.hooks();
        match result {
            Ok(()) => {
                let hook = match change {
                    Change::Create => &hooks.on_collection_after_create_success,
                    Change::Update => &hooks.on_collection_after_update_success,
                    Change::Delete => &hooks.on_collection_after_delete_success,
                };
                hook.trigger_bare(&mut event).await?;
                Ok(())
            }
            Err(error) => {
                let hook = match change {
                    Change::Create => &hooks.on_collection_after_create_error,
                    Change::Update => &hooks.on_collection_after_update_error,
                    Change::Delete => &hooks.on_collection_after_delete_error,
                };
                let _ = hook.trigger_bare(&mut event).await;
                Err(error)
            }
        }
    })
    .await
}

/// A DDL failure. Index statements are pre-validated against the schema
/// (see [`validate_indexes`]), so what reaches here is a name clash or
/// something only the driver could know.
fn ddl_error(error: DbError, change: Change) -> AppError {
    let message = match change {
        Change::Create => CREATE_FAILED,
        Change::Update => UPDATE_FAILED,
        Change::Delete => DELETE_FAILED,
    };
    let detail = error.to_string();
    match error {
        DbError::UniqueViolation(_) => AppError::validation(
            message,
            [(
                "name".to_string(),
                FieldError::new(
                    codes::COLLECTION_NAME_EXISTS,
                    "Collection name must be unique (case insensitive).",
                ),
            )],
        ),
        DbError::Validation(fields) => AppError::validation(message, fields),
        DbError::NotFound => AppError::not_found(""),
        _ => AppError::bad_request(format!("{message}\nRaw error: {detail}")),
    }
}

// ---------------------------------------------------------------- shaping

/// The body has to be an object before anything else can be said about
/// it.
fn object(body: Value) -> ApiResult<Map<String, Value>> {
    match body {
        Value::Object(map) => Ok(map),
        _ => Err(ApiError::bad_request(BAD_PAYLOAD)),
    }
}

/// Deserialize a collection payload. An unknown field `type` (or any
/// other shape error) is a *body-format* error with an empty `data`, not
/// a per-field one — KNOWN_DIVERGENCES §6.
pub(crate) fn deserialize(value: Value) -> ApiResult<Collection> {
    serde_json::from_value::<Collection>(value).map_err(|e| {
        tracing::debug!(detail = %e, "rejected collection payload");
        ApiError::bad_request(BAD_PAYLOAD)
    })
}

/// Everything the server owns on a freshly created collection.
pub(crate) fn prepare_new(next: &mut Collection) {
    next.system = false;
    if next.id.trim().is_empty() {
        next.id = cratebase_core::collection_id(next.collection_type.as_str(), &next.name);
    }
    next.created = DateTime::now();
    next.updated = next.created;
    dedupe_fields(next);
    next.ensure_system_fields();
    next.assign_field_ids();
    if next.is_auth() && next.indexes.is_empty() {
        next.indexes = default_auth_indexes(&next.name, &next.id);
    }
}

/// PocketBase silently collapses duplicate field names, keeping the last
/// definition (KNOWN_DIVERGENCES §5).
pub(crate) fn dedupe_fields(collection: &mut Collection) {
    let mut seen: Vec<String> = Vec::with_capacity(collection.fields.len());
    let mut deduped: Vec<Field> = Vec::with_capacity(collection.fields.len());
    for field in collection.fields.drain(..) {
        match seen.iter().position(|n| *n == field.name) {
            Some(position) => deduped[position] = field,
            None => {
                seen.push(field.name.clone());
                deduped.push(field);
            }
        }
    }
    collection.fields = deduped;
}

/// The two unique indexes every auth collection ships with.
fn default_auth_indexes(table: &str, id: &str) -> Vec<String> {
    vec![
        format!("CREATE UNIQUE INDEX `idx_tokenKey_{id}` ON `{table}` (`tokenKey`)"),
        format!("CREATE UNIQUE INDEX `idx_email_{id}` ON `{table}` (`email`) WHERE `email` != ''"),
    ]
}

// -------------------------------------------------------------- validation

pub(crate) fn validate(
    app: &App,
    next: &Collection,
    previous: Option<&Collection>,
    message: &str,
) -> ApiResult<()> {
    let mut errors: std::collections::BTreeMap<String, FieldError> = Default::default();

    if !cratebase_core::is_valid_identifier(&next.name) {
        errors.insert(
            "name".into(),
            FieldError::new("validation_match_invalid", "Invalid value format."),
        );
    } else if !next.system && next.name.starts_with('_') && previous.is_none() {
        errors.insert(
            "name".into(),
            FieldError::new(
                "validation_match_invalid",
                "The '_' prefix is reserved for system collections.",
            ),
        );
    } else {
        let clash = app
            .db()
            .collections
            .all()
            .all
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case(&next.name) && c.id != next.id);
        if clash {
            errors.insert(
                "name".into(),
                FieldError::new(
                    codes::COLLECTION_NAME_EXISTS,
                    "Collection name must be unique (case insensitive).",
                ),
            );
        }
    }

    if let Some(previous) = previous {
        if previous.collection_type != next.collection_type {
            errors.insert(
                "type".into(),
                FieldError::new(
                    "validation_collection_type_change",
                    "The collection type cannot be changed.",
                ),
            );
        }
    }

    if next.is_view() && !next.fields.iter().any(|f| f.name == "id") {
        errors.insert(
            "viewQuery".into(),
            FieldError::new(
                codes::INVALID_VIEW_QUERY,
                "Invalid view query - the query must include an \"id\" column.",
            ),
        );
    }

    // MFA needs a second factor to actually add anything: with only one
    // enabled auth method there is nothing to challenge for beyond the
    // first, so PocketBase rejects enabling it outright rather than
    // silently accepting a no-op MFA config.
    if next.is_auth() && next.auth.mfa.enabled {
        let enabled_methods = [
            next.auth.password_auth.enabled,
            next.auth.otp.enabled,
            next.auth.oauth2.enabled,
        ]
        .into_iter()
        .filter(|&e| e)
        .count();
        if enabled_methods < 2 {
            let mut data = Map::new();
            data.insert(
                "mfa".into(),
                json!({
                    "enabled": {
                        "code": "validation_mfa_not_enough_auths",
                        "message": "MFA requires at least two enabled auth methods.",
                    }
                }),
            );
            return Err(ApiError::nested_validation(message, data));
        }
    }

    if !errors.is_empty() {
        return Err(ApiError(AppError::validation(message, errors)));
    }

    // Rules are compiled against the collection being saved, so a rule
    // referring to a field this very request adds is accepted.
    validate_rules(app, next, message)?;
    validate_indexes(next, message)?;
    Ok(())
}

fn validate_rules(app: &App, next: &Collection, message: &str) -> ApiResult<()> {
    let ctx = cratebase_db::context::RequestContext::default();
    let root = Arc::new(next.clone());
    let resolver = cratebase_db::context::CollectionResolver::new(
        root,
        &app.db().collections,
        &ctx,
        app.db().dialect(),
    );
    let mut errors: std::collections::BTreeMap<String, FieldError> = Default::default();
    for (key, rule) in [
        ("listRule", &next.list_rule),
        ("viewRule", &next.view_rule),
        ("createRule", &next.create_rule),
        ("updateRule", &next.update_rule),
        ("deleteRule", &next.delete_rule),
    ] {
        let Some(expr) = rule.as_deref().map(str::trim).filter(|e| !e.is_empty()) else {
            continue;
        };
        if cratebase_filter::parse_and_compile(expr, &resolver, 0).is_err() {
            errors.insert(
                key.into(),
                FieldError::new(codes::INVALID_RULE, "Invalid rule expression."),
            );
        }
    }
    if next.is_auth() {
        for (key, rule) in [
            ("authRule", &next.auth.auth_rule),
            ("manageRule", &next.auth.manage_rule),
        ] {
            let Some(expr) = rule.as_deref().map(str::trim).filter(|e| !e.is_empty()) else {
                continue;
            };
            if cratebase_filter::parse_and_compile(expr, &resolver, 0).is_err() {
                errors.insert(
                    key.into(),
                    FieldError::new(codes::INVALID_RULE, "Invalid rule expression."),
                );
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ApiError(AppError::validation(message, errors)))
    }
}

/// Index statements have to parse *and* only mention real columns. The
/// column check is done here rather than left to the database so the
/// error can name the offending index by position, which is how
/// PocketBase reports it (KNOWN_DIVERGENCES §7).
fn validate_indexes(next: &Collection, message: &str) -> ApiResult<()> {
    for (position, statement) in next.indexes.iter().enumerate() {
        let Some(def) = schema::parse_index(statement) else {
            return Err(indexes_error(
                message,
                position,
                codes::INVALID_INDEX,
                "Invalid CREATE INDEX expression.",
            ));
        };
        if let Some(missing) = unknown_column(next, &def.columns_raw) {
            return Err(indexes_error(
                message,
                position,
                codes::INVALID_INDEX,
                &format!(
                    "Failed to create index {} - SQL logic error: no such column: {missing} (1).",
                    def.name
                ),
            ));
        }
    }
    Ok(())
}

/// The first bare identifier in an index column list that is not a field.
/// Function names (a word followed by `(`) and SQL keywords are ignored.
fn unknown_column(collection: &Collection, columns: &str) -> Option<String> {
    const KEYWORDS: &[&str] = &[
        "ASC", "DESC", "COLLATE", "NOCASE", "BINARY", "RTRIM", "NULLS", "FIRST", "LAST",
    ];
    let chars: Vec<char> = columns.chars().collect();
    let mut i = 0;
    let mut skip_next = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '`' || c == '[' {
            let close = match c {
                '[' => ']',
                other => other,
            };
            let start = i + 1;
            i += 1;
            while i < chars.len() && chars[i] != close {
                i += 1;
            }
            let word: String = chars[start..i.min(chars.len())].iter().collect();
            i += 1;
            if !skip_next && !collection.has_field(&word) {
                return Some(word);
            }
            skip_next = false;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let is_call = chars[i..].iter().find(|ch| !ch.is_whitespace()) == Some(&'(');
            let upper = word.to_ascii_uppercase();
            if !skip_next
                && !is_call
                && !KEYWORDS.contains(&upper.as_str())
                && !collection.has_field(&word)
            {
                return Some(word);
            }
            skip_next = upper == "COLLATE";
            continue;
        }
        i += 1;
    }
    None
}

/// `data.indexes["<position>"] = {code, message}`.
fn indexes_error(message: &str, position: usize, code: &str, detail: &str) -> ApiError {
    let mut indexes = Map::new();
    indexes.insert(
        position.to_string(),
        serde_json::to_value(FieldError::new(code, detail)).unwrap_or(Value::Null),
    );
    let mut data = Map::new();
    data.insert("indexes".into(), Value::Object(indexes));
    ApiError::nested_validation(message, data)
}

// ------------------------------------------------------------- view fields

/// Derive a view collection's fields from its `viewQuery`.
///
/// PocketBase parses the SELECT list and clones the origin collection's
/// field definitions, giving each clone a synthetic `_clone_XXXX` id
/// (KNOWN_DIVERGENCES §3). This is the same idea, done textually: the
/// select list gives the column names and aliases, and the first table in
/// the `FROM` clause supplies the types.
pub(crate) fn view_fields(app: &App, next: &Collection, message: &str) -> ApiResult<Vec<Field>> {
    let query = next.view_query.trim();
    let (select, from) = split_select(query).ok_or_else(|| {
        ApiError(AppError::validation(
            message,
            [(
                "viewQuery".to_string(),
                FieldError::new(codes::INVALID_VIEW_QUERY, "Invalid view query."),
            )],
        ))
    })?;
    let origin = from
        .split_whitespace()
        .next()
        .map(|t| t.trim_matches(|c| c == '`' || c == '"' || c == '\'' || c == ';'))
        .and_then(|t| app.db().collections.get(t));

    let mut fields = Vec::new();
    for item in split_columns(select) {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let name = column_alias(item);
        if name == "*" {
            if let Some(origin) = &origin {
                for field in &origin.fields {
                    fields.push(clone_field(field, &field.name));
                }
            }
            continue;
        }
        let source = name_after_dot(item);
        let field = origin
            .as_ref()
            .and_then(|c| c.field(&source).or_else(|| c.field(&name)))
            .cloned()
            .unwrap_or_else(|| Field::new(&name, FieldKind::default_for(FieldType::Text)));
        fields.push(clone_field(&field, &name));
    }
    if let Some(position) = fields.iter().position(|f| f.name == "id") {
        let mut id = Field::id_field();
        id.id = fields[position].id.clone();
        fields[position] = id;
        fields.swap(0, position);
    }
    Ok(fields)
}

fn clone_field(source: &Field, name: &str) -> Field {
    let mut field = source.clone();
    field.name = name.to_string();
    field.system = false;
    field.required = false;
    field.id = format!(
        "_clone_{}",
        cratebase_core::ids::random_string(4, b"abcdefghijklmnopqrstuvwxyz0123456789")
    );
    field
}

/// Split `SELECT <list> FROM <rest>` at the top-level `FROM`.
fn split_select(query: &str) -> Option<(&str, &str)> {
    let upper = query.to_ascii_uppercase();
    let start = upper.find("SELECT")? + "SELECT".len();
    let mut depth = 0usize;
    let bytes = upper.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'F' if depth == 0 && upper[i..].starts_with("FROM") => {
                let before_ok = bytes
                    .get(i.wrapping_sub(1))
                    .is_some_and(|b| b.is_ascii_whitespace());
                let after_ok = bytes
                    .get(i + 4)
                    .is_none_or(|b| b.is_ascii_whitespace() || *b == b'(');
                if before_ok && after_ok {
                    return Some((&query[start..i], query[i + 4..].trim()));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split a select list on commas that are not inside parentheses.
fn split_columns(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for c in list.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    out.push(current);
    out
}

/// The name a select item is exposed under: its `AS` alias, or the last
/// dotted segment of the expression.
fn column_alias(item: &str) -> String {
    let upper = item.to_ascii_uppercase();
    if let Some(position) = upper.rfind(" AS ") {
        return unquote(item[position + 4..].trim());
    }
    let last = item.split_whitespace().last().unwrap_or(item);
    if last != item && !last.ends_with(')') {
        return unquote(last);
    }
    name_after_dot(item)
}

fn name_after_dot(item: &str) -> String {
    let base = item.split_whitespace().next().unwrap_or(item);
    unquote(base.rsplit('.').next().unwrap_or(base))
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c| c == '`' || c == '"' || c == '\'' || c == '[' || c == ']')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_field_names_keep_the_last_definition() {
        let mut c = Collection::new("posts", CollectionType::Base);
        c.fields = vec![
            Field::new("a", FieldKind::default_for(FieldType::Text)),
            Field::new("a", FieldKind::default_for(FieldType::Number)),
        ];
        dedupe_fields(&mut c);
        assert_eq!(c.fields.len(), 1);
        assert_eq!(c.fields[0].field_type(), FieldType::Number);
    }

    #[test]
    fn index_columns_are_checked_against_the_schema() {
        let mut c = Collection::new("posts", CollectionType::Base);
        c.fields.insert(
            1,
            Field::new("title", FieldKind::default_for(FieldType::Text)),
        );
        assert_eq!(unknown_column(&c, "`title`"), None);
        assert_eq!(unknown_column(&c, "lower(title) DESC"), None);
        assert_eq!(unknown_column(&c, "`missing`"), Some("missing".into()));
        assert_eq!(unknown_column(&c, "title COLLATE NOCASE"), None);
    }

    #[test]
    fn select_lists_are_split_into_named_columns() {
        let (select, from) = split_select("SELECT id, title, n FROM posts").unwrap();
        assert_eq!(from, "posts");
        let names: Vec<String> = split_columns(select)
            .iter()
            .map(|c| column_alias(c))
            .collect();
        assert_eq!(names, ["id", "title", "n"]);

        let (select, from) =
            split_select("select p.id as id, upper(p.title) AS heading from posts p").unwrap();
        assert!(from.starts_with("posts"));
        let names: Vec<String> = split_columns(select)
            .iter()
            .map(|c| column_alias(c))
            .collect();
        assert_eq!(names, ["id", "heading"]);
    }

    #[test]
    fn auth_scaffolds_carry_the_two_default_indexes() {
        let indexes = default_auth_indexes("users", "pbc_1");
        assert_eq!(indexes.len(), 2);
        assert!(indexes[0].contains("tokenKey"));
        assert!(indexes[1].contains("email"));
    }
}
