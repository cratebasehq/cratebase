//! `GET /api/setup/status` / `POST /api/setup` — first-run superuser
//! creation.
//!
//! A fresh `cratebase serve` against an empty database has no superuser
//! and, until now, no way to create one short of dropping to a terminal
//! (`cratebase superuser create ...`). PocketBase's own dashboard instead
//! detects "no superuser exists yet" and renders an inline setup form —
//! this is that endpoint pair.
//!
//! `status` is a separate `GET` rather than folded into `/api/health`
//! because it has to be readable *before* any superuser exists and before
//! the caller has any credentials at all, while `/api/health`'s shape is
//! pinned by conformance tests for an already-configured instance.
//!
//! `POST /api/setup` re-checks "does a superuser exist" fresh on every
//! call rather than trusting a prior `status` read: another operator, a
//! concurrent browser tab, or the `superuser create` CLI could create the
//! first superuser in the gap between the dashboard's status check and
//! its form submission. Once any superuser exists, this endpoint is
//! permanently closed — it never mints a session token itself; the
//! dashboard logs in through the ordinary `auth-with-password` flow
//! immediately after a successful `POST`, so this file never needs to
//! touch token-minting code.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use cratebase_core::{codes, AppError, FieldError};
use cratebase_db::Executor;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::app::App;
use crate::http_error::{ApiError, ApiJson, ApiResult};

const VALIDATION_FAILED: &str = "An error occurred while validating the submitted data.";
const ALREADY_SET_UP: &str = "A superuser already exists.";

pub fn router() -> Router<App> {
    Router::new()
        .route("/setup/status", get(status))
        .route("/setup", post(create_first_superuser))
}

async fn any_superuser_exists(app: &App) -> ApiResult<bool> {
    let row = app
        .db()
        .query_one(r#"SELECT 1 AS "x" FROM "_superusers" LIMIT 1"#, &[])
        .await?;
    Ok(row.is_some())
}

async fn status(State(app): State<App>) -> ApiResult<Json<Value>> {
    let needs_setup = !any_superuser_exists(&app).await?;
    Ok(Json(json!({ "needsSetup": needs_setup })))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupBody {
    #[serde(default)]
    email: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    password_confirm: String,
}

async fn create_first_superuser(
    State(app): State<App>,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    if any_superuser_exists(&app).await? {
        return Err(ApiError::forbidden(ALREADY_SET_UP));
    }

    let body: SetupBody = serde_json::from_value(raw)
        .map_err(|_| ApiError::bad_request(AppError::DEFAULT_BAD_REQUEST))?;

    let mut errors: BTreeMap<String, FieldError> = Default::default();
    if body.email.trim().is_empty() {
        errors.insert(
            "email".into(),
            FieldError::new(codes::REQUIRED, "Cannot be blank."),
        );
    } else if !body.email.contains('@') {
        errors.insert(
            "email".into(),
            FieldError::new(codes::INVALID_EMAIL, "Must be a valid email address."),
        );
    }
    if body.password.chars().count() < 8 {
        errors.insert(
            "password".into(),
            FieldError::new(codes::MIN_TEXT, "Must be at least 8 characters."),
        );
    }
    if body.password != body.password_confirm {
        errors.insert(
            "passwordConfirm".into(),
            FieldError::new(codes::VALUES_MISMATCH, "Values don't match."),
        );
    }
    if !errors.is_empty() {
        return Err(ApiError(AppError::validation(VALIDATION_FAILED, errors)));
    }

    // Re-check immediately before the write, not just at the top of the
    // handler: the validation above has no `.await` that could race
    // against another setup call in a meaningful window, but the
    // create-time check is the one that actually has to be authoritative.
    if any_superuser_exists(&app).await? {
        return Err(ApiError::forbidden(ALREADY_SET_UP));
    }

    app.create_superuser(&body.email, &body.password).await?;
    Ok(Json(json!({})))
}
