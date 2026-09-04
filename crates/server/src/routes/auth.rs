//! `/api/collections/{collection}/auth-*` — password, OTP, MFA,
//! impersonation, verification, password reset and email change.
//!
//! Superusers are not special here: `_superusers` is an ordinary auth
//! collection, so `POST /api/collections/_superusers/auth-with-password`
//! goes through exactly the same code as any other login.
//!
//! # Tokens
//!
//! A session token is signed with `app secret + record.tokenKey +
//! authToken.secret`. Nothing is stored server-side; rotating a record's
//! `tokenKey` (which every password change does) invalidates every
//! outstanding session for that record. The claims are PocketBase's
//! exactly — `{collectionId, exp, id, refreshable, type}` — because the
//! SDK reads `refreshable` off the payload. Verification / password-reset /
//! email-change tokens are the same JWT machinery with a different `type`
//! and an `email`/`newEmail` extra claim instead of `refreshable`, which
//! is why they double as their own one-shot, single-purpose credential:
//! rotating `tokenKey` (done by every password/email change) invalidates
//! them exactly like a session token, so "the token is single-use" falls
//! out for free rather than needing a server-side consumption table.
//!
//! # Statuses that are not what you would guess
//!
//! * a disabled password method is **403**, not 400;
//! * an auth endpoint on a *base* collection is `404 "Missing or invalid
//!   auth collection context."`, while `auth-methods` on a collection
//!   that does not exist at all is `404 "Missing or invalid collection
//!   context."` — different wording for a different mistake;
//! * a failing `authRule` is **403**, a wrong password a flat
//!   `400 "Failed to authenticate."` with no `data`;
//! * `request-verification` / `request-password-reset` /
//!   `request-email-change` **always** answer `204`, even for an unknown
//!   address — anything else would let a caller enumerate accounts;
//! * a pending MFA challenge is `401 {"mfaId": "..."}` with no `status`,
//!   `message` or `data` at all — deliberately not the usual error
//!   envelope, so the SDK's generic error handling never renders it as a
//!   normal failure;
//! * `confirm-email-change` needs no `Authorization` header: the record
//!   it acts on comes from the token itself (so the link works from a
//!   fresh browser), and `password` is checked against *that* record —
//!   a token that cannot even be decoded means there is no record to
//!   check a password against, so both `token` and `password` fail
//!   together.
//!
//! W4b-2: `auth-with-oauth2` is not implemented (no provider wiring yet).
//! External auths need no dedicated routes: the SDK's
//! `listExternalAuths`/`unlinkExternalAuth` are thin wrappers around the
//! generic `_externalAuths` collection CRUD, which already works because
//! it is an ordinary (if system) collection.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cratebase_core::{codes, AppError, Collection, FieldError, Record};
use cratebase_db::records;
use cratebase_db::Executor;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::events::{collection_tags, MailerEvent, MailerRecordEvent, RecordRequestEvent};
use crate::extract::{Auth, MaybeAuth, RequestInfo};
use crate::hooks::{Hook, HookResult, Hooks};
use crate::http_error::{ApiError, ApiJson, ApiResult};
use crate::routes::common;

/// PocketBase's deliberately uninformative login failure.
const AUTH_FAILED: &str = "Failed to authenticate.";
const VALIDATION_FAILED: &str = "An error occurred while validating the submitted data.";
const PASSWORD_DISABLED: &str =
    "The collection is not configured to allow password authentication.";
const OTP_DISABLED: &str = "The collection is not configured to allow OTP authentication.";
const AUTH_RULE_FAILED: &str =
    "The request doesn't satisfy the collection requirements to authenticate.";
/// `auth-methods` on a name that is not a collection at all.
const MISSING_COLLECTION: &str = "Missing or invalid collection context.";
/// A `token` that could not even be parsed into claims of the right
/// shape. Distinct per flow because PocketBase's wording is.
const TOKEN_MISSING_EMAIL_CLAIM: &str = "validation_invalid_token_claims";
const TOKEN_INVALID_PAYLOAD: &str = "validation_invalid_token_payload";
const TOKEN_INVALID_PASSWORD: &str = "validation_invalid_password";
/// A `newEmail` equal to the record's current address.
const NOT_IN_INVALID: &str = "validation_not_in_invalid";

pub fn router() -> Router<App> {
    Router::new()
        .route(
            "/collections/{collection}/auth-with-password",
            post(auth_with_password),
        )
        .route("/collections/{collection}/auth-refresh", post(auth_refresh))
        .route("/collections/{collection}/auth-methods", get(auth_methods))
        .route(
            "/collections/{collection}/request-verification",
            post(request_verification),
        )
        .route(
            "/collections/{collection}/confirm-verification",
            post(confirm_verification),
        )
        .route(
            "/collections/{collection}/request-password-reset",
            post(request_password_reset),
        )
        .route(
            "/collections/{collection}/confirm-password-reset",
            post(confirm_password_reset),
        )
        .route(
            "/collections/{collection}/request-email-change",
            post(request_email_change),
        )
        .route(
            "/collections/{collection}/confirm-email-change",
            post(confirm_email_change),
        )
        .route("/collections/{collection}/request-otp", post(request_otp))
        .route(
            "/collections/{collection}/auth-with-otp",
            post(auth_with_otp),
        )
        .route(
            "/collections/{collection}/impersonate/{id}",
            post(impersonate),
        )
    // W4b-2: auth-with-oauth2 (no provider wiring yet).
}

// ------------------------------------------------------- auth-with-password

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PasswordBody {
    #[serde(default)]
    identity: String,
    #[serde(default)]
    password: String,
    /// Pins the lookup to one of `passwordAuth.identityFields` instead of
    /// trying them in order.
    #[serde(default)]
    identity_field: String,
    /// A pending MFA session id from a previous `401 {mfaId}` response.
    #[serde(default)]
    mfa_id: Option<String>,
}

async fn auth_with_password(
    State(app): State<App>,
    Path(name): Path<String>,
    info: RequestInfo,
    headers: HeaderMap,
    peer: crate::middleware::client_ip::PeerAddr,
    ApiJson(raw): ApiJson<Value>,
) -> Result<Response, ApiError> {
    let collection = common::auth_collection_of(&app, &name)?;
    if !collection.auth.password_auth.enabled {
        return Err(ApiError::forbidden(PASSWORD_DISABLED));
    }
    let body: PasswordBody = serde_json::from_value(raw.clone())
        .map_err(|_| ApiError::bad_request(AppError::DEFAULT_BAD_REQUEST))?;

    let mut errors: BTreeMap<String, FieldError> = Default::default();
    if body.identity.trim().is_empty() {
        errors.insert(
            "identity".into(),
            FieldError::new(codes::REQUIRED, "Cannot be blank."),
        );
    }
    if body.password.is_empty() {
        errors.insert(
            "password".into(),
            FieldError::new(codes::REQUIRED, "Cannot be blank."),
        );
    }
    if !errors.is_empty() {
        return Err(ApiError(AppError::validation(VALIDATION_FAILED, errors)));
    }

    let record = find_by_identity(&app, &collection, &body).await?;
    // The lookup and the verify are deliberately not distinguished in the
    // response: a caller must not be able to enumerate identities.
    let Some(record) = record else {
        // Still pay the hashing cost so a missing identity and a wrong
        // password take the same time.
        cratebase_auth::verify_password_async(&body.password, "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$0000000000000000000000000000000000000000000").await;
        return Err(ApiError::bad_request(AUTH_FAILED));
    };
    let hash = record.password_hash();
    if !cratebase_auth::verify_password_async(&body.password, &hash).await {
        return Err(ApiError::bad_request(AUTH_FAILED));
    }

    // `authRule` gates the login itself, separately from `listRule` and
    // friends. `null` means superusers only.
    if !passes_auth_rule(&app, &collection, &record).await? {
        return Err(ApiError::forbidden(AUTH_RULE_FAILED));
    }

    // A bcrypt hash imported from PocketBase is upgraded on the spot.
    // `tokenKey` is deliberately left alone: rotating it here would
    // invalidate the token this very request is about to mint.
    if cratebase_auth::needs_rehash(&hash) {
        if let Err(e) = rehash(&app, &collection, &record, &body.password).await {
            tracing::warn!(error = %e, "failed to upgrade a legacy password hash");
        }
    }

    match mfa_gate(
        &app,
        &collection,
        &record,
        "password",
        body.mfa_id.as_deref(),
    )
    .await?
    {
        MfaGate::Pending(mfa_id) => return Ok(mfa_pending_response(mfa_id)),
        MfaGate::Passed => {}
    }

    record_login_origin(&app, &collection, &record, &headers, peer).await;

    respond_with_token(&app, &collection, record, info, raw, |hooks| {
        &hooks.on_record_auth_with_password_request
    })
    .await
    .map(IntoResponse::into_response)
}

/// Look the record up by each configured identity field in turn.
async fn find_by_identity(
    app: &App,
    collection: &Arc<Collection>,
    body: &PasswordBody,
) -> ApiResult<Option<Record>> {
    let configured = collection.identity_fields();
    let candidates: Vec<String> = if body.identity_field.is_empty() {
        configured
    } else if configured.contains(&body.identity_field) {
        vec![body.identity_field.clone()]
    } else {
        // Pinning a field that is not an identity field is simply a
        // failed login, not a validation error.
        return Ok(None);
    };

    let mut params = Map::new();
    params.insert("identity".into(), Value::String(body.identity.clone()));
    for field in candidates {
        if !collection.has_field(&field) {
            continue;
        }
        let filter = format!("{field} = {{:identity}}");
        let found = records::find_first_by_filter(
            app.db(),
            &app.db().collections,
            collection,
            &filter,
            &params,
        )
        .await
        .map_err(|e| ApiError(e.into()))?;
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

/// `authRule`: `Some("")` is public, `Some(expr)` must match the record,
/// `None` means superusers only (so nobody can log in through the API).
async fn passes_auth_rule(
    app: &App,
    collection: &Arc<Collection>,
    record: &Record,
) -> ApiResult<bool> {
    match collection.auth.auth_rule.as_deref() {
        Some(expr) if expr.trim().is_empty() => Ok(true),
        None => Ok(false),
        Some(_) => {
            let ctx = cratebase_db::context::RequestContext::default();
            common::record_matches_rule(
                app.db(),
                &app.db().collections,
                &ctx,
                collection,
                &collection.auth.auth_rule,
                record.id(),
            )
            .await
            .map_err(|e| ApiError(e.into()))
        }
    }
}

async fn rehash(
    app: &App,
    collection: &Arc<Collection>,
    record: &Record,
    plaintext: &str,
) -> Result<(), AppError> {
    let hash = cratebase_auth::hash_password_async(plaintext)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let sql = format!(
        "UPDATE {} SET \"password\" = $1 WHERE \"id\" = $2",
        cratebase_db::quote_ident(collection.table_name())
    );
    app.db()
        .execute(
            &sql,
            &[
                cratebase_db::Sql::Text(hash),
                cratebase_db::Sql::Text(record.id().to_string()),
            ],
        )
        .await
        .map(|_| ())
        .map_err(AppError::from)
}

// -------------------------------------------------------------- auth-refresh

async fn auth_refresh(
    State(app): State<App>,
    Path(name): Path<String>,
    auth: Auth,
    headers: HeaderMap,
    info: RequestInfo,
) -> ApiResult<Json<Value>> {
    let collection = common::auth_collection_of(&app, &name)?;
    if auth.collection_id != collection.id {
        // PocketBase names the *authenticated* record's collection here.
        return Err(ApiError::forbidden(format!(
            "The request requires auth record from {} collection.",
            auth.collection_name
        )));
    }

    // An impersonation token (or any other token deliberately minted
    // non-refreshable) is echoed back unchanged rather than extended:
    // `auth-refresh` does not reject it, it just declines to renew it.
    if let Some(token) = bearer_token(&headers) {
        if let Ok(claims) = cratebase_auth::decode_unverified(token) {
            if !claims.is_refreshable() {
                let serialized = common::enrich_and_serialize(
                    &app,
                    &collection,
                    auth.record.clone(),
                    Some(auth.clone()),
                    true,
                )
                .await?;
                return Ok(Json(json!({ "token": token, "record": serialized })));
            }
        }
    }

    let record = auth.record.clone();
    respond_with_token(
        &app,
        &collection,
        record,
        info,
        Value::Object(Map::new()),
        |hooks| &hooks.on_record_auth_refresh_request,
    )
    .await
}

/// Extracts a bearer token from `Authorization`, the same way
/// [`crate::extract::Auth`] does — duplicated here (rather than exported
/// from `extract`) because it takes a raw [`HeaderMap`] instead of
/// request `Parts`.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    Some(raw.strip_prefix("Bearer ").unwrap_or(raw).trim()).filter(|t| !t.is_empty())
}

// -------------------------------------------------------------- auth-methods

async fn auth_methods(State(app): State<App>, Path(name): Path<String>) -> ApiResult<Json<Value>> {
    let collection = app
        .db()
        .collections
        .get(&name)
        .ok_or_else(|| ApiError::not_found(MISSING_COLLECTION))?;
    if !collection.is_auth() {
        return Err(ApiError::not_found(
            "Missing or invalid auth collection context.",
        ));
    }
    let auth = &collection.auth;
    Ok(Json(json!({
        "password": {
            "enabled": auth.password_auth.enabled,
            "identityFields": collection.identity_fields(),
        },
        // W4b-2: list the configured providers with their auth URLs and
        // PKCE state once the OAuth2 service exists.
        "oauth2": {
            "enabled": auth.oauth2.enabled,
            "providers": Value::Array(vec![]),
        },
        // A disabled method reports a zero duration, not its configured
        // one — pinned by `auth.test.ts`.
        "mfa": {
            "enabled": auth.mfa.enabled,
            "duration": if auth.mfa.enabled { auth.mfa.duration } else { 0 },
        },
        "otp": {
            "enabled": auth.otp.enabled,
            "duration": if auth.otp.enabled { auth.otp.duration } else { 0 },
        },
    })))
}

// ------------------------------------------------------------------- shared

/// Mint the session token, run the auth hooks and render
/// `{token, record}`.
async fn respond_with_token(
    app: &App,
    collection: &Arc<Collection>,
    record: Record,
    info: RequestInfo,
    body: Value,
    hook: fn(&Hooks) -> &Hook<RecordRequestEvent>,
) -> ApiResult<Json<Value>> {
    let info = info.with_body(body.as_object().cloned().unwrap_or_default());
    let mut event = RecordRequestEvent::new(
        app.clone(),
        collection.clone(),
        info.clone(),
        info.auth.clone(),
        Some(record),
        collection_tags(collection),
    );
    hook(app.hooks())
        .trigger_bare(&mut event)
        .await
        .map_err(ApiError)?;
    app.hooks()
        .on_record_auth_request
        .trigger_bare(&mut event)
        .await
        .map_err(ApiError)?;

    let record = event
        .record
        .ok_or_else(|| ApiError::bad_request(AUTH_FAILED))?;
    let token = mint(app, collection, &record)?;
    // The owner of a session always sees their own address.
    let serialized =
        common::enrich_and_serialize(app, collection, record, info.auth.clone(), true).await?;
    Ok(Json(json!({ "token": token, "record": serialized })))
}

/// A PocketBase-shaped session token: `{collectionId, exp, id,
/// refreshable, type}`, signed with the record's own `tokenKey`.
fn mint(app: &App, collection: &Arc<Collection>, record: &Record) -> ApiResult<String> {
    let config = &collection.auth.auth_token;
    let claims =
        cratebase_auth::new_auth_claims(record.id(), &collection.id, config.duration.max(1), true);
    cratebase_auth::sign(
        &claims,
        &app.token_signing_key(&record.token_key(), &config.secret),
    )
    .map_err(|e| ApiError::internal(e.to_string()))
}

/// Whether a submitted address is well-formed enough to look up or
/// email. Deliberately the same rule `routes::settings` uses, duplicated
/// rather than shared because it is three lines and the two modules
/// otherwise have no reason to depend on each other.
fn is_email(raw: &str) -> bool {
    let raw = raw.trim();
    match raw.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !raw.contains(' ')
        }
        None => false,
    }
}

fn email_validation_error() -> ApiError {
    let mut errors = BTreeMap::new();
    errors.insert(
        "email".into(),
        FieldError::new(codes::INVALID_EMAIL, "Must be a valid email address."),
    );
    ApiError(AppError::validation(VALIDATION_FAILED, errors))
}

/// Rule-free "does this auth collection have a record at this address".
async fn find_by_email(
    app: &App,
    collection: &Arc<Collection>,
    email: &str,
) -> ApiResult<Option<Record>> {
    if !collection.has_field("email") {
        return Ok(None);
    }
    let mut params = Map::new();
    params.insert("email".into(), Value::String(email.to_string()));
    records::find_first_by_filter(
        app.db(),
        &app.db().collections,
        collection,
        "email = {:email}",
        &params,
    )
    .await
    .map_err(|e| ApiError(e.into()))
}

/// Signs a one-shot token (verification / passwordReset / emailChange)
/// with the record's own key, exactly like a session token but under the
/// relevant `TokenConfig`'s secret.
fn mint_claims(
    app: &App,
    record: &Record,
    claims: cratebase_auth::Claims,
    type_secret: &str,
) -> ApiResult<String> {
    cratebase_auth::sign(
        &claims,
        &app.token_signing_key(&record.token_key(), type_secret),
    )
    .map_err(|e| ApiError::internal(e.to_string()))
}

/// Decodes and fully verifies a one-shot token: structurally the right
/// `token_type` and collection, a record it still resolves to, and a
/// signature that checks out under that record's *current* `tokenKey` —
/// which is exactly what makes the token single-use, since every
/// password/email change rotates `tokenKey` out from under it.
async fn decode_and_verify(
    app: &App,
    collection: &Arc<Collection>,
    token: &str,
    expected: cratebase_auth::TokenType,
    type_secret: &str,
) -> Option<Record> {
    let claims = cratebase_auth::decode_unverified(token).ok()?;
    if claims.token_type != expected || claims.collection_id != collection.id {
        return None;
    }
    let record = records::find_by_id_raw(app.db(), collection, &claims.id)
        .await
        .ok()?;
    let key = app.token_signing_key(&record.token_key(), type_secret);
    cratebase_auth::verify(token, &key).ok()?;
    Some(record)
}

/// Renders `template`, sends it to `to` and runs the record-scoped mailer
/// hook (then `onMailerSend`) around the actual delivery. Failures are
/// returned rather than swallowed; every call site that must never leak
/// account existence swallows them itself, deliberately, at the point
/// that decision belongs.
async fn send_record_mail(
    app: &App,
    collection: &Arc<Collection>,
    record: &Record,
    template: &cratebase_core::EmailTemplate,
    extra_vars: &[(&str, &str)],
    to: &str,
    hook: fn(&Hooks) -> &Hook<MailerRecordEvent>,
) -> ApiResult<()> {
    let settings = app.settings();
    let mut vars: Vec<(&str, &str)> = vec![
        ("APP_NAME", settings.meta.app_name.as_str()),
        ("APP_URL", settings.meta.app_url.as_str()),
    ];
    vars.extend_from_slice(extra_vars);
    let (subject, html) = cratebase_mailer::render_template(template, &vars);
    let message = cratebase_mailer::Message::new(
        (
            settings.meta.sender_address.clone(),
            settings.meta.sender_name.clone(),
        ),
        (to.to_string(), String::new()),
        subject,
        html,
    );
    let meta = Value::Object(
        vars.iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect(),
    );
    let mut event = MailerRecordEvent::new(
        app.clone(),
        collection.clone(),
        record.clone(),
        message,
        meta,
        collection_tags(collection),
    );
    let app2 = app.clone();
    hook(app.hooks())
        .trigger(&mut event, move |e| {
            let app = app2.clone();
            let message = e.message.clone();
            Box::pin(async move { deliver_mail(&app, message).await })
        })
        .await
        .map_err(ApiError)?;
    Ok(())
}

async fn deliver_mail(app: &App, message: cratebase_mailer::Message) -> HookResult {
    let mut event = MailerEvent::new(app.clone(), message);
    let app2 = app.clone();
    app.hooks()
        .on_mailer_send
        .trigger(&mut event, move |e| {
            let app = app2.clone();
            let message = e.message.clone();
            Box::pin(async move {
                app.mailer()
                    .send(&message)
                    .await
                    .map_err(|err| AppError::bad_request(err.to_string()))
            })
        })
        .await
}

/// Fires a `*Request` hook with no framework action to guard — used by
/// every one-shot auth flow below so a plugin can observe (or, by
/// erroring, abort) the request even though these handlers don't route
/// through [`respond_with_token`].
async fn fire_request_hook(
    app: &App,
    collection: &Arc<Collection>,
    info: &RequestInfo,
    record: Option<Record>,
    hook: fn(&Hooks) -> &Hook<RecordRequestEvent>,
) -> ApiResult<()> {
    let mut event = RecordRequestEvent::new(
        app.clone(),
        collection.clone(),
        info.clone(),
        info.auth.clone(),
        record,
        collection_tags(collection),
    );
    hook(app.hooks())
        .trigger_bare(&mut event)
        .await
        .map_err(ApiError)
}

// -------------------------------------------------------------- verification

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmailBody {
    #[serde(default)]
    email: String,
}

#[derive(Debug, Default, Deserialize)]
struct TokenBody {
    #[serde(default)]
    token: String,
}

async fn request_verification(
    State(app): State<App>,
    Path(name): Path<String>,
    info: RequestInfo,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<StatusCode> {
    let collection = common::auth_collection_of(&app, &name)?;
    let body: EmailBody = serde_json::from_value(raw).unwrap_or_default();
    if !is_email(&body.email) {
        return Err(email_validation_error());
    }

    // Never distinguish "no such account" or "already verified" from
    // success — both send no mail but still answer 204, so the endpoint
    // cannot be used to enumerate registered addresses.
    if let Ok(Some(record)) = find_by_email(&app, &collection, &body.email).await {
        if !record.verified() {
            fire_request_hook(&app, &collection, &info, Some(record.clone()), |h| {
                &h.on_record_request_verification_request
            })
            .await?;
            let duration = collection.auth.verification_token.duration.max(1);
            let claims = cratebase_auth::new_verification_claims(
                record.id(),
                &collection.id,
                duration,
                &record.email(),
            );
            if let Ok(token) = mint_claims(
                &app,
                &record,
                claims,
                &collection.auth.verification_token.secret,
            ) {
                let _ = send_record_mail(
                    &app,
                    &collection,
                    &record,
                    &collection.auth.verification_template,
                    &[("TOKEN", token.as_str())],
                    &record.email(),
                    |h| &h.on_mailer_record_verification_send,
                )
                .await;
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn confirm_verification(
    State(app): State<App>,
    Path(name): Path<String>,
    info: RequestInfo,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<StatusCode> {
    let collection = common::auth_collection_of(&app, &name)?;
    let body: TokenBody = serde_json::from_value(raw).unwrap_or_default();

    let structural = cratebase_auth::decode_unverified(&body.token)
        .ok()
        .filter(|c| c.token_type == cratebase_auth::TokenType::Verification && c.email().is_some());
    let Some(_) = &structural else {
        let mut errors = BTreeMap::new();
        errors.insert(
            "token".into(),
            FieldError::new(TOKEN_MISSING_EMAIL_CLAIM, "Missing email token claim."),
        );
        return Err(ApiError(AppError::validation(VALIDATION_FAILED, errors)));
    };
    let record = decode_and_verify(
        &app,
        &collection,
        &body.token,
        cratebase_auth::TokenType::Verification,
        &collection.auth.verification_token.secret,
    )
    .await;
    let Some(mut record) = record else {
        let mut errors = BTreeMap::new();
        errors.insert(
            "token".into(),
            FieldError::new(codes::INVALID_TOKEN, "Invalid or expired token."),
        );
        return Err(ApiError(AppError::validation(VALIDATION_FAILED, errors)));
    };

    fire_request_hook(&app, &collection, &info, Some(record.clone()), |h| {
        &h.on_record_confirm_verification_request
    })
    .await?;

    if !record.verified() {
        record.set("verified", Value::Bool(true));
        records::update(app.db(), &app.db().collections, &mut record)
            .await
            .map_err(|e| ApiError(e.into()))?;
    }
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------ password reset

async fn request_password_reset(
    State(app): State<App>,
    Path(name): Path<String>,
    info: RequestInfo,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<StatusCode> {
    let collection = common::auth_collection_of(&app, &name)?;
    let body: EmailBody = serde_json::from_value(raw).unwrap_or_default();
    if !is_email(&body.email) {
        return Err(email_validation_error());
    }

    if let Ok(Some(record)) = find_by_email(&app, &collection, &body.email).await {
        fire_request_hook(&app, &collection, &info, Some(record.clone()), |h| {
            &h.on_record_request_password_reset_request
        })
        .await?;
        let duration = collection.auth.password_reset_token.duration.max(1);
        let claims = cratebase_auth::new_password_reset_claims(
            record.id(),
            &collection.id,
            duration,
            &record.email(),
        );
        if let Ok(token) = mint_claims(
            &app,
            &record,
            claims,
            &collection.auth.password_reset_token.secret,
        ) {
            let _ = send_record_mail(
                &app,
                &collection,
                &record,
                &collection.auth.reset_password_template,
                &[("TOKEN", token.as_str())],
                &record.email(),
                |h| &h.on_mailer_record_password_reset_send,
            )
            .await;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn confirm_password_reset(
    State(app): State<App>,
    Path(name): Path<String>,
    info: RequestInfo,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<StatusCode> {
    let collection = common::auth_collection_of(&app, &name)?;

    #[derive(Debug, Default, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Body {
        #[serde(default)]
        token: String,
        #[serde(default)]
        password: String,
        #[serde(default)]
        password_confirm: String,
    }
    let body: Body = serde_json::from_value(raw).unwrap_or_default();

    let record = decode_and_verify(
        &app,
        &collection,
        &body.token,
        cratebase_auth::TokenType::PasswordReset,
        &collection.auth.password_reset_token.secret,
    )
    .await;

    let mut errors: BTreeMap<String, FieldError> = BTreeMap::new();
    if record.is_none() {
        errors.insert(
            "token".into(),
            FieldError::new(codes::INVALID_TOKEN, "Invalid or expired token."),
        );
    }
    if body.password.is_empty() || body.password != body.password_confirm {
        errors.insert(
            "passwordConfirm".into(),
            FieldError::new(codes::VALUES_MISMATCH, "Values don't match."),
        );
    }
    if !errors.is_empty() {
        return Err(ApiError(AppError::validation(VALIDATION_FAILED, errors)));
    }

    let mut record = record.expect("checked above");
    fire_request_hook(&app, &collection, &info, Some(record.clone()), |h| {
        &h.on_record_confirm_password_reset_request
    })
    .await?;

    record.set("password", Value::String(body.password));
    record.set("tokenKey", Value::String(crate::app::new_token_key()));
    records::update(app.db(), &app.db().collections, &mut record)
        .await
        .map_err(|e| ApiError(e.into()))?;
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------- email change

async fn request_email_change(
    State(app): State<App>,
    Path(name): Path<String>,
    auth: Auth,
    info: RequestInfo,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<StatusCode> {
    let collection = common::auth_collection_of(&app, &name)?;
    if auth.collection_id != collection.id {
        return Err(ApiError::forbidden(format!(
            "The request requires auth record from {} collection.",
            auth.collection_name
        )));
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Body {
        #[serde(default)]
        new_email: String,
    }
    let body: Body = serde_json::from_value(raw).unwrap_or_default();

    let mut errors: BTreeMap<String, FieldError> = BTreeMap::new();
    if !is_email(&body.new_email) {
        errors.insert(
            "newEmail".into(),
            FieldError::new(codes::INVALID_EMAIL, "Must be a valid email address."),
        );
    } else if body.new_email == auth.record.email() {
        errors.insert(
            "newEmail".into(),
            FieldError::new(NOT_IN_INVALID, "Must not be in list."),
        );
    }
    if !errors.is_empty() {
        return Err(ApiError(AppError::validation(VALIDATION_FAILED, errors)));
    }

    fire_request_hook(&app, &collection, &info, Some(auth.record.clone()), |h| {
        &h.on_record_request_email_change_request
    })
    .await?;

    let duration = collection.auth.email_change_token.duration.max(1);
    let claims = cratebase_auth::new_email_change_claims(
        auth.record.id(),
        &collection.id,
        duration,
        &auth.record.email(),
        &body.new_email,
    );
    if let Ok(token) = mint_claims(
        &app,
        &auth.record,
        claims,
        &collection.auth.email_change_token.secret,
    ) {
        let _ = send_record_mail(
            &app,
            &collection,
            &auth.record,
            &collection.auth.confirm_email_change_template,
            &[("TOKEN", token.as_str())],
            &body.new_email,
            |h| &h.on_mailer_record_email_change_send,
        )
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Unlike every other confirm-* endpoint, this one needs no
/// `Authorization` header: the record it acts on is whichever one the
/// token's (unverified) `id` claim names, so the link works from a
/// browser that was never logged in. `password` is the extra factor that
/// keeps a merely-intercepted email link from being sufficient — checked
/// against *that* record, which is why an undecodable token fails
/// `password` too: there is no record left to check it against.
async fn confirm_email_change(
    State(app): State<App>,
    Path(name): Path<String>,
    info: RequestInfo,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<StatusCode> {
    let collection = common::auth_collection_of(&app, &name)?;

    #[derive(Debug, Default, Deserialize)]
    struct Body {
        #[serde(default)]
        token: String,
        #[serde(default)]
        password: String,
    }
    let body: Body = serde_json::from_value(raw).unwrap_or_default();

    let structural = cratebase_auth::decode_unverified(&body.token)
        .ok()
        .filter(|c| {
            c.token_type == cratebase_auth::TokenType::EmailChange && c.new_email().is_some()
        });

    let mut errors: BTreeMap<String, FieldError> = BTreeMap::new();
    let mut candidate: Option<Record> = None;
    let mut token_ok = false;

    match &structural {
        None => {
            errors.insert(
                "token".into(),
                FieldError::new(
                    TOKEN_INVALID_PAYLOAD,
                    "Invalid token payload - newEmail must be set.",
                ),
            );
        }
        Some(claims) => {
            if claims.collection_id == collection.id {
                if let Ok(record) = records::find_by_id_raw(app.db(), &collection, &claims.id).await
                {
                    let key = app.token_signing_key(
                        &record.token_key(),
                        &collection.auth.email_change_token.secret,
                    );
                    token_ok = cratebase_auth::verify(&body.token, &key).is_ok();
                    candidate = Some(record);
                }
            }
            if !token_ok {
                errors.insert(
                    "token".into(),
                    FieldError::new(codes::INVALID_TOKEN, "Invalid or expired token."),
                );
            }
        }
    }

    let password_ok = match &candidate {
        Some(record) => {
            !body.password.is_empty()
                && cratebase_auth::verify_password_async(&body.password, &record.password_hash())
                    .await
        }
        None => false,
    };
    if !password_ok {
        errors.insert(
            "password".into(),
            FieldError::new(
                TOKEN_INVALID_PASSWORD,
                "Missing or invalid auth record password.",
            ),
        );
    }

    if !errors.is_empty() {
        return Err(ApiError(AppError::validation(VALIDATION_FAILED, errors)));
    }

    let claims = structural.expect("checked above: no errors means a decodable token");
    let mut record = candidate.expect("checked above: no errors means a resolved record");
    fire_request_hook(&app, &collection, &info, Some(record.clone()), |h| {
        &h.on_record_confirm_email_change_request
    })
    .await?;

    let new_email = claims
        .new_email()
        .expect("filtered for Some above")
        .to_string();
    record.set("email", Value::String(new_email));
    record.set("verified", Value::Bool(true));
    record.set("tokenKey", Value::String(crate::app::new_token_key()));
    records::update(app.db(), &app.db().collections, &mut record)
        .await
        .map_err(|e| ApiError(e.into()))?;
    Ok(StatusCode::NO_CONTENT)
}

// --------------------------------------------------------------------- OTP

async fn request_otp(
    State(app): State<App>,
    Path(name): Path<String>,
    info: RequestInfo,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let collection = common::auth_collection_of(&app, &name)?;
    if !collection.auth.otp.enabled {
        return Err(ApiError::forbidden(OTP_DISABLED));
    }
    let body: EmailBody = serde_json::from_value(raw).unwrap_or_default();
    if !is_email(&body.email) {
        return Err(email_validation_error());
    }

    let otp_id = match find_by_email(&app, &collection, &body.email).await? {
        Some(record) => {
            fire_request_hook(&app, &collection, &info, Some(record.clone()), |h| {
                &h.on_record_request_otp_request
            })
            .await?;

            let code = cratebase_auth::generate_otp(collection.auth.otp.length.max(0) as usize);
            let otps = app
                .db()
                .collections
                .get("_otps")
                .expect("_otps is a default system collection");
            let mut row = Record::new(otps.clone());
            row.set("collectionRef", Value::String(collection.id.clone()));
            row.set("recordRef", Value::String(record.id().to_string()));
            // A non-blank placeholder so `records::create`'s required-field
            // check passes; overwritten below with the real (SHA-256, not
            // Argon2 — see `cratebase_auth::hash_otp`) hash.
            row.set("password", Value::String(code.clone()));
            row.set("sentTo", Value::String(body.email.clone()));
            records::create(app.db(), &app.db().collections, &mut row)
                .await
                .map_err(|e| ApiError(e.into()))?;
            overwrite_otp_hash(&app, &otps, row.id(), &code).await?;

            let minutes = (collection.auth.otp.duration.max(1) + 59) / 60;
            let _ = send_record_mail(
                &app,
                &collection,
                &record,
                &collection.auth.otp.email_template,
                &[
                    ("OTP", code.as_str()),
                    ("OTP_ID", row.id()),
                    ("EXPIRES_IN", format!("{minutes} minutes").as_str()),
                ],
                &body.email,
                |h| &h.on_mailer_record_otp_send,
            )
            .await;
            row.id().to_string()
        }
        // No account at this address: still hand back a plausible id in
        // the same shape, so the response never leaks whether the
        // address exists.
        None => cratebase_core::ids::record_id(),
    };
    Ok(Json(json!({ "otpId": otp_id })))
}

/// Overwrites `_otps.password` with [`cratebase_auth::hash_otp`]'s fast
/// SHA-256, bypassing the column's usual Argon2id hashing — an OTP is
/// single-use and discarded within minutes, so the slow hash buys
/// nothing but latency (see `cratebase_auth::otp` module docs).
async fn overwrite_otp_hash(
    app: &App,
    otps: &Arc<Collection>,
    id: &str,
    code: &str,
) -> ApiResult<()> {
    let sql = format!(
        "UPDATE {} SET \"password\" = $1 WHERE \"id\" = $2",
        cratebase_db::quote_ident(otps.table_name())
    );
    app.db()
        .execute(
            &sql,
            &[
                cratebase_db::Sql::Text(cratebase_auth::hash_otp(code)),
                cratebase_db::Sql::Text(id.to_string()),
            ],
        )
        .await
        .map(|_| ())
        .map_err(|e| ApiError(AppError::from(e)))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OtpAuthBody {
    #[serde(default)]
    otp_id: String,
    #[serde(default)]
    password: String,
    /// A pending MFA session id from a previous `401 {mfaId}` response.
    #[serde(default)]
    mfa_id: Option<String>,
}

async fn auth_with_otp(
    State(app): State<App>,
    Path(name): Path<String>,
    info: RequestInfo,
    headers: HeaderMap,
    peer: crate::middleware::client_ip::PeerAddr,
    ApiJson(raw): ApiJson<Value>,
) -> Result<Response, ApiError> {
    let collection = common::auth_collection_of(&app, &name)?;
    if !collection.auth.otp.enabled {
        return Err(ApiError::forbidden(OTP_DISABLED));
    }
    let body: OtpAuthBody = serde_json::from_value(raw.clone())
        .map_err(|_| ApiError::bad_request(AppError::DEFAULT_BAD_REQUEST))?;

    let mut errors: BTreeMap<String, FieldError> = BTreeMap::new();
    if body.otp_id.trim().is_empty() {
        errors.insert(
            "otpId".into(),
            FieldError::new(codes::REQUIRED, "Cannot be blank."),
        );
    }
    if body.password.is_empty() {
        errors.insert(
            "password".into(),
            FieldError::new(codes::REQUIRED, "Cannot be blank."),
        );
    }
    if !errors.is_empty() {
        return Err(ApiError(AppError::validation(VALIDATION_FAILED, errors)));
    }

    let otps = app
        .db()
        .collections
        .get("_otps")
        .expect("_otps is a default system collection");
    let otp_row = records::find_by_id_raw(app.db(), &otps, &body.otp_id)
        .await
        .ok()
        .filter(|r| r.get_string("collectionRef") == collection.id);
    let Some(otp_row) = otp_row else {
        return Err(ApiError::bad_request("Invalid or expired OTP."));
    };

    let expired = match otp_row.get_datetime("created") {
        Some(created) => {
            let age = chrono::Utc::now() - created.inner();
            age.num_seconds() > collection.auth.otp.duration.max(1)
        }
        None => true,
    };
    let hash_ok = cratebase_auth::hash_otp(&body.password) == otp_row.get_string("password");

    if expired {
        // Cleaned up here rather than waiting for the sweep cron.
        let _ = records::delete(app.db(), &app.db().collections, &otp_row).await;
        return Err(ApiError::bad_request("Invalid or expired OTP."));
    }
    if !hash_ok {
        // A wrong guess does NOT consume the row: the owner can see it is
        // still pending and retry until it expires or succeeds.
        return Err(ApiError::bad_request("Invalid or expired OTP."));
    }
    records::delete(app.db(), &app.db().collections, &otp_row)
        .await
        .map_err(|e| ApiError(e.into()))?;

    let mut record =
        records::find_by_id_raw(app.db(), &collection, &otp_row.get_string("recordRef"))
            .await
            .map_err(|e| ApiError(e.into()))?;

    match mfa_gate(&app, &collection, &record, "otp", body.mfa_id.as_deref()).await? {
        MfaGate::Pending(mfa_id) => return Ok(mfa_pending_response(mfa_id)),
        MfaGate::Passed => {}
    }

    // A successful OTP proves control of the mailbox, so PocketBase marks
    // the record verified the same way a clicked verification link does.
    if !record.verified() {
        record.set("verified", Value::Bool(true));
        records::update(app.db(), &app.db().collections, &mut record)
            .await
            .map_err(|e| ApiError(e.into()))?;
    }

    record_login_origin(&app, &collection, &record, &headers, peer).await;

    respond_with_token(&app, &collection, record, info, raw, |hooks| {
        &hooks.on_record_auth_with_otp_request
    })
    .await
    .map(IntoResponse::into_response)
}

// --------------------------------------------------------------------- MFA

enum MfaGate {
    Passed,
    /// A first factor just succeeded; `.0` is the new `_mfas` row id the
    /// caller must echo back as `mfaId` with a *different* method.
    Pending(String),
}

/// A `401 {"mfaId": "..."}` — deliberately not PocketBase's usual error
/// envelope (no `status`/`message`/`data`), so the SDK's generic error
/// handling can't mistake it for an ordinary failure.
fn mfa_pending_response(mfa_id: String) -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({ "mfaId": mfa_id }))).into_response()
}

/// Gate a successful single-factor login behind MFA, PocketBase style:
///
/// * MFA disabled, or `mfa.rule` doesn't select this record → straight
///   through;
/// * no `mfaId` supplied → this credential check was the *first* factor:
///   open a `_mfas` session and report it as pending;
/// * a `mfaId` supplied → it must name a still-pending session for this
///   exact record, completed with a *different* method than the one
///   that just succeeded. Consumed (deleted) on success, so a session
///   cannot be replayed.
async fn mfa_gate(
    app: &App,
    collection: &Arc<Collection>,
    record: &Record,
    method: &str,
    mfa_id: Option<&str>,
) -> ApiResult<MfaGate> {
    if !collection.auth.mfa.enabled {
        return Ok(MfaGate::Passed);
    }
    let rule = collection.auth.mfa.rule.trim();
    let required = if rule.is_empty() {
        true
    } else {
        common::record_matches_rule(
            app.db(),
            &app.db().collections,
            &cratebase_db::context::RequestContext::default(),
            collection,
            &Some(collection.auth.mfa.rule.clone()),
            record.id(),
        )
        .await
        .map_err(|e| ApiError(e.into()))?
    };
    if !required {
        return Ok(MfaGate::Passed);
    }

    let mfas = app
        .db()
        .collections
        .get("_mfas")
        .expect("_mfas is a default system collection");
    match mfa_id {
        None => {
            let mut row = Record::new(mfas.clone());
            row.set("collectionRef", Value::String(collection.id.clone()));
            row.set("recordRef", Value::String(record.id().to_string()));
            row.set("method", Value::String(method.to_string()));
            records::create(app.db(), &app.db().collections, &mut row)
                .await
                .map_err(|e| ApiError(e.into()))?;
            Ok(MfaGate::Pending(row.id().to_string()))
        }
        Some(id) => {
            let row = records::find_by_id_raw(app.db(), &mfas, id)
                .await
                .ok()
                .filter(|r| {
                    r.get_string("collectionRef") == collection.id
                        && r.get_string("recordRef") == record.id()
                });
            let Some(row) = row else {
                return Err(ApiError::bad_request("Invalid or expired MFA session."));
            };
            if row.get_string("method") == method {
                return Err(ApiError::bad_request(
                    "A different authentication method is required.",
                ));
            }
            let _ = records::delete(app.db(), &app.db().collections, &row).await;
            Ok(MfaGate::Passed)
        }
    }
}

// ---------------------------------------------------------- auth origins

/// Records this login's `_authOrigins` fingerprint and, when it is a
/// genuinely new device for a record that already had at least one
/// prior origin on file, fires the "login from a new location" alert.
/// A record's very first login ever is never alerted — there is nothing
/// to compare it against yet.
///
/// Best-effort: a failure here must never fail the login it rides along
/// with, so the caller only gets a log line.
async fn record_login_origin(
    app: &App,
    collection: &Arc<Collection>,
    record: &Record,
    headers: &HeaderMap,
    peer: crate::middleware::client_ip::PeerAddr,
) {
    if let Err(e) = record_login_origin_inner(app, collection, record, headers, peer).await {
        tracing::warn!(error = %e.error, "failed to record the auth origin");
    }
}

async fn record_login_origin_inner(
    app: &App,
    collection: &Arc<Collection>,
    record: &Record,
    headers: &HeaderMap,
    peer: crate::middleware::client_ip::PeerAddr,
) -> ApiResult<()> {
    let settings = app.settings();
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let ip = crate::middleware::client_ip::client_ip(headers, peer.0, &settings.trusted_proxy);
    let fingerprint = cratebase_auth::auth_origin_fingerprint(user_agent, &ip);

    let origins = app
        .db()
        .collections
        .get("_authOrigins")
        .expect("_authOrigins is a default system collection");
    let mut params = Map::new();
    params.insert("c".into(), Value::String(collection.id.clone()));
    params.insert("r".into(), Value::String(record.id().to_string()));
    params.insert("f".into(), Value::String(fingerprint.clone()));

    let known = records::find_first_by_filter(
        app.db(),
        &app.db().collections,
        &origins,
        "collectionRef = {:c} && recordRef = {:r} && fingerprint = {:f}",
        &params,
    )
    .await
    .map_err(|e| ApiError(e.into()))?;
    if known.is_some() {
        return Ok(());
    }

    let had_prior_origin = records::find_first_by_filter(
        app.db(),
        &app.db().collections,
        &origins,
        "collectionRef = {:c} && recordRef = {:r}",
        &params,
    )
    .await
    .map_err(|e| ApiError(e.into()))?
    .is_some();

    let mut row = Record::new(origins.clone());
    row.set("collectionRef", Value::String(collection.id.clone()));
    row.set("recordRef", Value::String(record.id().to_string()));
    row.set("fingerprint", Value::String(fingerprint));
    records::create(app.db(), &app.db().collections, &mut row)
        .await
        .map_err(|e| ApiError(e.into()))?;

    if had_prior_origin && collection.auth.auth_alert.enabled {
        let alert_info = format!("{user_agent} ({ip})");
        let _ = send_record_mail(
            app,
            collection,
            record,
            &collection.auth.auth_alert.email_template,
            &[("ALERT_INFO", alert_info.as_str())],
            &record.email(),
            |h| &h.on_mailer_record_auth_alert_send,
        )
        .await;
    }
    Ok(())
}

// ---------------------------------------------------------------- impersonate

async fn impersonate(
    State(app): State<App>,
    Path((name, id)): Path<(String, String)>,
    caller: MaybeAuth,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let collection = common::auth_collection_of(&app, &name)?;
    let caller = match caller.0 {
        Some(a) if a.is_superuser => a,
        Some(_) => {
            return Err(ApiError::forbidden(
                "The authorized record is not allowed to perform this action.",
            ))
        }
        None => return Err(ApiError::unauthorized("")),
    };

    let record = records::find_by_id_raw(app.db(), &collection, &id)
        .await
        .map_err(|e| ApiError(e.into()))?;

    #[derive(Debug, Default, Deserialize)]
    struct Body {
        #[serde(default)]
        duration: i64,
    }
    let body: Body = serde_json::from_value(raw).unwrap_or_default();
    let config = &collection.auth.auth_token;
    let duration = if body.duration > 0 {
        body.duration
    } else {
        config.duration.max(1)
    };
    // Impersonation is deliberately non-refreshable: it is a superuser's
    // one-shot loan of a session, not a credential the impersonated
    // record can extend on its own behalf.
    let claims = cratebase_auth::new_auth_claims(record.id(), &collection.id, duration, false);
    let token = cratebase_auth::sign(
        &claims,
        &app.token_signing_key(&record.token_key(), &config.secret),
    )
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let serialized =
        common::enrich_and_serialize(&app, &collection, record, Some(caller), true).await?;
    Ok(Json(json!({ "token": token, "record": serialized })))
}
