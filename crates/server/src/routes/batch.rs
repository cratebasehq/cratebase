//! `POST /api/batch` — one HTTP round trip, one SQL transaction, several
//! record writes.
//!
//! # Wire format
//!
//! The official SDK's `createBatch()` always POSTs
//! `multipart/form-data`, never plain JSON, because that is the only
//! encoding that can carry both the request list and any files a
//! sub-request uploads in one body:
//!
//! * `@jsonPayload` — `{"requests": [{method, url, headers, body}, ...]}`,
//!   one entry per sub-request. `body` never contains a `File`; the SDK
//!   strips those out before serializing it.
//! * `requests.<index>.<field>` — one multipart part per uploaded file,
//!   named by the sub-request's position in the array and the schema
//!   field it targets. A `maxSelect > 1` file field repeats the same
//!   part name once per file, exactly like the single-record multipart
//!   path ([`crate::routes::common::parse_multipart`]).
//!
//! A hand-rolled `application/json` body (no files) is accepted too —
//! just `{"requests": [...]}` — since nothing about the shape requires
//! multipart when there is nothing to upload.
//!
//! Each sub-request's `url` is itself one of the record endpoints —
//! `POST .../records` (create), `PATCH .../records/{id}` (update), `PUT
//! .../records` (upsert, id read from the body) or `DELETE
//! .../records/{id}` — so this module re-derives the same method/url
//! dispatch the router already does, rather than reusing axum's routing:
//! there is no [`axum::extract::Request`] per sub-request, only a parsed
//! method/url/body triple.
//!
//! # One transaction, not N
//!
//! Every sub-request runs against the *same* [`TxApp`], not through
//! [`crate::routes::records`]'s handlers (each of which opens — and
//! commits — its own transaction via `App::run_scoped`). This module
//! calls straight into the shared pieces of the write path that already
//! accept an open `TxApp` — `write_record`, `delete_in_tx`,
//! `apply_auth_fields` — and skips the outer `*Request` hook
//! (`onRecordCreateRequest` and siblings) that normally wraps those in a
//! transaction of their own; a batch fires `onBatchRequest` once instead,
//! for the same reason.
//!
//! # Object storage is not transactional
//!
//! A file lands in the object store the moment its owning sub-request is
//! processed — the record row needs the generated name before it can be
//! inserted — which means a storage write has already happened by the
//! time a *later* sub-request fails and the whole SQL transaction rolls
//! back. Every write this batch made is tracked and, on rollback, undone;
//! every removal (a delete's files, an update's replaced ones) is
//! deferred until the transaction actually commits, so a rollback never
//! deletes a file the database still points at.
//!
//! # Two response shapes
//!
//! Success is `200` with one `{status, body}` per sub-request, in order
//! — `204`/`body: null` for a delete, matching the single-record
//! endpoints exactly. A failure is `400 "Batch transaction failed."` with
//! the *failing* sub-request's own status/message/data nested at
//! `data.requests["<index>"].response`; every request before it is
//! silently discarded along with the transaction, not echoed back.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{FromRequest, Multipart, Request, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use cratebase_core::{codes, AppError, Collection, Record};
use cratebase_db::context::{CollectionResolver, RequestContext};
use cratebase_db::{records, rules, validate, DbError, UploadMeta};
use cratebase_storage::Storage;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::app::{App, TxApp};
use crate::events::BatchRequestEvent;
use crate::extract::RequestInfo;
use crate::http_error::{rule_errors, ApiError, ApiResult};
use crate::realtime::{self, RecordAction};
use crate::routes::common::{self, StagedUpload};
use crate::routes::records::{self as record_route, Write};

pub fn router() -> Router<App> {
    Router::new().route("/batch", post(batch))
}

// --------------------------------------------------------------- payload

/// One `{method, url, headers?, body?}` entry as the SDK's
/// `createBatch()` serializes it. `body` is arbitrary JSON — files never
/// appear here, only in the sibling `requests.<index>.<field>` multipart
/// parts.
#[derive(Debug, Default, Deserialize)]
struct SubRequestJson {
    #[serde(default)]
    method: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    headers: Map<String, Value>,
    #[serde(default)]
    body: Value,
}

#[derive(Debug, Default, Deserialize)]
struct JsonPayload {
    #[serde(default)]
    requests: Vec<SubRequestJson>,
}

/// A parsed batch request, files still grouped by sub-request index and
/// field name (repeated parts under one field name are a `maxSelect > 1`
/// upload and become an array).
struct ParsedBatch {
    requests: Vec<SubRequestJson>,
    files: HashMap<usize, HashMap<String, Vec<StagedUpload>>>,
}

/// One assembled sub-request: JSON body and files merged the way
/// [`common::parse_multipart`] merges them for a single record — file
/// parts win, keyed under their (possibly modifier-suffixed) field name.
struct BatchItem {
    method: String,
    url: String,
    headers: Map<String, Value>,
    body: Map<String, Value>,
    uploads: Vec<StagedUpload>,
}

/// JSON or multipart, chosen by `Content-Type` — the same dispatch
/// [`crate::routes::records::read_body`] makes for a single record.
async fn parse_batch_body(app: &App, request: Request) -> ApiResult<ParsedBatch> {
    let is_multipart = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("multipart/form-data"));

    if !is_multipart {
        let bytes = axum::body::Bytes::from_request(request, app)
            .await
            .map_err(|_| ApiError(AppError::bad_request("")))?;
        let payload: JsonPayload = if bytes.iter().all(u8::is_ascii_whitespace) {
            JsonPayload::default()
        } else {
            serde_json::from_slice(&bytes).map_err(|_| ApiError(AppError::bad_request("")))?
        };
        return Ok(ParsedBatch {
            requests: payload.requests,
            files: HashMap::new(),
        });
    }

    let mut multipart = Multipart::from_request(request, app)
        .await
        .map_err(|_| ApiError(AppError::bad_request("")))?;
    let mut payload: Option<JsonPayload> = None;
    let mut files: HashMap<usize, HashMap<String, Vec<StagedUpload>>> = HashMap::new();

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(_) => return Err(ApiError(AppError::bad_request(""))),
        };
        let name = field.name().unwrap_or_default().to_string();
        if name.is_empty() {
            continue;
        }

        if name == common::JSON_PAYLOAD {
            let text = field
                .text()
                .await
                .map_err(|_| ApiError(AppError::bad_request("")))?;
            payload =
                Some(serde_json::from_str(&text).map_err(|_| ApiError(AppError::bad_request("")))?);
            continue;
        }

        let Some((idx, key)) = parse_file_field_name(&name) else {
            continue;
        };
        let original = field.file_name().unwrap_or("file").to_string();
        let content_type = field.content_type().map(str::to_string);
        let base = common::strip_modifier(&key).to_string();
        let staged = common::stage_field(field, &base, &original, content_type).await?;
        files
            .entry(idx)
            .or_default()
            .entry(key)
            .or_default()
            .push(staged);
    }

    Ok(ParsedBatch {
        requests: payload
            .ok_or_else(|| ApiError(AppError::bad_request("")))?
            .requests,
        files,
    })
}

/// `requests.<index>.<field>` → `(index, field)`.
fn parse_file_field_name(name: &str) -> Option<(usize, String)> {
    let rest = name.strip_prefix("requests.")?;
    let (idx, key) = rest.split_once('.')?;
    if key.is_empty() {
        return None;
    }
    Some((idx.parse().ok()?, key.to_string()))
}

/// Merge each sub-request's JSON body with its staged files, in order.
fn assemble(parsed: ParsedBatch) -> Vec<BatchItem> {
    let ParsedBatch {
        requests,
        mut files,
    } = parsed;
    requests
        .into_iter()
        .enumerate()
        .map(|(idx, req)| {
            let mut body = match req.body {
                Value::Object(map) => map,
                _ => Map::new(),
            };
            let mut uploads = Vec::new();
            if let Some(field_map) = files.remove(&idx) {
                for (key, staged) in field_map {
                    let value = if staged.len() == 1 {
                        Value::String(staged[0].name.clone())
                    } else {
                        Value::Array(
                            staged
                                .iter()
                                .map(|s| Value::String(s.name.clone()))
                                .collect(),
                        )
                    };
                    body.insert(key, value);
                    uploads.extend(staged);
                }
            }
            BatchItem {
                method: req.method,
                url: req.url,
                headers: req.headers,
                body,
                uploads,
            }
        })
        .collect()
}

// ----------------------------------------------------------------- target

/// The record endpoint a sub-request's `url` names.
struct Target {
    collection: String,
    id: Option<String>,
    path: String,
    query: Map<String, Value>,
}

fn parse_target(url: &str) -> ApiResult<Target> {
    let (path, query_str) = url.split_once('?').unwrap_or((url, ""));
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        ["api", "collections", name, "records"] => Ok(Target {
            collection: percent_decode(name),
            id: None,
            path: path.to_string(),
            query: query_map(query_str),
        }),
        ["api", "collections", name, "records", id] => Ok(Target {
            collection: percent_decode(name),
            id: Some(percent_decode(id)),
            path: path.to_string(),
            query: query_map(query_str),
        }),
        _ => Err(ApiError(AppError::bad_request(""))),
    }
}

enum Op {
    Create,
    Update(String),
    Upsert,
    Delete(String),
}

fn resolve_op(method: &str, target: &Target) -> ApiResult<Op> {
    match (method.to_ascii_uppercase().as_str(), target.id.clone()) {
        ("POST", None) => Ok(Op::Create),
        ("PATCH", Some(id)) => Ok(Op::Update(id)),
        ("PUT", None) => Ok(Op::Upsert),
        ("DELETE", Some(id)) => Ok(Op::Delete(id)),
        _ => Err(ApiError(AppError::bad_request(""))),
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_map(raw: &str) -> Map<String, Value> {
    let mut out = Map::new();
    for pair in raw.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(percent_decode(key), Value::String(percent_decode(value)));
    }
    out
}

fn normalize_header_key(key: &str) -> String {
    key.to_ascii_lowercase().replace('-', "_")
}

/// The `@request.*` view of one sub-request: its own method/body/query,
/// the outer request's headers overridden by its own, and the batch
/// caller's auth (a batch has no way to authenticate differently
/// per-request — it is one `Authorization` header for the whole thing).
fn sub_request_info(outer: &RequestInfo, item: &BatchItem, target: &Target) -> RequestInfo {
    let mut headers = outer.headers.clone();
    for (k, v) in &item.headers {
        headers.insert(normalize_header_key(k), v.clone());
    }
    RequestInfo {
        method: item.method.to_ascii_uppercase(),
        path: target.path.clone(),
        query: target.query.clone(),
        headers,
        body: item.body.clone(),
        context: "batch".to_string(),
        auth: outer.auth.clone(),
    }
}

// -------------------------------------------------------------- processing

/// What one successful sub-request leaves behind, kept apart from the
/// database write itself because none of it is safe to act on until the
/// whole batch's transaction actually commits (see the module doc).
struct ProcessOutcome {
    entry: Value,
    /// Object-store keys written for this request; removed if the batch
    /// rolls back.
    new_storage_keys: Vec<String>,
    /// `(collectionId, recordId, fileName)` an update replaced; removed
    /// once the batch commits, never on rollback (the old row is still
    /// live if the transaction aborts).
    post_commit_removals: Vec<(String, String, String)>,
    /// `(collectionId, recordId)` a delete removed; swept once the batch
    /// commits.
    post_commit_deletes: Vec<(String, String)>,
    realtime_event: Option<(Arc<Collection>, RecordAction, Record)>,
}

/// Bundles what every per-item write function needs but `delete_item`
/// doesn't, so adding a field here never means touching every call site's
/// argument list — this is what pushed `create_item`/`update_item` over
/// clippy's argument-count lint before the bundling.
struct ItemCtx<'a> {
    tx: &'a TxApp,
    storage: &'a Storage,
    collection: &'a Arc<Collection>,
    info: &'a RequestInfo,
    ctx: &'a RequestContext,
}

async fn process_item(
    tx: &TxApp,
    outer_info: &RequestInfo,
    item: BatchItem,
) -> Result<ProcessOutcome, ApiError> {
    let target = parse_target(&item.url)?;
    let op = resolve_op(&item.method, &target)?;
    let collection = common::collection_of(tx, &target.collection)?;
    if collection.is_view() {
        return Err(ApiError(AppError::bad_request(
            "Unsupported collection type.",
        )));
    }
    let info = sub_request_info(outer_info, &item, &target);
    let ctx = info.to_context();
    let storage = tx.storage();
    let ictx = ItemCtx {
        tx,
        storage: &storage,
        collection: &collection,
        info: &info,
        ctx: &ctx,
    };

    match op {
        Op::Create => create_item(&ictx, item.body, item.uploads, None).await,
        Op::Update(id) => update_item(&ictx, id, item.body, item.uploads).await,
        Op::Upsert => upsert_item(&ictx, item.body, item.uploads).await,
        Op::Delete(id) => delete_item(tx, &collection, &ctx, id).await,
    }
}

/// Mirrors `records::create_record`, minus the `*Request` hook and the
/// transaction it would open — this runs inside the batch's own.
/// `explicit_id` is how [`upsert_item`] forces the id the client asked
/// for when there is no existing row to update.
async fn create_item(
    ictx: &ItemCtx<'_>,
    body: Map<String, Value>,
    uploads: Vec<StagedUpload>,
    explicit_id: Option<String>,
) -> Result<ProcessOutcome, ApiError> {
    let ItemCtx {
        tx,
        storage,
        collection,
        info,
        ctx,
    } = *ictx;
    if rules::is_superuser_only(&collection.create_rule, ctx) {
        return Err(ApiError(rule_errors::superusers_only()));
    }
    {
        let resolver = CollectionResolver::new(
            collection.clone(),
            &tx.db().collections,
            ctx,
            tx.db().dialect(),
        );
        if !rules::check_create_rule(tx, &resolver, &collection.create_rule)
            .await
            .map_err(|e| ApiError(record_route::read_error(e)))?
        {
            return Err(ApiError(rule_errors::create_denied()));
        }
    }

    let mut input = body.clone();
    common::apply_number_modifiers(None, &mut input, collection);
    validate::apply_modifiers(None, &mut input, collection);
    let mut record = records::from_body(collection.clone(), &input);
    if let Some(id) = explicit_id {
        record.set_id(id);
    } else if record.id().is_empty() {
        record.set_id(cratebase_core::record_id());
    }
    record_route::apply_auth_fields(
        collection,
        &body,
        &mut record,
        None,
        ctx.is_superuser(),
        record_route::CREATE_FAILED,
    )
    .await?;

    let stored = common::store_uploads(storage, &collection.id, record.id(), &uploads).await?;
    let upload_meta: Vec<UploadMeta> = uploads.iter().map(StagedUpload::meta).collect();

    let saved = record_route::write_record(
        tx.clone(),
        collection.clone(),
        record,
        None,
        Write::Create,
        upload_meta,
    )
    .await;
    let saved = match saved {
        Ok(saved) => saved,
        Err(e) => {
            common::remove_keys(storage, &stored).await;
            return Err(common::relabel_upload_errors(ApiError(e), &uploads));
        }
    };

    let value = serialize_record(
        tx.app(),
        collection,
        info,
        saved.clone(),
        ctx.is_superuser(),
    )
    .await?;
    Ok(ProcessOutcome {
        entry: serde_json::json!({ "status": 200, "body": value }),
        new_storage_keys: stored,
        post_commit_removals: Vec::new(),
        post_commit_deletes: Vec::new(),
        realtime_event: Some((collection.clone(), RecordAction::Create, saved)),
    })
}

/// Mirrors `records::update_record`.
async fn update_item(
    ictx: &ItemCtx<'_>,
    id: String,
    body: Map<String, Value>,
    uploads: Vec<StagedUpload>,
) -> Result<ProcessOutcome, ApiError> {
    let ItemCtx {
        tx,
        storage,
        collection,
        info,
        ctx,
    } = *ictx;
    if rules::is_superuser_only(&collection.update_rule, ctx) {
        return Err(ApiError(rule_errors::superusers_only()));
    }
    let previous = records::find_by_id_raw(tx, collection, &id)
        .await
        .map_err(|e| ApiError(record_route::read_error(e)))?;
    if !common::record_matches_rule(
        tx,
        &tx.db().collections,
        ctx,
        collection,
        &collection.update_rule,
        &id,
    )
    .await
    .map_err(|e| ApiError(e.into()))?
    {
        return Err(ApiError(rule_errors::hidden_record()));
    }
    let manage = common::has_manage_access(tx, &tx.db().collections, ctx, collection, &id)
        .await
        .map_err(|e| ApiError(e.into()))?;

    let mut input = body.clone();
    common::apply_number_modifiers(Some(&previous), &mut input, collection);
    validate::apply_modifiers(Some(&previous), &mut input, collection);
    let mut record = previous.clone();
    records::apply_body(&mut record, &input);
    record_route::apply_auth_fields(
        collection,
        &body,
        &mut record,
        Some(&previous),
        manage,
        record_route::UPDATE_FAILED,
    )
    .await?;

    let stored = common::store_uploads(storage, &collection.id, &id, &uploads).await?;
    let upload_meta: Vec<UploadMeta> = uploads.iter().map(StagedUpload::meta).collect();

    let before = previous.clone();
    let saved = record_route::write_record(
        tx.clone(),
        collection.clone(),
        record,
        Some(before),
        Write::Update,
        upload_meta,
    )
    .await;
    let saved = match saved {
        Ok(saved) => saved,
        Err(e) => {
            common::remove_keys(storage, &stored).await;
            return Err(common::relabel_upload_errors(ApiError(e), &uploads));
        }
    };

    let kept: std::collections::HashSet<String> =
        common::record_file_names(&saved).into_iter().collect();
    let mut post_commit_removals = Vec::new();
    for name in common::record_file_names(&previous) {
        if !kept.contains(&name) {
            post_commit_removals.push((collection.id.clone(), id.clone(), name));
        }
    }

    let value = serialize_record(tx.app(), collection, info, saved.clone(), manage).await?;
    Ok(ProcessOutcome {
        entry: serde_json::json!({ "status": 200, "body": value }),
        new_storage_keys: stored,
        post_commit_removals,
        post_commit_deletes: Vec::new(),
        realtime_event: Some((collection.clone(), RecordAction::Update, saved)),
    })
}

/// `PUT .../records` with an `id` in the body: update if that id already
/// exists, otherwise create it with that exact id — PocketBase's batch
/// upsert. A blank/absent id behaves like a plain create.
async fn upsert_item(
    ictx: &ItemCtx<'_>,
    body: Map<String, Value>,
    uploads: Vec<StagedUpload>,
) -> Result<ProcessOutcome, ApiError> {
    let ItemCtx { tx, collection, .. } = *ictx;
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let existing = if id.is_empty() {
        None
    } else {
        match records::find_by_id_raw(tx, collection, &id).await {
            Ok(record) => Some(record),
            Err(DbError::NotFound) => None,
            Err(e) => return Err(ApiError(record_route::read_error(e))),
        }
    };

    match existing {
        Some(_) => update_item(ictx, id, body, uploads).await,
        None => {
            let explicit_id = if id.is_empty() { None } else { Some(id) };
            create_item(ictx, body, uploads, explicit_id).await
        }
    }
}

/// Mirrors `records::delete_record`, minus the immediate storage sweep —
/// deferred to after the whole batch commits.
async fn delete_item(
    tx: &TxApp,
    collection: &Arc<Collection>,
    ctx: &RequestContext,
    id: String,
) -> Result<ProcessOutcome, ApiError> {
    if rules::is_superuser_only(&collection.delete_rule, ctx) {
        return Err(ApiError(rule_errors::superusers_only()));
    }
    let record = records::find_by_id_raw(tx, collection, &id)
        .await
        .map_err(|e| ApiError(record_route::read_error(e)))?;
    if !common::record_matches_rule(
        tx,
        &tx.db().collections,
        ctx,
        collection,
        &collection.delete_rule,
        &id,
    )
    .await
    .map_err(|e| ApiError(e.into()))?
    {
        return Err(ApiError(rule_errors::hidden_record()));
    }

    let (deleted, removed) =
        record_route::delete_in_tx(tx.clone(), collection.clone(), record).await?;
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut post_commit_deletes = Vec::new();
    for file in &removed {
        if seen.insert((file.collection_id.clone(), file.record_id.clone())) {
            post_commit_deletes.push((file.collection_id.clone(), file.record_id.clone()));
        }
    }

    Ok(ProcessOutcome {
        entry: serde_json::json!({ "status": 204, "body": Value::Null }),
        new_storage_keys: Vec::new(),
        post_commit_removals: Vec::new(),
        post_commit_deletes,
        realtime_event: Some((collection.clone(), RecordAction::Delete, deleted)),
    })
}

/// Serialize a write's result the way `records::respond` does, minus
/// `?expand=` support (no batch conformance test exercises it, and a
/// relation graph resolved mid-transaction is a can of worms for another
/// day).
async fn serialize_record(
    app: &App,
    collection: &Arc<Collection>,
    info: &RequestInfo,
    record: Record,
    manage_access: bool,
) -> Result<Value, ApiError> {
    let show_email = common::show_email_for(info.auth.as_ref(), &record, manage_access);
    let mut value =
        common::enrich_and_serialize(app, collection, record, info.auth.clone(), show_email)
            .await?;
    common::project(&mut value, info.query.get("fields").and_then(Value::as_str));
    Ok(value)
}

// ------------------------------------------------------------------- errors

/// `data.requests = {code, message}` — the shape for a failure that
/// rejects the *whole* batch before any sub-request runs (too many
/// requests, none at all).
fn requests_validation_error(message: &str, code: &str, detail: &str) -> ApiError {
    let mut requests = Map::new();
    requests.insert("code".to_string(), Value::String(code.to_string()));
    requests.insert("message".to_string(), Value::String(detail.to_string()));
    let mut data = Map::new();
    data.insert("requests".to_string(), Value::Object(requests));
    ApiError::nested_validation(message, data)
}

/// An [`ApiError`] flattened into the `{status, message, data}` triple
/// batch nests under `data.requests["<index>"].response`.
fn error_response(err: ApiError) -> Value {
    let body = err.error.body();
    let data = match err.data {
        Some(map) => Value::Object(map.into_iter().collect()),
        None => Value::Object(body.data.into_iter().collect()),
    };
    serde_json::json!({ "status": body.status, "message": body.message, "data": data })
}

// ------------------------------------------------------------------ handler

async fn batch(
    State(app): State<App>,
    info: RequestInfo,
    request: Request,
) -> ApiResult<impl IntoResponse> {
    let settings = app.settings();
    if !settings.batch.enabled {
        return Err(ApiError::forbidden("Batch requests are not allowed."));
    }
    let max_requests = settings.batch.max_requests;

    let parsed = parse_batch_body(&app, request).await?;
    let items = assemble(parsed);

    if items.is_empty() {
        return Err(requests_validation_error(
            "Failed to process batch requests.",
            codes::REQUIRED,
            "Cannot be blank.",
        ));
    }
    if max_requests > 0 && items.len() as i64 > max_requests {
        return Err(requests_validation_error(
            "Failed to process batch requests.",
            "validation_length_too_long",
            "The number of batch requests exceeds the max allowed length.",
        ));
    }

    // `onBatchRequest`: a plugin's last chance to inspect or reject the
    // whole batch before any sub-request touches the database.
    let requests_json: Vec<Value> = items
        .iter()
        .map(
            |item| serde_json::json!({ "method": item.method, "url": item.url, "body": item.body }),
        )
        .collect();
    let mut hook_event =
        BatchRequestEvent::new(app.clone(), info.clone(), info.auth.clone(), requests_json);
    app.hooks()
        .on_batch_request
        .trigger_bare(&mut hook_event)
        .await
        .map_err(ApiError)?;

    let successes: Arc<Mutex<Vec<ProcessOutcome>>> = Arc::new(Mutex::new(Vec::new()));
    let failure: Arc<Mutex<Option<(usize, Value)>>> = Arc::new(Mutex::new(None));
    let successes_tx = successes.clone();
    let failure_tx = failure.clone();
    let info_tx = info.clone();

    let result: Result<(), AppError> = app
        .run_in_transaction(move |tx| async move {
            for (idx, item) in items.into_iter().enumerate() {
                match process_item(&tx, &info_tx, item).await {
                    Ok(outcome) => successes_tx
                        .lock()
                        .expect("batch outcomes poisoned")
                        .push(outcome),
                    Err(err) => {
                        *failure_tx.lock().expect("batch failure poisoned") =
                            Some((idx, error_response(err)));
                        return Err(AppError::bad_request("Batch transaction failed."));
                    }
                }
            }
            Ok(())
        })
        .await;

    match result {
        Ok(()) => {
            let successes =
                std::mem::take(&mut *successes.lock().expect("batch outcomes poisoned"));
            let storage = app.storage();

            for outcome in &successes {
                for (collection_id, record_id, name) in &outcome.post_commit_removals {
                    common::remove_file(&storage, collection_id, record_id, name).await;
                }
            }
            let mut swept: std::collections::HashSet<(String, String)> =
                std::collections::HashSet::new();
            for outcome in &successes {
                for (collection_id, record_id) in &outcome.post_commit_deletes {
                    if swept.insert((collection_id.clone(), record_id.clone())) {
                        let prefix = format!("{collection_id}/{record_id}/");
                        if let Err(e) = storage.delete_prefix(&prefix).await {
                            tracing::warn!(prefix = %prefix, error = %e, "failed to remove batch record files");
                        }
                    }
                }
            }
            for outcome in &successes {
                if let Some((collection, action, record)) = &outcome.realtime_event {
                    realtime::publish(&app, collection, *action, record);
                }
            }

            let entries: Vec<Value> = successes.into_iter().map(|o| o.entry).collect();
            Ok(Json(Value::Array(entries)))
        }
        Err(_) => {
            let successes =
                std::mem::take(&mut *successes.lock().expect("batch outcomes poisoned"));
            let storage = app.storage();
            let keys: Vec<String> = successes
                .into_iter()
                .flat_map(|o| o.new_storage_keys)
                .collect();
            common::remove_keys(&storage, &keys).await;

            let (idx, response) = failure
                .lock()
                .expect("batch failure poisoned")
                .take()
                .expect("batch rolled back without a captured failure");
            let mut requests = Map::new();
            requests.insert(
                idx.to_string(),
                serde_json::json!({
                    "code": "batch_request_failed",
                    "message": "Batch request failed.",
                    "response": response,
                }),
            );
            let mut data = Map::new();
            data.insert("requests".to_string(), Value::Object(requests));
            Err(ApiError::nested_validation(
                "Batch transaction failed.",
                data,
            ))
        }
    }
}
