use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use cratebase_auth::{
    generate_otp, hash_otp, hash_password, issue_action_token, issue_token, verify_password,
    verify_token, TokenKind,
};
use cratebase_core::AppError;
use cratebase_db::{admins, external_auths, otp, records};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::SmartIpKeyExtractor;
use tower_governor::GovernorLayer;

use crate::extract::CurrentAuth;
use crate::helpers::load_collection;
use crate::http_error::{ApiError, ApiResult};
use crate::mail::send_template;
use crate::state::AppState;

/// `rate_limit_enabled` governs only the password-login endpoints — the
/// actual credential-stuffing/brute-force vector. Refresh endpoints are
/// left unlimited: a real Flutter/mobile client legitimately calls
/// `auth-refresh` far more often than it calls `auth-with-password` (e.g.
/// on every app foreground), so bucketing them under the same strict
/// limit would false-positive-lock out real users rather than attackers.
///
/// Keyed by `SmartIpKeyExtractor` (checks `X-Forwarded-For`/`X-Real-Ip`/
/// `Forwarded` before falling back to the raw peer address): the common
/// deployment in [ARCHITECTURE.md](../../../../ARCHITECTURE.md) puts
/// Cratebase behind a reverse proxy for TLS, and under `PeerIpKeyExtractor`
/// every request would appear to come from the proxy's own IP — a single
/// shared bucket for every real client. This trusts those headers, which
/// is the trade-off every deployment without an explicit trusted-proxy
/// allowlist makes (tracked in ROADMAP.md).
pub fn router(rate_limit_enabled: bool) -> Router<AppState> {
    // Everything that triggers an outbound email (password login attempts,
    // and the three `request-*` flows below) shares one rate limit — each
    // is either a credential-stuffing vector or a mail-bombing vector
    // (spamming someone's inbox with reset/verification links), so the
    // same per-IP throttle protects both. `confirm-*` endpoints don't:
    // they consume a token whose entropy makes brute-forcing infeasible,
    // not a password or an inbox.
    let limited_router = Router::new()
        .route("/admins/auth-with-password", post(admin_login))
        .route(
            "/collections/{collection}/auth-with-password",
            post(record_login),
        )
        .route(
            "/collections/{collection}/auth-with-oauth2",
            post(auth_with_oauth2),
        )
        .route(
            "/collections/{collection}/request-verification",
            post(request_verification),
        )
        .route(
            "/collections/{collection}/request-password-reset",
            post(request_password_reset),
        )
        .route(
            "/collections/{collection}/request-email-change",
            post(request_email_change),
        );
    let limited_router = if rate_limit_enabled {
        let governor_conf = GovernorConfigBuilder::default()
            .key_extractor(SmartIpKeyExtractor)
            .per_second(3)
            .burst_size(8)
            .finish()
            .expect("static rate limit config is always valid");
        limited_router.layer(GovernorLayer::new(Arc::new(governor_conf)))
    } else {
        limited_router
    };

    let unlimited_router = Router::new()
        .route(
            "/collections/{collection}/auth-methods",
            get(auth_methods),
        )
        .route("/admins/auth-refresh", post(admin_refresh))
        .route(
            "/collections/{collection}/auth-refresh",
            post(record_refresh),
        )
        .route(
            "/collections/{collection}/confirm-verification",
            post(confirm_verification),
        )
        .route(
            "/collections/{collection}/confirm-password-reset",
            post(confirm_password_reset),
        )
        .route(
            "/collections/{collection}/confirm-email-change",
            post(confirm_email_change),
        );

    limited_router.merge(unlimited_router)
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

    let Some((id, password_hash, verified)) =
        records::find_auth_credentials(&app.db, &collection, &body.identity).await?
    else {
        return Err(invalid_credentials());
    };
    if !verify_password(&body.password, &password_hash) {
        return Err(invalid_credentials());
    }
    if collection.auth_options.require_email_verification.unwrap_or(false) && !verified {
        return Err(ApiError(AppError::Forbidden(
            "please verify your email before signing in".into(),
        )));
    }

    // Second factor: park the password success behind a pending marker
    // instead of issuing a real session token straight away. The marker
    // and the OTP it names are minted together (same TTL) so one always
    // outlives the other by exactly zero seconds — `/mfa/confirm` is the
    // only way to turn this into a usable token.
    if collection.auth_options.mfa_required() {
        let ttl = app.config.otp_token_ttl_seconds;
        let code = generate_otp();
        otp::create(&app.db, &collection.id, &id, &hash_otp(&code), ttl).await?;
        let mfa_id = issue_token(&id, TokenKind::Mfa, &collection.id, &app.config.auth_secret, ttl)
            .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
        if collection.auth_options.identity_is_email() {
            let _ = send_template(
                &app,
                &body.identity,
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
        return Ok(Json(json!({ "mfaId": mfa_id, "mfaRequired": true })));
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

#[derive(Deserialize)]
struct EmailOnly {
    email: String,
}

#[derive(Deserialize)]
struct TokenOnly {
    token: String,
}

#[derive(Deserialize)]
struct ConfirmPasswordReset {
    token: String,
    password: String,
    #[serde(rename = "passwordConfirm")]
    password_confirm: String,
}

#[derive(Deserialize)]
struct NewEmailOnly {
    #[serde(rename = "newEmail")]
    new_email: String,
}

fn invalid_token() -> ApiError {
    ApiError(AppError::BadRequest("invalid or expired token".into()))
}

fn looks_like_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

/// `"3600"` -> `"1 hour"`, `"120"` -> `"2 minutes"`, otherwise the raw
/// second count — for the "this link expires in ..." line in emails.
fn format_ttl(seconds: i64) -> String {
    if seconds >= 3600 && seconds % 3600 == 0 {
        let h = seconds / 3600;
        format!("{h} hour{}", if h == 1 { "" } else { "s" })
    } else if seconds >= 60 && seconds % 60 == 0 {
        let m = seconds / 60;
        format!("{m} minute{}", if m == 1 { "" } else { "s" })
    } else {
        format!("{seconds} seconds")
    }
}

fn not_email_identity() -> ApiError {
    ApiError(AppError::BadRequest(
        "this collection's identity field isn't an email address".into(),
    ))
}

/// `POST /collections/{c}/request-verification` — always responds 204
/// regardless of whether `email` matches a record, so the response can't
/// be used to enumerate registered accounts.
async fn request_verification(
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
        let ttl = app.config.verification_token_ttl_seconds;
        if let Ok(token) = issue_action_token(
            &id,
            TokenKind::VerifyEmail,
            &collection.id,
            None,
            &app.config.auth_secret,
            ttl,
        ) {
            let action_url = format!("{}/verify-email?token={token}", app.config.public_app_url);
            let _ = send_template(
                &app,
                &body.email,
                &format!("Confirm your email for {}", app.config.mail_from_name),
                "verification.html",
                &[
                    ("appName", &app.config.mail_from_name),
                    ("actionUrl", &action_url),
                    ("expiresIn", &format_ttl(ttl)),
                ],
            )
            .await;
        }
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `POST /collections/{c}/confirm-verification`
async fn confirm_verification(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    Json(body): Json<TokenOnly>,
) -> ApiResult<axum::http::StatusCode> {
    let collection = load_collection(&app, &collection_name).await?;
    let claims = verify_token(&body.token, &app.config.auth_secret).map_err(|_| invalid_token())?;
    if claims.kind != TokenKind::VerifyEmail || claims.collection_id != collection.id {
        return Err(invalid_token());
    }
    records::set_verified(&app.db, &collection, &claims.sub).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `POST /collections/{c}/request-password-reset` — same no-enumeration
/// shape as `request_verification`.
async fn request_password_reset(
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
        let ttl = app.config.password_reset_token_ttl_seconds;
        if let Ok(token) = issue_action_token(
            &id,
            TokenKind::ResetPassword,
            &collection.id,
            None,
            &app.config.auth_secret,
            ttl,
        ) {
            let action_url = format!("{}/reset-password?token={token}", app.config.public_app_url);
            let _ = send_template(
                &app,
                &body.email,
                &format!("Reset your {} password", app.config.mail_from_name),
                "password-reset.html",
                &[
                    ("appName", &app.config.mail_from_name),
                    ("actionUrl", &action_url),
                    ("expiresIn", &format_ttl(ttl)),
                ],
            )
            .await;
        }
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `POST /collections/{c}/confirm-password-reset`
async fn confirm_password_reset(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    Json(body): Json<ConfirmPasswordReset>,
) -> ApiResult<axum::http::StatusCode> {
    let collection = load_collection(&app, &collection_name).await?;
    let claims = verify_token(&body.token, &app.config.auth_secret).map_err(|_| invalid_token())?;
    if claims.kind != TokenKind::ResetPassword || claims.collection_id != collection.id {
        return Err(invalid_token());
    }
    let min_len = collection.auth_options.min_password_length.unwrap_or(8) as usize;
    if body.password.chars().count() < min_len {
        return Err(ApiError(AppError::BadRequest(format!(
            "password must be at least {min_len} characters"
        ))));
    }
    if body.password != body.password_confirm {
        return Err(ApiError(AppError::BadRequest(
            "passwords do not match".into(),
        )));
    }
    let hash = hash_password(&body.password).map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
    let mut data = serde_json::Map::new();
    data.insert("password_hash".into(), json!(hash));
    records::update_record(&app.db, &collection, &claims.sub, data).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `POST /collections/{c}/request-email-change` — requires a session
/// token for the record whose email is changing; the confirmation link
/// goes to the *new* address to prove ownership of it before the switch
/// takes effect.
async fn request_email_change(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    CurrentAuth(auth): CurrentAuth,
    Json(body): Json<NewEmailOnly>,
) -> ApiResult<axum::http::StatusCode> {
    let collection = load_collection(&app, &collection_name).await?;
    let ctx =
        auth.ok_or_else(|| ApiError(AppError::Unauthorized("missing or invalid token".into())))?;
    if ctx.is_superuser || ctx.collection_id != collection.id {
        return Err(ApiError(AppError::Unauthorized(
            "missing or invalid token".into(),
        )));
    }
    if !collection.auth_options.identity_is_email() {
        return Err(not_email_identity());
    }
    if !looks_like_email(&body.new_email) {
        return Err(ApiError(AppError::BadRequest(
            "not a valid email address".into(),
        )));
    }

    let ttl = app.config.email_change_token_ttl_seconds;
    let token = issue_action_token(
        &ctx.id,
        TokenKind::ChangeEmail,
        &collection.id,
        Some(&body.new_email),
        &app.config.auth_secret,
        ttl,
    )
    .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
    let action_url = format!("{}/confirm-email-change?token={token}", app.config.public_app_url);
    let _ = send_template(
        &app,
        &body.new_email,
        &format!("Confirm your new email for {}", app.config.mail_from_name),
        "email-change.html",
        &[
            ("appName", &app.config.mail_from_name),
            ("newEmail", &body.new_email),
            ("actionUrl", &action_url),
            ("expiresIn", &format_ttl(ttl)),
        ],
    )
    .await;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `POST /collections/{c}/confirm-email-change`
async fn confirm_email_change(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    Json(body): Json<TokenOnly>,
) -> ApiResult<axum::http::StatusCode> {
    let collection = load_collection(&app, &collection_name).await?;
    let claims = verify_token(&body.token, &app.config.auth_secret).map_err(|_| invalid_token())?;
    if claims.kind != TokenKind::ChangeEmail || claims.collection_id != collection.id {
        return Err(invalid_token());
    }
    let Some(new_email) = claims.new_email else {
        return Err(invalid_token());
    };
    let identity = collection.auth_options.identity_field();
    let mut data = serde_json::Map::new();
    data.insert(identity.to_string(), json!(new_email));
    records::update_record(&app.db, &collection, &claims.sub, data).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AuthMethodsQuery {
    #[serde(rename = "redirectUri")]
    redirect_uri: Option<String>,
}

/// `GET /collections/{c}/auth-methods?redirectUri=...` — lists what a
/// client can authenticate with. `redirectUri` (the client's own OAuth2
/// callback — a web page or a mobile deep link) is baked into each
/// provider's `authUrl` if supplied, so the client can `open()` the URL
/// directly with no further assembly.
async fn auth_methods(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    Query(q): Query<AuthMethodsQuery>,
) -> ApiResult<Json<Value>> {
    let collection = load_collection(&app, &collection_name).await?;
    if !collection.is_auth() {
        return Err(ApiError(AppError::BadRequest(format!(
            "'{collection_name}' is not an auth collection"
        ))));
    }
    let providers: Vec<Value> = app
        .config
        .oauth_providers
        .iter()
        .map(|p| {
            let mut url = format!(
                "{}?client_id={}&response_type=code&scope={}",
                p.auth_url,
                urlencoding::encode(&p.client_id),
                urlencoding::encode(p.scope)
            );
            if let Some(redirect) = &q.redirect_uri {
                url.push_str(&format!("&redirect_uri={}", urlencoding::encode(redirect)));
            }
            json!({ "name": p.name, "authUrl": url })
        })
        .collect();
    Ok(Json(json!({
        "password": true,
        "oauth2": { "enabled": !providers.is_empty(), "providers": providers },
    })))
}

#[derive(Deserialize)]
struct OAuth2Login {
    provider: String,
    code: String,
    #[serde(rename = "redirectUri")]
    redirect_uri: String,
}

/// `POST /collections/{c}/auth-with-oauth2` — exchanges `code` for the
/// provider's access token, fetches the provider's profile, and either
/// signs in the record already linked to that provider identity, links
/// an existing record with a matching email, or creates a new one (with a
/// random unusable password — set later via `confirm-password-reset` if
/// the person ever wants to add password login too). The email is
/// pre-verified: the provider already proved ownership of it.
async fn auth_with_oauth2(
    State(app): State<AppState>,
    Path(collection_name): Path<String>,
    Json(body): Json<OAuth2Login>,
) -> ApiResult<Json<Value>> {
    let collection = load_collection(&app, &collection_name).await?;
    if !collection.is_auth() {
        return Err(ApiError(AppError::BadRequest(format!(
            "'{collection_name}' is not an auth collection"
        ))));
    }
    if !collection.auth_options.identity_is_email() {
        return Err(not_email_identity());
    }
    let provider = app
        .config
        .oauth_providers
        .iter()
        .find(|p| p.name == body.provider)
        .ok_or_else(|| {
            ApiError(AppError::BadRequest(format!(
                "unknown or unconfigured provider '{}'",
                body.provider
            )))
        })?;

    let access_token = crate::oauth2::exchange_code(provider, &body.code, &body.redirect_uri)
        .await
        .map_err(|e| ApiError(AppError::BadRequest(format!("oauth2 exchange failed: {e}"))))?;
    let external_user = crate::oauth2::fetch_user(provider, &access_token)
        .await
        .map_err(|e| ApiError(AppError::BadRequest(format!("oauth2 profile lookup failed: {e}"))))?;

    let record_id = match external_auths::find_linked_record(
        &app.db,
        &collection.id,
        provider.name,
        &external_user.provider_user_id,
    )
    .await?
    {
        Some(id) => id,
        None => {
            let email = external_user
                .email
                .ok_or_else(|| ApiError(AppError::BadRequest("provider account has no email address".into())))?;
            let id = match records::find_auth_credentials(&app.db, &collection, &email).await? {
                Some((id, _, _)) => id,
                None => {
                    let random_password = cratebase_core::new_id();
                    let hash = hash_password(&random_password)
                        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
                    let mut fields = serde_json::Map::new();
                    fields.insert("email".into(), json!(email));
                    fields.insert("password_hash".into(), json!(hash));
                    let record = records::create_record(&app.db, &collection, fields).await?;
                    let new_id = record["id"].as_str().unwrap_or_default().to_string();
                    // The provider already proved ownership of this
                    // email; don't also make them click a verification
                    // link for an account they can't set a password on
                    // yet anyway.
                    records::set_verified(&app.db, &collection, &new_id).await?;
                    new_id
                }
            };
            external_auths::link(&app.db, &collection.id, &id, provider.name, &external_user.provider_user_id).await?;
            id
        }
    };

    let record = records::get_record(&app.db, &collection, &record_id, None).await?;
    let token = issue_token(
        &record_id,
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
