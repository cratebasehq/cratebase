//! `/api/collections/{collection}/records` — the read and write surface
//! every SDK call ultimately lands on, and the one this project is
//! benchmarked against.
//!
//! # Shape of a request
//!
//! 1. resolve the collection from the **in-memory store** (never a query);
//! 2. resolve the caller once (the foundation caches it on the request);
//! 3. turn the request into a [`RequestContext`] so rules and filters see
//!    `@request.body`, `@request.query`, `@request.headers`,
//!    `@request.method` and `@request.auth`;
//! 4. apply the API rule — and PocketBase's four *different* outcomes for
//!    a failing one (see [`crate::http_error::rule_errors`]);
//! 5. run the hook sequence (`*Request` → validate → `*Execute` →
//!    `AfterSuccess`/`AfterError`) with the write inside one transaction;
//! 6. serialize **once** and project `?fields=` over that same value.
//!
//! # Errors that are not what you would guess
//!
//! * a failing `listRule` is an empty page, not an error;
//! * `viewRule`/`updateRule`/`deleteRule` failures are `404`, so the API
//!   cannot be used to probe for hidden ids;
//! * a failing `createRule` is `400 "Failed to create record."`;
//! * a `null` rule for a non-superuser is `403`;
//! * a malformed body or an unusable `sort` is the *generic* 400 with an
//!   empty `data`.
//!
//! All five are pinned by `tests/conformance/records.test.ts`.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{FromRequest, Multipart, Path, Request, State};
use axum::http::{header, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use cratebase_core::{codes, AppError, Collection, FieldError, FieldKind, Record};
use cratebase_db::context::{CollectionResolver, RequestContext};
use cratebase_db::engine::Executor;
use cratebase_db::{expand, records, rules, validate, DbError, FileRef, ListParams, UploadMeta};
use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::app::{App, TxApp};
use crate::events::{collection_tags, RecordErrorEvent, RecordEvent, RecordRequestEvent};
use crate::extract::RequestInfo;
use crate::hooks::{Hook, Hooks};
use crate::http_error::{rule_errors, ApiError, ApiQuery, ApiResult};
use crate::realtime::{self, RecordAction};
use crate::routes::common::{self, ParsedBody};

/// PocketBase's per-operation error wrappers.
pub(crate) const CREATE_FAILED: &str = "Failed to create record.";
pub(crate) const UPDATE_FAILED: &str = "Failed to update record.";
pub(crate) const DELETE_FAILED: &str = "Failed to delete record.";
/// Writing to a view collection.
const UNSUPPORTED_TYPE: &str = "Unsupported collection type.";

pub fn router() -> Router<App> {
    Router::new()
        .route(
            "/collections/{collection}/records",
            get(list).post(create_record),
        )
        .route(
            "/collections/{collection}/records/{id}",
            get(view).patch(update_record).delete(delete_record),
        )
}

// ------------------------------------------------------------------ queries

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListQuery {
    pub(crate) page: Option<i64>,
    pub(crate) per_page: Option<i64>,
    pub(crate) sort: Option<String>,
    pub(crate) filter: Option<String>,
    pub(crate) expand: Option<String>,
    pub(crate) fields: Option<String>,
    pub(crate) skip_total: Option<String>,
    /// `?nearestTo={field}:{comma-separated floats or another record's
    /// id}` — switches the list from the normal sort/paginate flow to
    /// application-side cosine-similarity ranking; see
    /// [`nearest_list`] and `crate::embeddings`'s module doc for the
    /// scope this is (and isn't) meant to cover.
    pub(crate) nearest_to: Option<String>,
    /// How many ranked results `?nearestTo=` returns; default 20,
    /// clamped to `records::MAX_PER_PAGE` like a normal `perPage`.
    pub(crate) nearest_limit: Option<i64>,
}

/// `?skipTotal=1` / `=true`. Anything else (including its absence) counts
/// the rows, which is the only reason the `COUNT(*)` runs at all.
fn flag(raw: Option<&String>) -> bool {
    matches!(
        raw.map(String::as_str),
        Some("1" | "true" | "TRUE" | "True")
    )
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SingleQuery {
    pub(crate) expand: Option<String>,
    pub(crate) fields: Option<String>,
}

/// The envelope PocketBase wraps every list in.
#[derive(Debug, serde::Serialize)]
pub(crate) struct Page {
    page: i64,
    #[serde(rename = "perPage")]
    per_page: i64,
    #[serde(rename = "totalItems")]
    total_items: i64,
    #[serde(rename = "totalPages")]
    total_pages: i64,
    items: Value,
}

// --------------------------------------------------------------------- list

pub(crate) async fn list(
    State(app): State<App>,
    Path(name): Path<String>,
    ApiQuery(query): ApiQuery<ListQuery>,
    info: RequestInfo,
) -> ApiResult<Json<Page>> {
    let collection = common::collection_of(&app, &name)?;
    let ctx = info.to_context();
    if rules::is_superuser_only(&collection.list_rule, &ctx) {
        return Err(ApiError(rule_errors::superusers_only()));
    }

    if let Some(spec) = query.nearest_to.clone() {
        let result = nearest_list(&app, &collection, &ctx, &query, &spec).await?;
        return finish_list(
            &app,
            &collection,
            &info,
            &ctx,
            result,
            query.fields.as_deref(),
        )
        .await;
    }

    let slot: Arc<Mutex<Option<cratebase_db::ListResult>>> = Arc::new(Mutex::new(None));
    let mut event = RecordRequestEvent::new(
        app.clone(),
        collection.clone(),
        info.clone(),
        info.auth.clone(),
        None,
        collection_tags(&collection),
    );
    {
        let inner_app = app.clone();
        let inner_collection = collection.clone();
        let sink = slot.clone();
        let page = query.page.unwrap_or(1);
        let per_page = query.per_page.unwrap_or(records::DEFAULT_PER_PAGE);
        let sort = query.sort.clone();
        let filter = query.filter.clone();
        let expand = query.expand.clone();
        let skip_total = flag(query.skip_total.as_ref());
        app.hooks()
            .on_record_list_request
            .trigger(&mut event, move |e| {
                let ctx = e.request.to_context();
                Box::pin(async move {
                    let result = records::list(
                        inner_app.db(),
                        &inner_app.db().collections,
                        &ctx,
                        &inner_collection,
                        ListParams {
                            page,
                            per_page,
                            sort: sort.as_deref(),
                            filter: filter.as_deref(),
                            expand: expand.as_deref(),
                            skip_total,
                        },
                    )
                    .await
                    .map_err(read_error)?;
                    *sink.lock().expect("list slot poisoned") = Some(result);
                    Ok(())
                })
            })
            .await
            .map_err(ApiError)?;
    }

    // A hook that stopped the chain without producing a page answers with
    // an empty one rather than a 500.
    let result =
        slot.lock()
            .expect("list slot poisoned")
            .take()
            .unwrap_or(cratebase_db::ListResult {
                items: vec![],
                page: query.page.unwrap_or(1).max(1),
                per_page: query
                    .per_page
                    .unwrap_or(records::DEFAULT_PER_PAGE)
                    .clamp(1, records::MAX_PER_PAGE),
                total_items: 0,
                total_pages: 0,
            });

    finish_list(
        &app,
        &collection,
        &info,
        &ctx,
        result,
        query.fields.as_deref(),
    )
    .await
}

/// The manage-access lookup, per-item serialization and `?fields=`
/// projection every list result goes through, whether it came from the
/// normal hooked fetch or [`nearest_list`]'s ranking.
async fn finish_list(
    app: &App,
    collection: &Arc<Collection>,
    info: &RequestInfo,
    ctx: &RequestContext,
    result: cratebase_db::ListResult,
    fields: Option<&str>,
) -> ApiResult<Json<Page>> {
    let manageable = manageable_ids(app, collection, ctx, &result.items).await?;
    let mut items = Vec::with_capacity(result.items.len());
    for record in result.items {
        let show_email = common::show_email_for(
            info.auth.as_ref(),
            &record,
            manageable.contains(record.id()),
        );
        items.push(
            common::enrich_and_serialize(app, collection, record, info.auth.clone(), show_email)
                .await?,
        );
    }
    let mut items = Value::Array(items);
    common::project(&mut items, fields);

    Ok(Json(Page {
        page: result.page,
        per_page: result.per_page,
        total_items: result.total_items,
        total_pages: result.total_pages,
        items,
    }))
}

/// Safety ceiling on how many rule/filter-matching rows `?nearestTo=`
/// will fetch before ranking. This is the honest "thousands of rows, not
/// millions" scale limit documented on `crate::embeddings`: there is no
/// SQL-level ANN index behind a vector field in this pass, so every
/// candidate row is pulled into memory and ranked in Rust. A collection
/// with more matching rows than this cap silently ranks only the first
/// [`NEAREST_CANDIDATE_CAP`] of them rather than trying (and failing) to
/// load the whole table.
const NEAREST_CANDIDATE_CAP: usize = 20_000;
/// Page size used while gathering `?nearestTo=` candidates directly
/// through [`records::list`], independent of the caller's own `perPage`.
const NEAREST_FETCH_PAGE: i64 = 1000;

/// `?nearestTo={field}:{target}` — fetch every row the list rule/filter
/// would normally return (paginated internally up to
/// [`NEAREST_CANDIDATE_CAP`], ignoring `?sort=`), rank by cosine
/// similarity to the resolved query vector, and truncate to
/// `?nearestLimit=` (default 20). See `crate::embeddings`'s module doc
/// for why this is application-side rather than a native ANN index.
async fn nearest_list(
    app: &App,
    collection: &Arc<Collection>,
    ctx: &RequestContext,
    query: &ListQuery,
    spec: &str,
) -> ApiResult<cratebase_db::ListResult> {
    let (field_name, target) = spec
        .split_once(':')
        .ok_or_else(|| ApiError::bad_request("nearestTo must be \"field:vector-or-id\""))?;
    let field = collection
        .field(field_name)
        .filter(|f| matches!(f.kind, FieldKind::Vector { .. }))
        .ok_or_else(|| ApiError::bad_request(format!("'{field_name}' is not a vector field")))?;
    let FieldKind::Vector { dimensions, .. } = &field.kind else {
        unreachable!("filtered to Vector above")
    };

    let query_vector = resolve_query_vector(app, collection, ctx, field_name, target).await?;
    if query_vector.len() != *dimensions {
        return Err(ApiError::bad_request(format!(
            "nearestTo vector must have {dimensions} dimension(s), got {}",
            query_vector.len()
        )));
    }
    let limit = query
        .nearest_limit
        .unwrap_or(20)
        .clamp(1, records::MAX_PER_PAGE) as usize;

    let mut candidates = Vec::new();
    let mut page = 1;
    loop {
        let batch = records::list(
            app.db(),
            &app.db().collections,
            ctx,
            collection,
            ListParams {
                page,
                per_page: NEAREST_FETCH_PAGE,
                sort: None,
                filter: query.filter.as_deref(),
                expand: None,
                skip_total: true,
            },
        )
        .await
        .map_err(read_error)?;
        let got = batch.items.len();
        candidates.extend(batch.items);
        if got < NEAREST_FETCH_PAGE as usize || candidates.len() >= NEAREST_CANDIDATE_CAP {
            break;
        }
        page += 1;
    }
    candidates.truncate(NEAREST_CANDIDATE_CAP);

    let mut ranked: Vec<(f32, Record)> = candidates
        .into_iter()
        .filter_map(|r| {
            let v = crate::embeddings::vector_of(&r, field_name)?;
            Some((crate::embeddings::cosine_similarity(&query_vector, &v), r))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    ranked.truncate(limit);
    let mut items: Vec<Record> = ranked.into_iter().map(|(_, r)| r).collect();

    if let Some(spec) = query
        .expand
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        expand::resolve(app.db(), &app.db().collections, ctx, &mut items, spec, 0)
            .await
            .map_err(read_error)?;
    }

    Ok(cratebase_db::ListResult {
        total_items: items.len() as i64,
        total_pages: 1,
        page: 1,
        per_page: limit as i64,
        items,
    })
}

/// Resolve `?nearestTo=`'s target into a query vector: either a
/// comma-separated list of floats, or another record's id — whose own
/// `field_name` value (subject to the collection's `viewRule`, so this
/// cannot be used to probe a hidden record's vector) becomes the vector.
async fn resolve_query_vector(
    app: &App,
    collection: &Arc<Collection>,
    ctx: &RequestContext,
    field_name: &str,
    target: &str,
) -> ApiResult<Vec<f32>> {
    let looks_numeric = target.contains(|c: char| c.is_ascii_digit())
        && target
            .split(',')
            .all(|part| part.trim().parse::<f32>().is_ok());
    if looks_numeric {
        return Ok(target
            .split(',')
            .map(|part| part.trim().parse::<f32>().unwrap_or(0.0))
            .collect());
    }
    let record = records::find_by_id(
        app.db(),
        &app.db().collections,
        ctx,
        collection,
        target,
        None,
    )
    .await
    .map_err(read_error)?;
    crate::embeddings::vector_of(&record, field_name).ok_or_else(|| {
        ApiError::bad_request(format!("record '{target}' has no '{field_name}' vector"))
    })
}

/// The subset of `items` the caller has `manageRule` access to, resolved
/// in **one** query rather than one per row. Free for every collection
/// without a manage rule, which is all of them by default.
async fn manageable_ids(
    app: &App,
    collection: &Arc<Collection>,
    ctx: &RequestContext,
    items: &[Record],
) -> ApiResult<HashSet<String>> {
    if !collection.is_auth() || items.is_empty() {
        return Ok(HashSet::new());
    }
    if ctx.is_superuser() {
        return Ok(items.iter().map(|r| r.id().to_string()).collect());
    }
    if ctx.auth.is_none() || collection.auth.manage_rule.is_none() {
        return Ok(HashSet::new());
    }
    let ids: Vec<String> = items.iter().map(|r| r.id().to_string()).collect();
    common::records_matching_rule(
        app.db(),
        &app.db().collections,
        ctx,
        collection,
        &collection.auth.manage_rule,
        &ids,
    )
    .await
    .map_err(|e| ApiError(e.into()))
}

// --------------------------------------------------------------------- view

pub(crate) async fn view(
    State(app): State<App>,
    Path((name, id)): Path<(String, String)>,
    ApiQuery(query): ApiQuery<SingleQuery>,
    info: RequestInfo,
) -> ApiResult<Json<Value>> {
    let collection = common::collection_of(&app, &name)?;
    let ctx = info.to_context();
    if rules::is_superuser_only(&collection.view_rule, &ctx) {
        return Err(ApiError(rule_errors::superusers_only()));
    }

    let mut event = RecordRequestEvent::new(
        app.clone(),
        collection.clone(),
        info.clone(),
        info.auth.clone(),
        None,
        collection_tags(&collection),
    );
    {
        let inner_app = app.clone();
        let inner_collection = collection.clone();
        let expand = query.expand.clone();
        app.hooks()
            .on_record_view_request
            .trigger(&mut event, move |e| {
                let ctx = e.request.to_context();
                Box::pin(async move {
                    let record = records::find_by_id(
                        inner_app.db(),
                        &inner_app.db().collections,
                        &ctx,
                        &inner_collection,
                        &id,
                        expand.as_deref(),
                    )
                    .await
                    .map_err(read_error)?;
                    e.record = Some(record);
                    Ok(())
                })
            })
            .await
            .map_err(ApiError)?;
    }

    let record = event
        .record
        .ok_or_else(|| ApiError(rule_errors::hidden_record()))?;
    let manage = common::has_manage_access(
        app.db(),
        &app.db().collections,
        &ctx,
        &collection,
        record.id(),
    )
    .await
    .map_err(|e| ApiError(e.into()))?;
    let show_email = common::show_email_for(info.auth.as_ref(), &record, manage);
    let mut value =
        common::enrich_and_serialize(&app, &collection, record, info.auth.clone(), show_email)
            .await?;
    common::project(&mut value, query.fields.as_deref());
    Ok(Json(value))
}

// ------------------------------------------------------------------- create

pub(crate) async fn create_record(
    State(app): State<App>,
    Path(name): Path<String>,
    ApiQuery(query): ApiQuery<SingleQuery>,
    info: RequestInfo,
    request: Request,
) -> ApiResult<Json<Value>> {
    let collection = common::collection_of(&app, &name)?;
    if collection.is_view() {
        return Err(ApiError::bad_request(UNSUPPORTED_TYPE));
    }
    let body = read_body(&app, &collection, request).await?;
    let info = info.with_body(body.data.clone());
    let ctx = info.to_context();

    if rules::is_superuser_only(&collection.create_rule, &ctx) {
        return Err(ApiError(rule_errors::superusers_only()));
    }
    // Creating a new `_superusers` row is always "managing another
    // superuser" — there is no existing self row to be self-service
    // about — so it needs the stricter owner check on top of the plain
    // "any superuser" gate `is_superuser_only` just ran.
    if collection.is_superusers() {
        require_owner(info.auth.as_ref(), "create a superuser account")?;
    }
    {
        let resolver = CollectionResolver::new(
            collection.clone(),
            &app.db().collections,
            &ctx,
            app.db().dialect(),
        );
        if !rules::check_create_rule(app.db(), &resolver, &collection.create_rule)
            .await
            .map_err(read_error)?
        {
            return Err(ApiError(rule_errors::create_denied()));
        }
    }

    let mut input = body.data.clone();
    common::apply_number_modifiers(None, &mut input, &collection);
    validate::apply_modifiers(None, &mut input, &collection);
    let mut record = records::from_body(collection.clone(), &input);
    crate::embeddings::apply_embeddings(&mut record, &input)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    // The id must exist before the files are stored: the object key is
    // `{collectionId}/{recordId}/{name}`.
    if record.id().is_empty() {
        record.set_id(cratebase_core::record_id());
    }
    apply_auth_fields(
        &collection,
        &body.data,
        &mut record,
        None,
        ctx.is_superuser(),
        CREATE_FAILED,
    )
    .await?;

    let storage = app.storage();
    let stored =
        common::store_uploads(&storage, &collection.id, record.id(), &body.uploads).await?;
    let uploads = body.upload_meta();

    let saved = run_request(
        &app,
        &collection,
        &info,
        |hooks| &hooks.on_record_create_request,
        move |tx, collection, record| {
            Box::pin(write_record(
                tx,
                collection,
                record,
                None,
                Write::Create,
                uploads,
            ))
        },
        record,
        true,
    )
    .await;

    let saved = match saved {
        Ok(saved) => saved,
        Err(e) => {
            common::remove_keys(&storage, &stored).await;
            return Err(common::relabel_upload_errors(e, &body.uploads));
        }
    };

    realtime::publish(&app, &collection, RecordAction::Create, &saved);
    respond(&app, &collection, &info, saved, query, ctx.is_superuser()).await
}

// ------------------------------------------------------------------- update

pub(crate) async fn update_record(
    State(app): State<App>,
    Path((name, id)): Path<(String, String)>,
    ApiQuery(query): ApiQuery<SingleQuery>,
    info: RequestInfo,
    request: Request,
) -> ApiResult<Json<Value>> {
    let collection = common::collection_of(&app, &name)?;
    if collection.is_view() {
        return Err(ApiError::bad_request(UNSUPPORTED_TYPE));
    }
    let body = read_body(&app, &collection, request).await?;
    let info = info.with_body(body.data.clone());
    let ctx = info.to_context();

    if rules::is_superuser_only(&collection.update_rule, &ctx) {
        return Err(ApiError(rule_errors::superusers_only()));
    }
    let previous = records::find_by_id_raw(app.db(), &collection, &id)
        .await
        .map_err(read_error)?;
    if !common::record_matches_rule(
        app.db(),
        &app.db().collections,
        &ctx,
        &collection,
        &collection.update_rule,
        &id,
    )
    .await
    .map_err(|e| ApiError(e.into()))?
    {
        return Err(ApiError(rule_errors::hidden_record()));
    }

    // `_superusers`: any superuser may still update their own
    // non-role fields (self-service password/email changes stay on the
    // `is_superuser_only` gate above), but touching *another* account,
    // or changing anyone's `role` (including one's own — closing a
    // self-promotion hole), needs an owner. On top of that, demoting the
    // sole remaining owner is rejected outright: nobody would be left
    // who could ever promote a replacement.
    if collection.is_superusers() {
        let role_change = body.data.contains_key("role");
        let touches_other = info.auth.as_ref().is_none_or(|a| a.id != id);
        if role_change || touches_other {
            require_owner(info.auth.as_ref(), "manage another superuser's account")?;
        }
        if role_change {
            let demoted_from_owner = previous.get_string("role")
                == cratebase_core::SUPERUSER_ROLE_OWNER
                && body.data.get("role").and_then(Value::as_str)
                    != Some(cratebase_core::SUPERUSER_ROLE_OWNER);
            if demoted_from_owner && count_owners(&app).await? <= 1 {
                return Err(ApiError::bad_request(
                    "Cannot change the role of the last remaining owner.",
                ));
            }
        }
    }

    let manage = common::has_manage_access(app.db(), &app.db().collections, &ctx, &collection, &id)
        .await
        .map_err(|e| ApiError(e.into()))?;

    let mut input = body.data.clone();
    common::apply_number_modifiers(Some(&previous), &mut input, &collection);
    validate::apply_modifiers(Some(&previous), &mut input, &collection);
    let mut record = previous.clone();
    records::apply_body(&mut record, &input);
    crate::embeddings::apply_embeddings(&mut record, &input)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    apply_auth_fields(
        &collection,
        &body.data,
        &mut record,
        Some(&previous),
        manage,
        UPDATE_FAILED,
    )
    .await?;

    let storage = app.storage();
    let stored = common::store_uploads(&storage, &collection.id, &id, &body.uploads).await?;
    let uploads = body.upload_meta();

    let before = previous.clone();
    let saved = run_request(
        &app,
        &collection,
        &info,
        |hooks| &hooks.on_record_update_request,
        move |tx, collection, record| {
            Box::pin(write_record(
                tx,
                collection,
                record,
                Some(before),
                Write::Update,
                uploads,
            ))
        },
        record,
        true,
    )
    .await;

    let saved = match saved {
        Ok(saved) => saved,
        Err(e) => {
            common::remove_keys(&storage, &stored).await;
            return Err(common::relabel_upload_errors(e, &body.uploads));
        }
    };

    // Replaced or removed files are only orphaned once the row is
    // committed; deleting them earlier would lose data on a rollback.
    let kept: HashSet<String> = common::record_file_names(&saved).into_iter().collect();
    for name in common::record_file_names(&previous) {
        if !kept.contains(&name) {
            common::remove_file(&storage, &collection.id, &id, &name).await;
        }
    }

    realtime::publish(&app, &collection, RecordAction::Update, &saved);
    respond(&app, &collection, &info, saved, query, manage).await
}

// ------------------------------------------------------------------- delete

pub(crate) async fn delete_record(
    State(app): State<App>,
    Path((name, id)): Path<(String, String)>,
    info: RequestInfo,
) -> ApiResult<StatusCode> {
    let collection = common::collection_of(&app, &name)?;
    if collection.is_view() {
        return Err(ApiError::bad_request(UNSUPPORTED_TYPE));
    }
    let ctx = info.to_context();
    if rules::is_superuser_only(&collection.delete_rule, &ctx) {
        return Err(ApiError(rule_errors::superusers_only()));
    }
    let record = records::find_by_id_raw(app.db(), &collection, &id)
        .await
        .map_err(read_error)?;
    if !common::record_matches_rule(
        app.db(),
        &app.db().collections,
        &ctx,
        &collection,
        &collection.delete_rule,
        &id,
    )
    .await
    .map_err(|e| ApiError(e.into()))?
    {
        return Err(ApiError(rule_errors::hidden_record()));
    }

    // Deleting a superuser account is always owner-only, self or not —
    // PocketBase-style admin/session lockout is a worse failure mode
    // than a superuser having to ask an owner to remove their own
    // account. The sole remaining owner can never be deleted at all:
    // that would leave nobody who could ever create another one.
    if collection.is_superusers() {
        require_owner(info.auth.as_ref(), "delete a superuser account")?;
        if record.get_string("role") == cratebase_core::SUPERUSER_ROLE_OWNER
            && count_owners(&app).await? <= 1
        {
            return Err(ApiError::bad_request(
                "Cannot delete the last remaining owner.",
            ));
        }
    }

    let files: Arc<Mutex<Vec<FileRef>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = files.clone();
    let deleted = run_request(
        &app,
        &collection,
        &info,
        |hooks| &hooks.on_record_delete_request,
        move |tx, collection, record| {
            Box::pin(async move {
                let (record, removed) = delete_in_tx(tx, collection, record).await?;
                *sink.lock().expect("file slot poisoned") = removed;
                Ok(record)
            })
        },
        record,
        delete_needs_transaction(&app, &collection),
    )
    .await?;

    // The storage sweep happens after the commit; `records::delete`
    // reports every record the (possibly cascading) delete removed, so
    // clearing each one's directory also takes its cached thumbs.
    let storage = app.storage();
    // Taken out of the mutex before the first `await`: a `MutexGuard` held
    // across one would make this handler's future non-`Send`.
    let removed: Vec<FileRef> = std::mem::take(&mut *files.lock().expect("file slot poisoned"));
    let mut swept: HashSet<(String, String)> = HashSet::new();
    for file in &removed {
        if swept.insert((file.collection_id.clone(), file.record_id.clone())) {
            let prefix = format!("{}/{}/", file.collection_id, file.record_id);
            if let Err(e) = storage.delete_prefix(&prefix).await {
                tracing::warn!(prefix = %prefix, error = %e, "failed to remove record files");
            }
        }
    }

    realtime::publish(&app, &collection, RecordAction::Delete, &deleted);
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------- `_superusers` roles

/// The owner-only gate `create_record`/`update_record`/`delete_record`
/// apply to `_superusers` writes that manage *another* account. `auth`
/// is `None` only when a plugin or test calls one of those handlers
/// with no caller at all, which can't happen over real HTTP (the
/// `is_superuser_only` check above already rejected it) but is handled
/// the safe way regardless. Delegates to [`crate::extract::RequireOwner`]
/// so the request-extractor form and this inline form can never
/// disagree about who counts as an owner.
fn require_owner(auth: Option<&crate::extract::Auth>, action: &str) -> ApiResult<()> {
    if auth.is_some_and(crate::extract::RequireOwner::holds) {
        Ok(())
    } else {
        Err(ApiError(AppError::Forbidden(format!(
            "Only an owner can {action}."
        ))))
    }
}

/// How many `_superusers` rows currently carry `role = "owner"` — the
/// count the lockout checks in `update_record`/`delete_record` compare
/// against so the very last owner can never be demoted or deleted.
async fn count_owners(app: &App) -> ApiResult<i64> {
    Ok(app
        .db()
        .query_scalar(
            r#"SELECT COUNT(*) FROM "_superusers" WHERE "role" = 'owner'"#,
            &[],
        )
        .await
        .map_err(|e| ApiError(AppError::from(e)))?
        .and_then(|v| v.as_i64())
        .unwrap_or(0))
}

/// Whether this delete has to be wrapped in an explicit transaction.
///
/// It does when it is more than one statement — a cascade or a stripped
/// reference, see [`records::delete_touches_other_collections`] — or when
/// any delete hook is bound, because a hook may write through the same
/// scope (those writes must commit with the delete) or fail after it (the
/// delete must then roll back). With neither, the whole operation is a
/// single `DELETE ... WHERE id = ?`, which is already atomic on its own,
/// and `BEGIN IMMEDIATE`/`COMMIT` would only add two writer round trips
/// and hold the single writer connection across them.
///
/// The default configuration — no bound hooks, no relation pointing at
/// the collection — is the common one, so this is the path most deletes
/// take.
fn delete_needs_transaction(app: &App, collection: &Collection) -> bool {
    if records::delete_touches_other_collections(&app.db().collections, collection) {
        return true;
    }
    let hooks = app.hooks();
    !(hooks.on_record_delete_request.is_empty()
        && hooks.on_record_delete.is_empty()
        && hooks.on_record_delete_execute.is_empty()
        && hooks.on_record_after_delete_success.is_empty()
        && hooks.on_record_after_delete_error.is_empty())
}

// ----------------------------------------------------------- write plumbing

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Write {
    Create,
    Update,
}

impl Write {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Write::Create => CREATE_FAILED,
            Write::Update => UPDATE_FAILED,
        }
    }

    pub(crate) fn outer(self, hooks: &Hooks) -> &Hook<RecordEvent> {
        match self {
            Write::Create => &hooks.on_record_create,
            Write::Update => &hooks.on_record_update,
        }
    }

    pub(crate) fn execute(self, hooks: &Hooks) -> &Hook<RecordEvent> {
        match self {
            Write::Create => &hooks.on_record_create_execute,
            Write::Update => &hooks.on_record_update_execute,
        }
    }
}

/// The `*Request` hook and the transaction it wraps — the part every
/// write shares. `work` runs inside `App::run_scoped`, so its own
/// database calls (and any hook's) join the same transaction.
///
/// `transactional` is false only for a write that is provably one
/// statement with nothing able to join or abort it; see
/// [`delete_needs_transaction`]. Creates and updates always pass true:
/// their validation reads (uniqueness, relation existence) have to see
/// the same snapshot the row is written against.
async fn run_request<F>(
    app: &App,
    collection: &Arc<Collection>,
    info: &RequestInfo,
    hook: fn(&Hooks) -> &Hook<RecordRequestEvent>,
    work: F,
    record: Record,
    transactional: bool,
) -> ApiResult<Record>
where
    F: FnOnce(TxApp, Arc<Collection>, Record) -> BoxFuture<'static, Result<Record, AppError>>
        + Send
        + 'static,
{
    let mut event = RecordRequestEvent::new(
        app.clone(),
        collection.clone(),
        info.clone(),
        info.auth.clone(),
        Some(record),
        collection_tags(collection),
    );

    let outcome = {
        let inner_app = app.clone();
        let inner_collection = collection.clone();
        hook(app.hooks())
            .trigger(&mut event, move |e| {
                let record = e.record.take();
                Box::pin(async move {
                    let Some(record) = record else {
                        return Ok(());
                    };
                    let saved = inner_app
                        .run_scoped(transactional, move |tx| work(tx, inner_collection, record))
                        .await?;
                    e.record = Some(saved);
                    Ok(())
                })
            })
            .await
    };

    match outcome {
        Ok(()) => event
            .record
            .ok_or_else(|| ApiError::internal("the write hook produced no record")),
        Err(error) => Err(ApiError(error)),
    }
}

/// Validate + write one record inside the caller's transaction, firing
/// `onRecordValidate`, `onRecord{Create,Update}`, their `*Execute`
/// siblings and finally the after-success / after-error hooks.
pub(crate) async fn write_record(
    tx: TxApp,
    collection: Arc<Collection>,
    record: Record,
    previous: Option<Record>,
    write: Write,
    uploads: Vec<UploadMeta>,
) -> Result<Record, AppError> {
    let tags = collection_tags(&collection);

    let mut event = RecordEvent::new(
        tx.clone(),
        collection.clone(),
        record,
        previous.clone(),
        tags.clone(),
    );
    tx.hooks()
        .on_record_validate
        .trigger_bare(&mut event)
        .await?;

    let mut event = RecordEvent::new(
        tx.clone(),
        collection.clone(),
        event.record,
        previous.clone(),
        tags.clone(),
    );
    let inner_tags = tags.clone();
    let result = write
        .outer(tx.hooks())
        .trigger(&mut event, move |e| {
            let app = e.app.clone();
            let collection = e.collection.clone();
            let previous = e.previous.clone();
            let record = std::mem::replace(&mut e.record, Record::new(collection.clone()));
            Box::pin(async move {
                let mut inner =
                    RecordEvent::new(app.clone(), collection, record, previous, inner_tags);
                let outcome = write
                    .execute(app.hooks())
                    .trigger(&mut inner, move |ie| {
                        Box::pin(async move {
                            let store = ie.app.db().collections.clone();
                            // Disjoint field borrows: the executor is the
                            // event's `app`, the record its `record`.
                            let executor: &dyn Executor = &ie.app;
                            let outcome = match write {
                                Write::Create => {
                                    records::create_with_uploads(
                                        executor,
                                        &store,
                                        &mut ie.record,
                                        &uploads,
                                    )
                                    .await
                                }
                                Write::Update => {
                                    records::update_with_uploads(
                                        executor,
                                        &store,
                                        &mut ie.record,
                                        &uploads,
                                    )
                                    .await
                                }
                            };
                            outcome.map_err(|e| write_error(e, write.message()))
                        })
                    })
                    .await;
                e.record = inner.record;
                outcome
            })
        })
        .await;

    finish(tx, collection, event.record, previous, tags, write, result).await
}

/// Run the after-success or after-error hook and hand back the record.
async fn finish(
    tx: TxApp,
    collection: Arc<Collection>,
    record: Record,
    previous: Option<Record>,
    tags: Vec<String>,
    write: Write,
    result: Result<(), AppError>,
) -> Result<Record, AppError> {
    match result {
        Ok(()) => {
            let mut success = RecordEvent::new(tx.clone(), collection, record, previous, tags);
            let hook = match write {
                Write::Create => &tx.hooks().on_record_after_create_success,
                Write::Update => &tx.hooks().on_record_after_update_success,
            };
            hook.trigger_bare(&mut success).await?;
            Ok(success.record)
        }
        Err(error) => {
            let mut failure = RecordErrorEvent::new(
                tx.app().clone(),
                collection,
                record,
                error.to_string(),
                tags,
            );
            let hook = match write {
                Write::Create => &tx.hooks().on_record_after_create_error,
                Write::Update => &tx.hooks().on_record_after_update_error,
            };
            let _ = hook.trigger_bare(&mut failure).await;
            Err(error)
        }
    }
}

/// The delete counterpart of [`write_record`]. Returns the deleted record
/// plus the files it orphaned, which the caller removes after the commit.
pub(crate) async fn delete_in_tx(
    tx: TxApp,
    collection: Arc<Collection>,
    record: Record,
) -> Result<(Record, Vec<FileRef>), AppError> {
    let tags = collection_tags(&collection);
    let files: Arc<Mutex<Vec<FileRef>>> = Arc::new(Mutex::new(Vec::new()));
    let mut event = RecordEvent::new(tx.clone(), collection.clone(), record, None, tags.clone());

    let inner_tags = tags.clone();
    let sink = files.clone();
    let result = tx
        .hooks()
        .on_record_delete
        .trigger(&mut event, move |e| {
            let app = e.app.clone();
            let collection = e.collection.clone();
            let record = std::mem::replace(&mut e.record, Record::new(collection.clone()));
            Box::pin(async move {
                let mut inner = RecordEvent::new(app.clone(), collection, record, None, inner_tags);
                let outcome = app
                    .hooks()
                    .on_record_delete_execute
                    .trigger(&mut inner, move |ie| {
                        Box::pin(async move {
                            let store = ie.app.db().collections.clone();
                            let executor: &dyn Executor = &ie.app;
                            let removed = records::delete(executor, &store, &ie.record)
                                .await
                                .map_err(|e| write_error(e, DELETE_FAILED))?;
                            *sink.lock().expect("file slot poisoned") = removed;
                            Ok(())
                        })
                    })
                    .await;
                e.record = inner.record;
                outcome
            })
        })
        .await;

    let record = event.record;
    match result {
        Ok(()) => {
            let mut success = RecordEvent::new(tx.clone(), collection, record, None, tags);
            tx.hooks()
                .on_record_after_delete_success
                .trigger_bare(&mut success)
                .await?;
            let removed = std::mem::take(&mut *files.lock().expect("file slot poisoned"));
            Ok((success.record, removed))
        }
        Err(error) => {
            let mut failure = RecordErrorEvent::new(
                tx.app().clone(),
                collection,
                record,
                error.to_string(),
                tags,
            );
            let _ = tx
                .hooks()
                .on_record_after_delete_error
                .trigger_bare(&mut failure)
                .await;
            Err(error)
        }
    }
}

// ------------------------------------------------------------- auth fields

/// PocketBase's guards on the auth system fields.
///
/// * `password` needs a matching `passwordConfirm`, and — unless the
///   caller is a superuser or holds `manageRule` access — the current
///   `oldPassword`. Any password change rotates `tokenKey`, which is what
///   invalidates every outstanding session for the record without a
///   server-side session table.
/// * `verified` (and, on update, `email`) cannot be changed by the record
///   itself: PocketBase reports `validation_values_mismatch`, *not* a
///   permission error (KNOWN_DIVERGENCES §21). `email` is only guarded on
///   update, because a signup has to be able to supply one.
pub(crate) async fn apply_auth_fields(
    collection: &Arc<Collection>,
    body: &Map<String, Value>,
    record: &mut Record,
    previous: Option<&Record>,
    manage_access: bool,
    message: &str,
) -> ApiResult<()> {
    if !collection.is_auth() {
        return Ok(());
    }
    let mut errors: BTreeMap<String, FieldError> = BTreeMap::new();
    let mismatch = || FieldError::new(codes::VALUES_MISMATCH, "Values don't match.");

    let submitted_password = body
        .get("password")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .map(str::to_string);

    if let Some(password) = &submitted_password {
        if body.get("passwordConfirm").and_then(Value::as_str) != Some(password.as_str()) {
            errors.insert("passwordConfirm".into(), mismatch());
        }
        // Only a *change* needs the old password; a signup has none.
        if !manage_access && previous.is_some() {
            match body.get("oldPassword").and_then(Value::as_str) {
                None | Some("") => {
                    errors.insert(
                        "oldPassword".into(),
                        FieldError::new(codes::REQUIRED, "Cannot be blank."),
                    );
                }
                Some(old) => {
                    let stored = previous.map(Record::password_hash).unwrap_or_default();
                    if !cratebase_auth::verify_password_async(old, &stored).await {
                        errors.insert(
                            "oldPassword".into(),
                            FieldError::new(
                                codes::OLD_PASSWORD,
                                "Missing or invalid old password.",
                            ),
                        );
                    }
                }
            }
        }
    }

    if !manage_access {
        if let Some(value) = body.get("verified") {
            if truthy(value) != previous.map(Record::verified).unwrap_or(false) {
                errors.insert("verified".into(), mismatch());
            }
        }
        if previous.is_some() {
            if let Some(value) = body.get("email") {
                let wanted = value.as_str().unwrap_or_default();
                if wanted != previous.map(Record::email).unwrap_or_default() {
                    errors.insert("email".into(), mismatch());
                }
            }
        }
    }

    if !errors.is_empty() {
        return Err(ApiError(AppError::validation(message, errors)));
    }
    if submitted_password.is_some() {
        record.set("tokenKey", Value::String(crate::app::new_token_key()));
    }
    Ok(())
}

/// Go's `cast.ToBool`, which is what PocketBase compares a submitted
/// `verified` against.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::String(s) => matches!(s.as_str(), "1" | "t" | "T" | "true" | "TRUE" | "True"),
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        _ => false,
    }
}

// ------------------------------------------------------------------ helpers

/// Serialize the result of a write, honouring `?expand=` and `?fields=`.
async fn respond(
    app: &App,
    collection: &Arc<Collection>,
    info: &RequestInfo,
    mut record: Record,
    query: SingleQuery,
    manage_access: bool,
) -> ApiResult<Json<Value>> {
    if let Some(spec) = query
        .expand
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let ctx = info.to_context();
        let mut items = vec![record];
        cratebase_db::expand::resolve(app.db(), &app.db().collections, &ctx, &mut items, spec, 0)
            .await
            .map_err(read_error)?;
        record = items.remove(0);
    }
    let show_email = common::show_email_for(info.auth.as_ref(), &record, manage_access);
    let mut value =
        common::enrich_and_serialize(app, collection, record, info.auth.clone(), show_email)
            .await?;
    common::project(&mut value, query.fields.as_deref());
    Ok(Json(value))
}

/// JSON or multipart, chosen by `Content-Type`.
async fn read_body(
    app: &App,
    collection: &Arc<Collection>,
    request: Request,
) -> ApiResult<ParsedBody> {
    let is_multipart = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("multipart/form-data"));

    if is_multipart {
        let multipart = Multipart::from_request(request, app)
            .await
            .map_err(|_| ApiError(AppError::bad_request("")))?;
        common::parse_multipart(collection, multipart).await
    } else {
        let bytes = Bytes::from_request(request, app)
            .await
            .map_err(|_| ApiError(AppError::bad_request("")))?;
        common::parse_json_body(&bytes)
    }
}

/// A read failure. `NotFound` is PocketBase's plain 404; a rejected
/// `sort`/`filter` surfaces as [`DbError::Filter`] and must become the
/// *generic* 400 with an empty `data` (KNOWN_DIVERGENCES §18).
pub(crate) fn read_error(error: DbError) -> AppError {
    match error {
        DbError::NotFound => rule_errors::hidden_record(),
        DbError::Filter(detail) => {
            // The client only ever sees the generic wording, so the
            // reason a filter or sort was rejected has to reach the
            // operator here or nowhere.
            tracing::debug!(detail = %detail, "rejected filter or sort");
            AppError::bad_request("")
        }
        other => other.into(),
    }
}

/// A write failure, re-labelled with the operation's wrapper message.
fn write_error(error: DbError, message: &str) -> AppError {
    match error {
        DbError::Validation(fields) => AppError::validation(message, fields),
        DbError::Unsupported(_) => AppError::bad_request(UNSUPPORTED_TYPE),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_total_only_accepts_the_truthy_spellings() {
        assert!(flag(Some(&"1".to_string())));
        assert!(flag(Some(&"true".to_string())));
        assert!(!flag(Some(&"0".to_string())));
        assert!(!flag(None));
    }

    #[test]
    fn a_bad_sort_is_the_generic_400_with_no_field_data() {
        let err = read_error(DbError::Filter(
            cratebase_filter::FilterError::UnknownField("nope".into()),
        ));
        assert_eq!(err.status(), 400);
        assert_eq!(err.body().message, AppError::DEFAULT_BAD_REQUEST);
        assert!(err.body().data.is_empty());
    }

    #[test]
    fn validation_errors_carry_the_operation_wrapper() {
        let err = write_error(
            DbError::field("title", codes::REQUIRED, "Cannot be blank."),
            CREATE_FAILED,
        );
        let body = err.body();
        assert_eq!(body.message, CREATE_FAILED);
        assert_eq!(body.data["title"]["code"], codes::REQUIRED);
    }

    #[test]
    fn truthy_matches_gos_cast_tobool() {
        assert!(truthy(&Value::Bool(true)));
        assert!(truthy(&Value::String("true".into())));
        assert!(!truthy(&Value::String("yes".into())));
        assert!(!truthy(&Value::Null));
    }
}

#[cfg(test)]
mod superuser_role_tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use cratebase_auth::TokenType;
    use cratebase_db::engine::{Executor, Sql};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::app::App;
    use crate::config::Config;

    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    /// Creates a superuser (always `role: "owner"`, per
    /// `App::create_superuser`'s doc comment) and hands back its id and a
    /// bearer session token. `set_role` flips the row to a different
    /// role afterwards via a raw update — a stand-in for a dashboard
    /// admin who was demoted/promoted earlier, without going through the
    /// very HTTP path under test.
    async fn superuser(app: &App, email: &str, role: &str) -> (String, String) {
        let id = app
            .create_superuser(email, "password12345")
            .await
            .expect("create superuser");
        if role != cratebase_core::SUPERUSER_ROLE_OWNER {
            app.db()
                .execute(
                    r#"UPDATE "_superusers" SET "role" = $1 WHERE "id" = $2"#,
                    &[Sql::from(role), Sql::from(id.as_str())],
                )
                .await
                .expect("set role");
        }
        let token = app
            .mint_token("_superusers", &id, TokenType::Auth, 3600)
            .await
            .expect("mint token");
        (id, token)
    }

    fn json_request(method: &str, uri: &str, token: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn empty_request(method: &str, uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    async fn role_of(app: &App, id: &str) -> String {
        app.db()
            .query_scalar(
                r#"SELECT "role" FROM "_superusers" WHERE "id" = $1"#,
                &[Sql::from(id)],
            )
            .await
            .unwrap()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap()
    }

    #[tokio::test]
    async fn only_an_owner_can_create_a_new_superuser() {
        let (app, _dir) = test_app().await;
        let (_owner_id, owner_token) = superuser(&app, "owner@example.com", "owner").await;
        let (_admin_id, admin_token) = superuser(&app, "admin@example.com", "admin").await;
        let router = crate::routes::api_router(&app).with_state(app.clone());

        let rejected = router
            .clone()
            .oneshot(json_request(
                "POST",
                "/collections/_superusers/records",
                &admin_token,
                json!({
                    "email": "second@example.com",
                    "password": "password12345",
                    "passwordConfirm": "password12345",
                    "role": "admin",
                }),
            ))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);

        let accepted = router
            .oneshot(json_request(
                "POST",
                "/collections/_superusers/records",
                &owner_token,
                json!({
                    "email": "second@example.com",
                    "password": "password12345",
                    "passwordConfirm": "password12345",
                    "role": "admin",
                }),
            ))
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn sole_owner_cannot_be_demoted_or_deleted() {
        let (app, _dir) = test_app().await;
        let (owner_id, owner_token) = superuser(&app, "owner@example.com", "owner").await;
        let router = crate::routes::api_router(&app).with_state(app.clone());

        let demote = router
            .clone()
            .oneshot(json_request(
                "PATCH",
                &format!("/collections/_superusers/records/{owner_id}"),
                &owner_token,
                json!({ "role": "admin" }),
            ))
            .await
            .unwrap();
        assert_eq!(demote.status(), StatusCode::BAD_REQUEST);
        assert_eq!(role_of(&app, &owner_id).await, "owner");

        let delete = router
            .oneshot(empty_request(
                "DELETE",
                &format!("/collections/_superusers/records/{owner_id}"),
                &owner_token,
            ))
            .await
            .unwrap();
        assert_eq!(delete.status(), StatusCode::BAD_REQUEST);
        assert!(app.find_superuser_by_id(&owner_id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_second_owner_can_demote_and_delete_another_owner() {
        let (app, _dir) = test_app().await;
        let (owner1_id, owner1_token) = superuser(&app, "owner1@example.com", "owner").await;
        let (owner2_id, _owner2_token) = superuser(&app, "owner2@example.com", "owner").await;
        let (owner3_id, _owner3_token) = superuser(&app, "owner3@example.com", "owner").await;
        let router = crate::routes::api_router(&app).with_state(app.clone());

        let demote = router
            .clone()
            .oneshot(json_request(
                "PATCH",
                &format!("/collections/_superusers/records/{owner2_id}"),
                &owner1_token,
                json!({ "role": "admin" }),
            ))
            .await
            .unwrap();
        assert_eq!(demote.status(), StatusCode::OK);
        assert_eq!(role_of(&app, &owner2_id).await, "admin");

        let delete = router
            .oneshot(empty_request(
                "DELETE",
                &format!("/collections/_superusers/records/{owner3_id}"),
                &owner1_token,
            ))
            .await
            .unwrap();
        assert_eq!(delete.status(), StatusCode::NO_CONTENT);
        assert!(app
            .find_superuser_by_id(&owner3_id)
            .await
            .unwrap()
            .is_none());
        // The acting owner (owner1) is untouched and still an owner.
        assert_eq!(role_of(&app, &owner1_id).await, "owner");
    }

    #[tokio::test]
    async fn admin_role_keeps_every_non_management_superuser_endpoint() {
        let (app, _dir) = test_app().await;
        let (_owner_id, _owner_token) = superuser(&app, "owner@example.com", "owner").await;
        let (admin_id, admin_token) = superuser(&app, "admin@example.com", "admin").await;
        let router = crate::routes::api_router(&app).with_state(app.clone());

        // An ordinary superuser-only endpoint unrelated to `_superusers`
        // management: creating a `_cron_jobs` row. `is_superuser_only`
        // is the only gate here, exactly as before this feature existed.
        let created = router
            .clone()
            .oneshot(json_request(
                "POST",
                "/collections/_cron_jobs/records",
                &admin_token,
                json!({
                    "name": "noop",
                    "expression": "* * * * *",
                    "sql": "SELECT 1",
                }),
            ))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::OK);

        // Self-service on their own `_superusers` row (no role change)
        // still works for a plain admin.
        let self_update = router
            .oneshot(json_request(
                "PATCH",
                &format!("/collections/_superusers/records/{admin_id}"),
                &admin_token,
                json!({
                    "password": "newpassword12345",
                    "passwordConfirm": "newpassword12345",
                    "oldPassword": "password12345",
                }),
            ))
            .await
            .unwrap();
        assert_eq!(self_update.status(), StatusCode::OK);
    }
}
