use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::routing::get;
use axum::{Json, Router};
use cratebase_core::field::FieldType;
use cratebase_core::{new_id, AppError, Collection};
use cratebase_db::records::{self, ListParams};
use cratebase_db::resolver::{evaluate_create_rule, evaluate_rule, RequestContext, RuleOutcome};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::extract::CurrentAuth;
use crate::helpers::{file_key, load_collection, unique_filename};
use crate::http_error::{ApiError, ApiResult};
use crate::payload::{parse_payload, Upload};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/collections/{collection}/records", get(list).post(create))
        .route(
            "/collections/{collection}/records/{id}",
            get(view).patch(update).delete(remove),
        )
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub filter: Option<String>,
    pub sort: Option<String>,
    pub page: Option<i64>,
    #[serde(rename = "perPage")]
    pub per_page: Option<i64>,
}

fn forbidden() -> ApiError {
    ApiError(AppError::Forbidden(
        "you are not allowed to perform this action".into(),
    ))
}

async fn list(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    CurrentAuth(auth): CurrentAuth,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let collection = load_collection(&app, &collection_name).await?;
    let ctx = RequestContext { auth, data: None };

    let outcome = evaluate_rule(&collection.list_rule, &collection, app.db.backend, &ctx, 0)?;
    let rule_filter = match outcome {
        RuleOutcome::DenyAll => return Err(forbidden()),
        RuleOutcome::AllowAll => None,
        RuleOutcome::Filtered(f) => Some(f),
    };

    let result = records::list_records(
        &app.db,
        &collection,
        &ctx,
        rule_filter,
        ListParams {
            filter: q.filter.as_deref(),
            sort: q.sort.as_deref(),
            page: q.page.unwrap_or(1),
            per_page: q.per_page.unwrap_or(30),
        },
    )
    .await?;

    Ok(Json(serde_json::json!({
        "page": result.page,
        "perPage": result.per_page,
        "totalItems": result.total_items,
        "totalPages": result.total_pages,
        "items": result.items,
    })))
}

async fn view(
    State(app): State<AppState>,
    Path((collection_name, id)): Path<(String, String)>,
    CurrentAuth(auth): CurrentAuth,
) -> ApiResult<Json<Value>> {
    let collection = load_collection(&app, &collection_name).await?;
    let ctx = RequestContext { auth, data: None };

    let outcome = evaluate_rule(&collection.view_rule, &collection, app.db.backend, &ctx, 0)?;
    let rule_filter = match outcome {
        RuleOutcome::DenyAll => return Err(forbidden()),
        RuleOutcome::AllowAll => None,
        RuleOutcome::Filtered(f) => Some(f),
    };

    let record = records::get_record(&app.db, &collection, &id, rule_filter).await?;
    Ok(Json(record))
}

/// Merge uploaded file parts into the fields map, respecting each field's
/// single/multiple cardinality. Replaces (does not append to) any existing
/// value — see `payload.rs` module docs for the tradeoff.
fn merge_uploaded_filenames(
    fields: &mut Map<String, Value>,
    collection: &Collection,
    uploads: &[(Upload, String)],
) {
    let mut by_field: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for (upload, stored_name) in uploads {
        by_field
            .entry(upload.field.clone())
            .or_default()
            .push(stored_name.clone());
    }
    for (field_name, names) in by_field {
        let multiple = collection
            .field(&field_name)
            .map(|f| f.field_type.supports_multiple() && f.options.multiple.unwrap_or(false))
            .unwrap_or(false);
        if multiple {
            fields.insert(
                field_name,
                Value::Array(names.into_iter().map(Value::String).collect()),
            );
        } else {
            fields.insert(field_name, Value::String(names.into_iter().last().unwrap()));
        }
    }
}

/// Collect the stored filenames currently referenced by a record's file
/// fields, for best-effort cleanup when they're replaced or the record is
/// deleted.
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

async fn delete_stored_files(
    app: &AppState,
    collection: &Collection,
    record_id: &str,
    files: Vec<(String, Vec<String>)>,
) {
    for (_, names) in files {
        for name in names {
            let key = file_key(&collection.id, record_id, &name);
            if let Err(e) = app.storage.delete(&key).await {
                tracing::warn!(key, error = %e, "failed to clean up orphaned file");
            }
        }
    }
}

async fn create(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    CurrentAuth(auth): CurrentAuth,
    req: Request<Body>,
) -> ApiResult<Json<Value>> {
    let collection = load_collection(&app, &collection_name).await?;
    let payload = parse_payload(&collection, &app, req).await?;
    let mut fields = payload.fields;

    let ctx = RequestContext {
        auth: auth.clone(),
        data: Some(fields.clone()),
    };
    let allowed = evaluate_create_rule(&app.db, &collection.create_rule, &collection, &ctx).await?;
    if !allowed {
        return Err(forbidden());
    }

    let id = new_id();
    let mut stored = Vec::new();
    for upload in payload.uploads {
        let stored_name = unique_filename(&upload.filename);
        let key = file_key(&collection.id, &id, &stored_name);
        app.storage.put(&key, upload.bytes.clone()).await?;
        stored.push((upload, stored_name));
    }
    merge_uploaded_filenames(&mut fields, &collection, &stored);

    let fields = crate::auth_fields::prepare_auth_create(&collection, fields)?;

    let record = records::create_record_with_id(&app.db, &collection, id, fields).await?;
    app.realtime
        .publish(&collection.name, "create", &record)
        .await;
    Ok(Json(record))
}

async fn update(
    State(app): State<AppState>,
    Path((collection_name, id)): Path<(String, String)>,
    CurrentAuth(auth): CurrentAuth,
    req: Request<Body>,
) -> ApiResult<Json<Value>> {
    let collection = load_collection(&app, &collection_name).await?;
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
    )?;
    let rule_filter = match outcome {
        RuleOutcome::DenyAll => return Err(forbidden()),
        RuleOutcome::AllowAll => None,
        RuleOutcome::Filtered(f) => Some(f),
    };
    // Fetching through the update rule both enforces it (404s if denied)
    // and gives us the pre-update file field values for cleanup below.
    let previous = records::get_record(&app.db, &collection, &id, rule_filter).await?;

    let payload = parse_payload(&collection, &app, req).await?;
    let mut fields = payload.fields;

    let mut stored = Vec::new();
    for upload in payload.uploads {
        let stored_name = unique_filename(&upload.filename);
        let key = file_key(&collection.id, &id, &stored_name);
        app.storage.put(&key, upload.bytes.clone()).await?;
        stored.push((upload, stored_name));
    }
    let replaced_fields: Vec<String> = stored.iter().map(|(u, _)| u.field.clone()).collect();
    merge_uploaded_filenames(&mut fields, &collection, &stored);

    let fields = crate::auth_fields::prepare_auth_update(&collection, fields)?;

    let record = records::update_record(&app.db, &collection, &id, fields).await?;

    if !replaced_fields.is_empty() {
        let old_files: Vec<(String, Vec<String>)> = file_field_values(&collection, &previous)
            .into_iter()
            .filter(|(name, _)| replaced_fields.contains(name))
            .collect();
        delete_stored_files(&app, &collection, &id, old_files).await;
    }

    app.realtime
        .publish(&collection.name, "update", &record)
        .await;
    Ok(Json(record))
}

async fn remove(
    State(app): State<AppState>,
    Path((collection_name, id)): Path<(String, String)>,
    CurrentAuth(auth): CurrentAuth,
) -> ApiResult<axum::http::StatusCode> {
    let collection = load_collection(&app, &collection_name).await?;
    let ctx = RequestContext { auth, data: None };
    let outcome = evaluate_rule(
        &collection.delete_rule,
        &collection,
        app.db.backend,
        &ctx,
        0,
    )?;
    let rule_filter = match outcome {
        RuleOutcome::DenyAll => return Err(forbidden()),
        RuleOutcome::AllowAll => None,
        RuleOutcome::Filtered(f) => Some(f),
    };

    let record = records::get_record(&app.db, &collection, &id, rule_filter).await?;
    records::delete_record(&app.db, &collection, &id).await?;

    let files = file_field_values(&collection, &record);
    delete_stored_files(&app, &collection, &id, files).await;

    app.realtime
        .publish(&collection.name, "delete", &record)
        .await;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
