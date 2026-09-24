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
//! Two things stand between "found this endpoint" and "owns the
//! instance":
//!
//! * **A token.** `App::init_setup_token` prints a one-time install token
//!   (and the dashboard URL carrying it) to the log at boot, mirroring
//!   PocketBase's own installer link. `POST /api/setup` requires it,
//!   compared in constant time ([`tokens_match`]) — without one, whoever
//!   finds a freshly deployed public instance first would otherwise get
//!   to create its owner account.
//! * **One write scope.** Checking "does a superuser exist" and inserting
//!   the row used to be two separate statements outside any transaction,
//!   so two concurrent calls could both pass the check and both insert —
//!   two owners on one fresh instance. Both now happen inside one
//!   `app.run_in_transaction` scope: SQLite already serializes every
//!   transaction behind its single writer lock (`App::run_in_transaction`'s
//!   doc), and on Postgres `pg_advisory_xact_lock` makes a second, still
//!   racing caller wait for the first to commit or roll back before it
//!   even runs its own check.
//!
//! This endpoint never mints a session token itself; the dashboard logs
//! in through the ordinary `auth-with-password` flow immediately after a
//! successful `POST`, so this file never needs to touch token-minting
//! code.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use cratebase_core::{codes, AppError, FieldError, SUPERUSER_ROLE_OWNER};
use cratebase_db::engine::Sql;
use cratebase_db::Executor;
use cratebase_filter::Dialect;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::app::{insert_superuser_row, App};
use crate::http_error::{ApiError, ApiJson, ApiResult};

const VALIDATION_FAILED: &str = "An error occurred while validating the submitted data.";
const ALREADY_SET_UP: &str = "A superuser already exists.";
const BAD_TOKEN: &str = "Missing or invalid setup token.";
/// Case-insensitive per HTTP convention; `HeaderMap::get` already is.
const TOKEN_HEADER: &str = "x-setup-token";

/// Arbitrary fixed key for `pg_advisory_xact_lock` while this file
/// re-checks and inserts (see the module doc). Any `i64` works as long as
/// nothing else in the codebase takes the same one — nothing does today.
/// Spells out as the bytes `b"cb_setup"`.
const SETUP_LOCK_KEY: i64 = 0x63625f7365747570;

pub fn router() -> Router<App> {
    Router::new()
        .route("/setup/status", get(status))
        .route("/setup", post(create_first_superuser))
}

async fn any_superuser_exists(executor: &dyn Executor) -> Result<bool, AppError> {
    let row = executor
        .query_one(r#"SELECT 1 AS "x" FROM "_superusers" LIMIT 1"#, &[])
        .await
        .map_err(AppError::from)?;
    Ok(row.is_some())
}

async fn status(State(app): State<App>) -> ApiResult<Json<Value>> {
    let needs_setup = !any_superuser_exists(app.db()).await?;
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
    /// Also accepted as the `X-Setup-Token` header; the header wins when
    /// both are present. The dashboard sends it in the body since that is
    /// the one request it already builds by hand.
    #[serde(default)]
    token: String,
}

/// Constant-time comparison so a wrong guess can't be narrowed down one
/// byte at a time through response timing. Length is allowed to
/// short-circuit — same as `subtle`'s `ConstantTimeEq` for slices — the
/// token's bytes are the secret, not how many of them there are.
fn tokens_match(provided: &str, expected: &str) -> bool {
    let (a, b) = (provided.as_bytes(), expected.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

async fn create_first_superuser(
    State(app): State<App>,
    headers: HeaderMap,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    if any_superuser_exists(app.db()).await? {
        return Err(ApiError::forbidden(ALREADY_SET_UP));
    }

    let body: SetupBody = serde_json::from_value(raw)
        .map_err(|_| ApiError::bad_request(AppError::DEFAULT_BAD_REQUEST))?;

    let header_token = headers.get(TOKEN_HEADER).and_then(|v| v.to_str().ok());
    let provided = header_token.unwrap_or(body.token.as_str());
    // `app.setup_token()` is `None` both before any superuser ever
    // existed... and after one was deleted without a restart (see
    // `App::init_setup_token`'s doc) — either way there is no token that
    // can satisfy this, so setup stays closed until the next boot.
    match app.setup_token() {
        Some(expected) if tokens_match(provided, expected) => {}
        _ => return Err(ApiError::forbidden(BAD_TOKEN)),
    }

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

    // The re-check and the insert run inside one write scope — see the
    // module doc for why the old two-statement version could double-book
    // the owner. `body` is done being read by this point, so its fields
    // move straight into the closure instead of being borrowed out of it.
    let SetupBody {
        email, password, ..
    } = body;
    app.run_in_transaction(|tx| async move {
        if tx.dialect() == Dialect::Postgres {
            // SQLite needs nothing extra: `begin()` already holds the
            // single writer lock for this whole scope (see
            // `App::run_in_transaction`'s doc), so a second concurrent
            // call simply can't get a transaction open until this one
            // finishes. Postgres has no such single writer, and an empty
            // table gives `SELECT ... FOR UPDATE` nothing to lock (see
            // `routes::records::count_owners`, which relies on exactly
            // that for a non-empty table) — an advisory lock is the only
            // thing here to actually wait on.
            tx.execute(
                "SELECT pg_advisory_xact_lock($1)",
                &[Sql::from(SETUP_LOCK_KEY)],
            )
            .await?;
        }
        if any_superuser_exists(&tx).await? {
            return Err(AppError::forbidden(ALREADY_SET_UP));
        }
        insert_superuser_row(&tx, &email, &password, SUPERUSER_ROLE_OWNER).await?;
        Ok(())
    })
    .await?;

    Ok(Json(json!({})))
}
