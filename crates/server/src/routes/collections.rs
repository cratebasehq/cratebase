use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use cratebase_core::field::Field;
use cratebase_core::{new_id, now, AppError, AuthOptions, Collection, CollectionType};
use cratebase_db::collections;
use cratebase_db::resolver::{CollectionResolver, RequestContext};
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
    /// Raw SELECT statement backing a `View` collection. Required (and
    /// only meaningful) when `type` is `"view"`; ignored otherwise.
    #[serde(default)]
    pub view_query: Option<String>,
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
    if input.collection_type == CollectionType::View
        && input
            .view_query
            .as_deref()
            .is_none_or(|q| q.trim().is_empty())
    {
        return Err(ApiError(AppError::BadRequest(
            "view collections require a non-empty 'viewQuery'".into(),
        )));
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
        if field.field_type == cratebase_core::field::FieldType::Autodate
            && !field.options.on_create.unwrap_or(false)
            && !field.options.on_update.unwrap_or(false)
        {
            return Err(ApiError(AppError::BadRequest(format!(
                "'{}' is an autodate field but sets neither onCreate nor onUpdate",
                field.name
            ))));
        }
    }
    Ok(())
}

/// Parses and compiles every non-empty rule expression against a
/// draft `Collection` — the exact same resolver construction
/// `cratebase_db::resolver::evaluate_rule`/`evaluate_create_rule` use at
/// request time (including prefetching relation dot-notation targets via
/// `load_related_collections`, so a rule referencing `author.name` is
/// validated exactly as strictly as it will be enforced), run here with
/// an empty [`RequestContext`] (auth/data are only *values* substituted
/// during resolution — `@request.auth.*` and `@request.data.*` resolve
/// structurally to `Value::Null` when absent, never an error, so this
/// catches exactly the failures a real request would hit later: syntax
/// errors, unknown field names, and unknown/non-relation dot-notation
/// targets). Without this, a typo'd rule saved silently and only
/// surfaced as every request being denied with no indication why.
async fn validate_rules(collection: &Collection, db: &cratebase_db::Db) -> ApiResult<()> {
    let ctx = RequestContext::default();
    let rules: [(&str, &Option<String>, bool); 5] = [
        ("listRule", &collection.list_rule, false),
        ("viewRule", &collection.view_rule, false),
        ("createRule", &collection.create_rule, true),
        ("updateRule", &collection.update_rule, false),
        ("deleteRule", &collection.delete_rule, false),
    ];
    for (label, rule, use_data_for_fields) in rules {
        let Some(expr) = rule else { continue };
        if expr.trim().is_empty() {
            continue;
        }
        let related =
            cratebase_db::resolver::load_related_collections(db, collection, expr).await?;
        let resolver = CollectionResolver {
            collection,
            backend: db.backend,
            ctx: &ctx,
            use_data_for_fields,
            related,
        };
        if let Err(e) =
            cratebase_filter::parse_and_compile(expr, &resolver, db.backend.dialect(), 0)
        {
            return Err(ApiError(AppError::BadRequest(format!(
                "invalid {label}: {e}"
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
    let view_query = if input.collection_type == CollectionType::View {
        input.view_query.clone()
    } else {
        None
    };
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
        view_query,
        created: ts.clone(),
        updated: ts,
    };
    validate_rules(&collection, &app.db).await?;
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
    let view_query = if previous.collection_type == CollectionType::View {
        input.view_query.clone()
    } else {
        None
    };
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
        view_query,
        created: previous.created.clone(),
        updated: now(),
    };
    validate_rules(&updated, &app.db).await?;
    collections::update_collection(&app.db, &previous, &updated).await?;
    Ok(Json(updated))
}

/// Deletes the collection unless it's the `users` auth collection
/// `cratebase_db::system::ensure_default_collections` auto-provisions on
/// first boot — that only runs when `users` is *missing*, so deleting it
/// mid-run breaks sign-in until the next server restart recreates it.
/// The dashboard already hides this action for `users`
/// (`CollectionSettings`); guarded here too since the dashboard isn't
/// the only way to call this endpoint.
async fn remove(
    State(app): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    let collection = load_collection(&app, &id).await?;
    if collection.name == "users" {
        return Err(ApiError(AppError::BadRequest(
            "the built-in 'users' collection can't be deleted".into(),
        )));
    }
    collections::delete_collection(&app.db, &collection).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
