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
use cratebase_core::{codes, AppError, Collection, FieldError, Record};
use cratebase_db::context::{CollectionResolver, RequestContext};
use cratebase_db::engine::Executor;
use cratebase_db::{records, rules, validate, DbError, FileRef, ListParams, UploadMeta};
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
const CREATE_FAILED: &str = "Failed to create record.";
const UPDATE_FAILED: &str = "Failed to update record.";
const DELETE_FAILED: &str = "Failed to delete record.";
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
struct ListQuery {
    page: Option<i64>,
    per_page: Option<i64>,
    sort: Option<String>,
    filter: Option<String>,
    expand: Option<String>,
    fields: Option<String>,
    skip_total: Option<String>,
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
struct SingleQuery {
    expand: Option<String>,
    fields: Option<String>,
}

/// The envelope PocketBase wraps every list in.
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

// --------------------------------------------------------------------- list

async fn list(
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

    let manageable = manageable_ids(&app, &collection, &ctx, &result.items).await?;
    let mut items = Vec::with_capacity(result.items.len());
    for record in result.items {
        let show_email = common::show_email_for(
            info.auth.as_ref(),
            &record,
            manageable.contains(record.id()),
        );
        items.push(
            common::enrich_and_serialize(&app, &collection, record, info.auth.clone(), show_email)
                .await?,
        );
    }
    let mut items = Value::Array(items);
    common::project(&mut items, query.fields.as_deref());

    Ok(Json(Page {
        page: result.page,
        per_page: result.per_page,
        total_items: result.total_items,
        total_pages: result.total_pages,
        items,
    }))
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

async fn view(
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

async fn create_record(
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

async fn update_record(
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

    let manage = common::has_manage_access(app.db(), &app.db().collections, &ctx, &collection, &id)
        .await
        .map_err(|e| ApiError(e.into()))?;

    let mut input = body.data.clone();
    common::apply_number_modifiers(Some(&previous), &mut input, &collection);
    validate::apply_modifiers(Some(&previous), &mut input, &collection);
    let mut record = previous.clone();
    records::apply_body(&mut record, &input);
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

async fn delete_record(
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

// ----------------------------------------------------------- write plumbing

#[derive(Clone, Copy, PartialEq, Eq)]
enum Write {
    Create,
    Update,
}

impl Write {
    fn message(self) -> &'static str {
        match self {
            Write::Create => CREATE_FAILED,
            Write::Update => UPDATE_FAILED,
        }
    }

    fn outer(self, hooks: &Hooks) -> &Hook<RecordEvent> {
        match self {
            Write::Create => &hooks.on_record_create,
            Write::Update => &hooks.on_record_update,
        }
    }

    fn execute(self, hooks: &Hooks) -> &Hook<RecordEvent> {
        match self {
            Write::Create => &hooks.on_record_create_execute,
            Write::Update => &hooks.on_record_update_execute,
        }
    }
}

/// The `*Request` hook and the transaction it wraps — the part every
/// write shares. `work` runs inside `App::run_in_transaction`, so its own
/// database calls (and any hook's) join the same transaction.
async fn run_request<F>(
    app: &App,
    collection: &Arc<Collection>,
    info: &RequestInfo,
    hook: fn(&Hooks) -> &Hook<RecordRequestEvent>,
    work: F,
    record: Record,
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
                        .run_in_transaction(move |tx| work(tx, inner_collection, record))
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
async fn write_record(
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
async fn delete_in_tx(
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
async fn apply_auth_fields(
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
fn read_error(error: DbError) -> AppError {
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
