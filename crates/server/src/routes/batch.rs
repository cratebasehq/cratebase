use std::collections::HashMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use cratebase_core::field::FieldType;
use cratebase_core::{new_id, AppError, Collection};
use cratebase_db::records::{self, RecordTx};
use cratebase_db::resolver::{
    evaluate_create_rule_tx, evaluate_rule, AuthContext, RequestContext, RuleOutcome,
};
use cratebase_db::validate;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::extract::CurrentAuth;
use crate::helpers::{file_key, load_collection_tx};
use crate::http_error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/batch", post(batch))
}

/// Upper bound on sub-requests per batch. PocketBase documents no hard cap,
/// but an unbounded batch turns one HTTP request into an unbounded amount of
/// transactional work on a single connection — 50 mirrors the page-size caps
/// used elsewhere in the API (`ListQuery::per_page` etc.) as a sane ceiling.
const MAX_BATCH_REQUESTS: usize = 50;

#[derive(Deserialize)]
struct BatchSubRequest {
    method: String,
    url: String,
    #[serde(default)]
    body: Option<Value>,
    /// Accepted for shape-compatibility with PocketBase's batch payload but
    /// intentionally unused: PocketBase's batch API has no per-sub-request
    /// auth override either — every sub-request runs under the single
    /// `Authorization` header on the outer `/api/batch` request.
    #[serde(default)]
    #[allow(dead_code)]
    headers: Option<Map<String, Value>>,
}

#[derive(Deserialize)]
struct BatchRequest {
    requests: Vec<BatchSubRequest>,
}

fn forbidden() -> ApiError {
    ApiError(AppError::Forbidden(
        "you are not allowed to perform this action".into(),
    ))
}

fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(AppError::BadRequest(msg.into()))
}

/// A write made durable only once the whole batch commits: realtime
/// publish, and (for deletes) best-effort storage cleanup of the deleted
/// record's file fields. Deferred until after `tx.commit()` succeeds so a
/// later sub-request's rollback can't leave a realtime event or a deleted
/// file referring to a database write that never actually happened.
enum Effect {
    Publish {
        collection: Collection,
        event: &'static str,
        record: Value,
    },
    RecordDeleted {
        collection: Collection,
        record_id: String,
        record: Value,
        files: Vec<(String, Vec<String>)>,
    },
}

impl Effect {
    async fn apply(self, app: &AppState) {
        match self {
            Effect::Publish {
                collection,
                event,
                record,
            } => {
                app.realtime.publish(&app.db, &collection, event, &record).await;
            }
            Effect::RecordDeleted {
                collection,
                record_id,
                record,
                files,
            } => {
                for (_, names) in files {
                    for name in names {
                        let key = file_key(&collection.id, &record_id, &name);
                        if let Err(e) = app.storage.delete(&key).await {
                            tracing::warn!(key, error = %e, "failed to clean up orphaned file");
                        }
                    }
                }
                app.realtime.publish(&app.db, &collection, "delete", &record).await;
            }
        }
    }
}

/// Mirrors `records::file_field_values` in `routes/records.rs` (private to
/// that module) — collects the stored filenames a deleted record's file
/// fields referenced, for best-effort cleanup after commit.
fn file_field_values(collection: &Collection, record: &Value) -> Vec<(String, Vec<String>)> {
    collection
        .schema
        .iter()
        .filter(|f| f.field_type == FieldType::File)
        .filter_map(|f| {
            let value = record.get(&f.name)?;
            let names: Vec<String> = match value {
                Value::String(s) => vec![s.clone()],
                Value::Array(items) => items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect(),
                _ => return None,
            };
            Some((f.name.clone(), names))
        })
        .collect()
}

/// Splits a sub-request's `url` into `(collection, record_id)`. Only the
/// two shapes the record CRUD routes expose are recognized; anything else
/// (files, auth, collection management, ...) is out of scope for batch,
/// same as PocketBase.
fn parse_url(url: &str) -> ApiResult<(String, Option<String>)> {
    let path = url.split('?').next().unwrap_or(url);
    let path = path.strip_prefix("/api").unwrap_or(path);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        ["collections", name, "records"] => Ok(((*name).to_string(), None)),
        ["collections", name, "records", id] => Ok(((*name).to_string(), Some((*id).to_string()))),
        _ => Err(bad_request(format!(
            "unsupported batch request url '{url}': only /api/collections/{{collection}}/records[/{{id}}] is supported"
        ))),
    }
}

fn body_object(body: Option<Value>) -> ApiResult<Map<String, Value>> {
    match body {
        None => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map),
        Some(_) => Err(bad_request("batch sub-request body must be a JSON object")),
    }
}

async fn create_in_tx(
    app: &AppState,
    auth: &Option<AuthContext>,
    tx: &mut RecordTx,
    collection: Collection,
    body: Option<Value>,
) -> ApiResult<(u16, Value, Option<Effect>)> {
    let fields = body_object(body)?;

    let ctx = RequestContext {
        auth: auth.clone(),
        data: Some(fields.clone()),
    };
    let allowed =
        evaluate_create_rule_tx(tx, app.db.backend, &collection.create_rule, &collection, &ctx)
            .await?;
    if !allowed {
        return Err(forbidden());
    }

    let fields = crate::auth_fields::prepare_auth_create(&collection, fields)?;
    let normalized =
        validate::validate_and_normalize_tx(tx, app.db.backend, &collection, &fields, false)
            .await?;

    let id = new_id();
    let record =
        records::create_record_with_id_tx(tx, app.db.backend, &collection, id, fields, normalized).await?;

    let effect = Effect::Publish {
        collection,
        event: "create",
        record: record.clone(),
    };
    Ok((200, record, Some(effect)))
}

/// Relation dot-notation in `updateRule` isn't supported mid-batch: it
/// needs a target-collection lookup, which — like every other read in
/// this file — must go through `tx` rather than the pool to avoid
/// re-acquiring a second connection while `tx` holds the only one (see
/// `crates/db/src/pool.rs`). Passing an empty `related` map here means
/// such a rule just resolves those idents as `UnknownField`, same as it
/// already does for `createRule` above.
async fn update_in_tx(
    app: &AppState,
    auth: &Option<AuthContext>,
    tx: &mut RecordTx,
    collection: Collection,
    id: String,
    body: Option<Value>,
) -> ApiResult<(u16, Value, Option<Effect>)> {
    let ctx_check = RequestContext {
        auth: auth.clone(),
        data: None,
    };
    let outcome = evaluate_rule(
        &collection.update_rule,
        &collection,
        app.db.backend,
        &ctx_check,
        0,
        &HashMap::new(),
    )?;
    let rule_filter = match outcome {
        RuleOutcome::DenyAll => return Err(forbidden()),
        RuleOutcome::AllowAll => None,
        RuleOutcome::Filtered(f) => Some(f),
    };
    // Reads through `tx`, the same connection every write in this batch
    // lands on, so a record created earlier in this same batch (still
    // uncommitted) *is* visible to a later update/delete sub-request.
    records::get_record_tx(tx, app.db.backend, &collection, &id, rule_filter).await?;

    let fields = body_object(body)?;
    let fields = crate::auth_fields::prepare_auth_update(&collection, fields)?;
    let normalized =
        validate::validate_and_normalize_tx(tx, app.db.backend, &collection, &fields, true)
            .await?;

    let record =
        records::update_record_tx(tx, app.db.backend, &collection, &id, fields, normalized).await?;

    let effect = Effect::Publish {
        collection,
        event: "update",
        record: record.clone(),
    };
    Ok((200, record, Some(effect)))
}

async fn delete_in_tx(
    app: &AppState,
    auth: &Option<AuthContext>,
    tx: &mut RecordTx,
    collection: Collection,
    id: String,
) -> ApiResult<(u16, Value, Option<Effect>)> {
    let ctx = RequestContext {
        auth: auth.clone(),
        data: None,
    };
    let outcome = evaluate_rule(
        &collection.delete_rule,
        &collection,
        app.db.backend,
        &ctx,
        0,
        &HashMap::new(),
    )?;
    let rule_filter = match outcome {
        RuleOutcome::DenyAll => return Err(forbidden()),
        RuleOutcome::AllowAll => None,
        RuleOutcome::Filtered(f) => Some(f),
    };
    let record = records::get_record_tx(tx, app.db.backend, &collection, &id, rule_filter).await?;
    records::delete_record_tx(tx, app.db.backend, &collection, &id).await?;

    let files = file_field_values(&collection, &record);
    let effect = Effect::RecordDeleted {
        collection,
        record_id: id,
        record,
        files,
    };
    Ok((204, Value::Null, Some(effect)))
}

async fn execute_sub_request(
    app: &AppState,
    auth: &Option<AuthContext>,
    tx: &mut RecordTx,
    sub: &BatchSubRequest,
) -> ApiResult<(u16, Value, Option<Effect>)> {
    let (collection_name, id) = parse_url(&sub.url)?;
    let collection = load_collection_tx(tx, &collection_name).await?;
    let method = sub.method.to_uppercase();

    match (method.as_str(), id) {
        ("POST", None) => create_in_tx(app, auth, tx, collection, sub.body.clone()).await,
        ("PATCH", Some(id)) => update_in_tx(app, auth, tx, collection, id, sub.body.clone()).await,
        ("DELETE", Some(id)) => delete_in_tx(app, auth, tx, collection, id).await,
        ("POST", Some(_)) => Err(bad_request(
            "POST batch requests must target a collection's records list url (no trailing id)",
        )),
        (m, None) if m == "PATCH" || m == "DELETE" => Err(bad_request(format!(
            "{m} batch requests must target a specific record url"
        ))),
        (m, _) => Err(bad_request(format!(
            "unsupported batch method '{m}': only POST, PATCH, DELETE are supported"
        ))),
    }
}

/// Runs every sub-request against the same collection/record handler logic
/// (rule evaluation + `cratebase-db` create/update/delete) the individual
/// record routes use, but confined to one SQL transaction: any sub-request
/// failing rolls back every write already made by earlier sub-requests in
/// the batch, matching PocketBase's "all requests succeed or none do"
/// batch semantics. All sub-requests run under the single `Authorization`
/// header on this outer request — batch has no per-sub-request auth
/// override, same as PocketBase.
///
/// Response shape diverges from PocketBase's documented batch API (which
/// returns a flat array where each entry carries its own independent
/// status/body, since PocketBase's batch endpoint is not itself
/// transactional). Since this implementation *is* atomic, "one entry per
/// sub-request" would be misleading on failure — the failing entry would
/// look identical to how PocketBase renders a genuinely-applied error even
/// though every write here was undone. Instead:
/// - success: `200 { "results": [{ "status", "body" }, ...] }` in request order.
/// - failure: the failing sub-request's own HTTP status, with
///   `{ "failedIndex": <n>, "error": { "code", "message", "data" } }`
///   (the same `ErrorBody` shape every other endpoint returns).
async fn batch(
    State(app): State<AppState>,
    CurrentAuth(auth): CurrentAuth,
    Json(payload): Json<BatchRequest>,
) -> ApiResult<Response> {
    if payload.requests.is_empty() {
        return Err(bad_request("requests must contain at least one entry"));
    }
    if payload.requests.len() > MAX_BATCH_REQUESTS {
        return Err(bad_request(format!(
            "batch accepts at most {MAX_BATCH_REQUESTS} sub-requests, got {}",
            payload.requests.len()
        )));
    }

    let mut tx = app
        .db
        .pool
        .begin()
        .await
        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    let mut results = Vec::with_capacity(payload.requests.len());
    let mut effects: Vec<Effect> = Vec::new();

    for (index, sub) in payload.requests.iter().enumerate() {
        match execute_sub_request(&app, &auth, &mut tx, sub).await {
            Ok((status, body, effect)) => {
                results.push(json!({ "status": status, "body": body }));
                effects.extend(effect);
            }
            Err(err) => {
                // `tx` is dropped here without `commit()`, which rolls
                // back every write made by earlier sub-requests in this
                // batch (sqlx rolls back on drop).
                let error_body = err.0.body();
                let status =
                    StatusCode::from_u16(error_body.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                return Ok((
                    status,
                    Json(json!({ "failedIndex": index, "error": error_body })),
                )
                    .into_response());
            }
        }
    }

    tx.commit()
        .await
        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    for effect in effects {
        effect.apply(&app).await;
    }

    Ok((StatusCode::OK, Json(json!({ "results": results }))).into_response())
}
