use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use cratebase_core::field::Field;
use cratebase_core::{new_id, now, AppError, AuthOptions, Collection, CollectionType};
use cratebase_db::collections;
use serde::Deserialize;

use crate::extract::RequireAdmin;
use crate::helpers::load_collection;
use crate::http_error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/collections", get(list).post(create))
        .route("/collections/{id}", get(view).patch(update).delete(remove))
}

/// Editable fields of a collection. `id`/`created`/`updated` are always
/// server-managed.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionInput {
    pub name: String,
    #[serde(rename = "type")]
    pub collection_type: CollectionType,
    #[serde(default)]
    pub schema: Vec<Field>,
    #[serde(default)]
    pub list_rule: Option<String>,
    #[serde(default)]
    pub view_rule: Option<String>,
    #[serde(default)]
    pub create_rule: Option<String>,
    #[serde(default)]
    pub update_rule: Option<String>,
    #[serde(default)]
    pub delete_rule: Option<String>,
    #[serde(default)]
    pub auth_options: AuthOptions,
}

fn validate_input(input: &CollectionInput) -> ApiResult<()> {
    if !cratebase_core::field::is_valid_identifier(&input.name) {
        return Err(ApiError(AppError::BadRequest(format!(
            "'{}' is not a valid collection name (letters, digits, underscore; must not start with a digit)",
            input.name
        ))));
    }
    if input.collection_type == CollectionType::Auth {
        let identity_field = input.auth_options.identity_field();
        if !cratebase_core::field::is_valid_identifier(identity_field)
            || matches!(identity_field, "password" | "password_hash")
        {
            return Err(ApiError(AppError::BadRequest(format!(
                "'{identity_field}' is not a valid identity field name"
            ))));
        }
    }
    for field in &input.schema {
        if !cratebase_core::field::is_valid_identifier(&field.name) {
            return Err(ApiError(AppError::BadRequest(format!(
                "'{}' is not a valid field name",
                field.name
            ))));
        }
        let reserved_for_auth = input.collection_type == CollectionType::Auth
            && (field.name == input.auth_options.identity_field()
                || matches!(field.name.as_str(), "password" | "password_hash"));
        if cratebase_core::RESERVED_FIELD_NAMES.contains(&field.name.as_str()) || reserved_for_auth
        {
            return Err(ApiError(AppError::BadRequest(format!(
                "'{}' is a reserved field name",
                field.name
            ))));
        }
    }
    Ok(())
}

async fn list(
    State(app): State<AppState>,
    _admin: RequireAdmin,
) -> ApiResult<Json<Vec<Collection>>> {
    Ok(Json(collections::list_collections(&app.db).await?))
}

async fn view(
    State(app): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<String>,
) -> ApiResult<Json<Collection>> {
    Ok(Json(load_collection(&app, &id).await?))
}

async fn create(
    State(app): State<AppState>,
    _admin: RequireAdmin,
    Json(input): Json<CollectionInput>,
) -> ApiResult<Json<Collection>> {
    validate_input(&input)?;
    let ts = now();
    let collection = Collection {
        id: new_id(),
        name: input.name,
        collection_type: input.collection_type,
        schema: input.schema,
        list_rule: input.list_rule,
        view_rule: input.view_rule,
        create_rule: input.create_rule,
        update_rule: input.update_rule,
        delete_rule: input.delete_rule,
        auth_options: input.auth_options,
        view_query: None,
        created: ts.clone(),
        updated: ts,
    };
    collections::create_collection(&app.db, &collection).await?;
    Ok(Json(collection))
}

async fn update(
    State(app): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<String>,
    Json(input): Json<CollectionInput>,
) -> ApiResult<Json<Collection>> {
    validate_input(&input)?;
    let previous = load_collection(&app, &id).await?;
    let updated = Collection {
        id: previous.id.clone(),
        name: input.name,
        collection_type: previous.collection_type, // immutable after creation
        schema: input.schema,
        list_rule: input.list_rule,
        view_rule: input.view_rule,
        create_rule: input.create_rule,
        update_rule: input.update_rule,
        delete_rule: input.delete_rule,
        auth_options: input.auth_options,
        view_query: previous.view_query.clone(),
        created: previous.created.clone(),
        updated: now(),
    };
    collections::update_collection(&app.db, &previous, &updated).await?;
    Ok(Json(updated))
}

async fn remove(
    State(app): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    let collection = load_collection(&app, &id).await?;
    collections::delete_collection(&app.db, &collection).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
