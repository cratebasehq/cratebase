//! One-time-code auth flows for `Auth`-typed collections: passwordless
//! OTP login (`request-otp`/`auth-with-otp`), and the second-factor
//! exchange for password logins on an `mfaRequired` collection
//! (`mfa/confirm`, paired with the MFA branch in `routes::auth::record_login`).
//! Shares the `_otp_codes` table (`cratebase_db::otp`) and hashing scheme
//! (`cratebase_auth::{generate_otp, hash_otp}`) between both flows — an
//! MFA second factor *is* an OTP login, just gated behind a password
//! first.

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use cratebase_auth::{generate_otp, hash_otp, issue_token, verify_token, TokenKind};
use cratebase_core::AppError;
use cratebase_db::{otp, records};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::helpers::load_collection;
use crate::http_error::{ApiError, ApiResult};
use crate::mail::send_template;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/collections/{collection}/request-otp",
            post(request_otp),
        )
        .route(
            "/collections/{collection}/auth-with-otp",
            post(auth_with_otp),
        )
        .route("/collections/{collection}/mfa/confirm", post(mfa_confirm))
}

fn not_email_identity() -> ApiError {
    ApiError(AppError::BadRequest(
        "otp login requires an auth collection with an email identity field".into(),
    ))
}

fn invalid_otp() -> ApiError {
    ApiError(AppError::Unauthorized("invalid or expired code".into()))
}

/// `"3600"` -> `"1 hour"`, `"120"` -> `"2 minutes"`, otherwise the raw
/// second count — mirrors `routes::auth::format_ttl` (private to that
/// module, so duplicated here rather than exported just for this).
fn format_ttl(seconds: i64) -> String {
    if seconds % 3600 == 0 && seconds >= 3600 {
        let hours = seconds / 3600;
        format!("{hours} hour{}", if hours == 1 { "" } else { "s" })
    } else if seconds % 60 == 0 && seconds >= 60 {
        let minutes = seconds / 60;
        format!("{minutes} minute{}", if minutes == 1 { "" } else { "s" })
    } else {
        format!("{seconds} seconds")
    }
}

/// Issues a real session token for `id`, exactly like a successful
/// password/OTP login — shared by `auth_with_otp` and `mfa_confirm` so
/// both endpoints produce an identical response shape to `auth-with-password`.
async fn issue_session(
    app: &AppState,
    collection: &cratebase_core::Collection,
    id: &str,
) -> ApiResult<Json<Value>> {
    let record = records::get_record(&app.db, collection, id, None).await?;
    let token = issue_token(
        id,
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

#[derive(Deserialize)]
struct EmailOnly {
    email: String,
}

/// `POST /collections/{c}/request-otp` — always responds 204 regardless of
/// whether `email` matches a record, so the response can't be used to
/// enumerate registered accounts (same shape as
/// `routes::auth::request_verification`).
async fn request_otp(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    Json(body): Json<EmailOnly>,
) -> ApiResult<axum::http::StatusCode> {
    let collection = load_collection(&app, &collection_name).await?;
    if !collection.is_auth() || !collection.auth_options.identity_is_email() {
        return Err(not_email_identity());
    }
    if let Ok(Some((id, _, _))) =
        records::find_auth_credentials(&app.db, &collection, &body.email).await
    {
        let ttl = app.config.otp_token_ttl_seconds;
        let code = generate_otp();
        if otp::create(&app.db, &collection.id, &id, &hash_otp(&code), ttl)
            .await
            .is_ok()
        {
            let _ = send_template(
                &app,
                &body.email,
                &format!("Your {} verification code", app.config.mail_from_name),
                "otp.html",
                &[
                    ("appName", &app.config.mail_from_name),
                    ("code", &code),
                    ("expiresIn", &format_ttl(ttl)),
                ],
            )
            .await;
        }
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct EmailOtp {
    email: String,
    otp: String,
}

/// `POST /collections/{c}/auth-with-otp` — passwordless login: exchanges
/// an identity + a code minted by `request-otp` for a real session token.
async fn auth_with_otp(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    Json(body): Json<EmailOtp>,
) -> ApiResult<Json<Value>> {
    let collection = load_collection(&app, &collection_name).await?;
    if !collection.is_auth() || !collection.auth_options.identity_is_email() {
        return Err(not_email_identity());
    }
    let Some((id, _, _)) = records::find_auth_credentials(&app.db, &collection, &body.email).await?
    else {
        return Err(invalid_otp());
    };
    let matched =
        otp::verify_and_consume(&app.db, &collection.id, &id, &hash_otp(&body.otp)).await?;
    if !matched {
        return Err(invalid_otp());
    }
    issue_session(&app, &collection, &id).await
}

#[derive(Deserialize)]
struct MfaConfirm {
    #[serde(rename = "mfaId")]
    mfa_id: String,
    otp: String,
}

/// `POST /collections/{c}/mfa/confirm` — the second half of an MFA login:
/// exchanges the pending marker `record_login` returned plus the emailed
/// code for a real session token.
async fn mfa_confirm(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    Json(body): Json<MfaConfirm>,
) -> ApiResult<Json<Value>> {
    let collection = load_collection(&app, &collection_name).await?;
    let claims =
        verify_token(&body.mfa_id, &app.config.auth_secret).map_err(|_| invalid_otp())?;
    if claims.kind != TokenKind::Mfa || claims.collection_id != collection.id {
        return Err(invalid_otp());
    }
    let matched = otp::verify_and_consume(
        &app.db,
        &collection.id,
        &claims.sub,
        &hash_otp(&body.otp),
    )
    .await?;
    if !matched {
        return Err(invalid_otp());
    }
    issue_session(&app, &collection, &claims.sub).await
}
