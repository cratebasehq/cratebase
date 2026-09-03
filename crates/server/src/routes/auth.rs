use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use cratebase_auth::{issue_token, verify_password, TokenKind};
use cratebase_core::AppError;
use cratebase_db::{admins, records};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::extract::CurrentAuth;
use crate::helpers::load_collection;
use crate::http_error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admins/auth-with-password", post(admin_login))
        .route("/admins/auth-refresh", post(admin_refresh))
        .route(
            "/collections/{collection}/auth-with-password",
            post(record_login),
        )
        .route(
            "/collections/{collection}/auth-refresh",
            post(record_refresh),
        )
}

#[derive(Deserialize)]
struct AdminPasswordLogin {
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct RecordPasswordLogin {
    identity: String,
    password: String,
}

fn invalid_credentials() -> ApiError {
    ApiError(AppError::Unauthorized("invalid email or password".into()))
}

async fn admin_login(
    State(app): State<AppState>,
    Json(body): Json<AdminPasswordLogin>,
) -> ApiResult<Json<Value>> {
    let admin = admins::get_admin_by_email(&app.db, &body.email)
        .await
        .map_err(|_| invalid_credentials())?;
    if !verify_password(&body.password, &admin.password_hash) {
        return Err(invalid_credentials());
    }
    let token = issue_token(
        &admin.id,
        TokenKind::Admin,
        "",
        &app.config.auth_secret,
        app.config.admin_token_ttl_seconds,
    )
    .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
    Ok(Json(json!({ "token": token, "admin": admin })))
}

async fn admin_refresh(
    State(app): State<AppState>,
    CurrentAuth(auth): CurrentAuth,
) -> ApiResult<Json<Value>> {
    let ctx =
        auth.ok_or_else(|| ApiError(AppError::Unauthorized("missing or invalid token".into())))?;
    if !ctx.is_superuser {
        return Err(ApiError(AppError::Unauthorized(
            "missing or invalid token".into(),
        )));
    }
    let admin = admins::get_admin_by_id(&app.db, &ctx.id).await?;
    let token = issue_token(
        &admin.id,
        TokenKind::Admin,
        "",
        &app.config.auth_secret,
        app.config.admin_token_ttl_seconds,
    )
    .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
    Ok(Json(json!({ "token": token, "admin": admin })))
}

async fn record_login(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    Json(body): Json<RecordPasswordLogin>,
) -> ApiResult<Json<Value>> {
    let collection = load_collection(&app, &collection_name).await?;
    if !collection.is_auth() {
        return Err(ApiError(AppError::BadRequest(format!(
            "'{collection_name}' is not an auth collection"
        ))));
    }

    let Some((id, password_hash)) =
        records::find_auth_credentials(&app.db, &collection, &body.identity).await?
    else {
        return Err(invalid_credentials());
    };
    if !verify_password(&body.password, &password_hash) {
        return Err(invalid_credentials());
    }

    let record = records::get_record(&app.db, &collection, &id, None).await?;
    let token = issue_token(
        &id,
        TokenKind::Auth,
        &collection.id,
        &app.config.auth_secret,
        collection
            .auth_options
            .token_ttl_seconds
            .unwrap_or(app.config.auth_token_ttl_seconds),
    )
    .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    Ok(Json(json!({ "token": token, "record": record })))
}

async fn record_refresh(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    CurrentAuth(auth): CurrentAuth,
) -> ApiResult<Json<Value>> {
    let collection = load_collection(&app, &collection_name).await?;
    let ctx =
        auth.ok_or_else(|| ApiError(AppError::Unauthorized("missing or invalid token".into())))?;
    if ctx.is_superuser || ctx.collection_id != collection.id {
        return Err(ApiError(AppError::Unauthorized(
            "missing or invalid token".into(),
        )));
    }
    let record = records::get_record(&app.db, &collection, &ctx.id, None).await?;
    let token = issue_token(
        &ctx.id,
        TokenKind::Auth,
        &collection.id,
        &app.config.auth_secret,
        collection
            .auth_options
            .token_ttl_seconds
            .unwrap_or(app.config.auth_token_ttl_seconds),
    )
    .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
    Ok(Json(json!({ "token": token, "record": record })))
}
