//! `/api/collections/{collection}/auth-*` — the password subset.
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
//! SDK reads `refreshable` off the payload.
//!
//! # Statuses that are not what you would guess
//!
//! * a disabled password method is **403**, not 400;
//! * an auth endpoint on a *base* collection is `404 "Missing or invalid
//!   auth collection context."`, while `auth-methods` on a collection
//!   that does not exist at all is `404 "Missing or invalid collection
//!   context."` — different wording for a different mistake;
//! * a failing `authRule` is **403**, a wrong password a flat
//!   `400 "Failed to authenticate."` with no `data`.
//!
//! W4b-2: OAuth2, OTP, MFA, impersonate, verification, password reset,
//! email change, external auths and `_authOrigins`/auth-alert emails.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use cratebase_core::{codes, AppError, Collection, FieldError, Record};
use cratebase_db::records;
use cratebase_db::Executor;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::events::{collection_tags, RecordRequestEvent};
use crate::extract::{Auth, RequestInfo};
use crate::http_error::{ApiError, ApiJson, ApiResult};
use crate::routes::common;

/// PocketBase's deliberately uninformative login failure.
const AUTH_FAILED: &str = "Failed to authenticate.";
const VALIDATION_FAILED: &str = "An error occurred while validating the submitted data.";
const PASSWORD_DISABLED: &str =
    "The collection is not configured to allow password authentication.";
const AUTH_RULE_FAILED: &str =
    "The request doesn't satisfy the collection requirements to authenticate.";
/// `auth-methods` on a name that is not a collection at all.
const MISSING_COLLECTION: &str = "Missing or invalid collection context.";

pub fn router() -> Router<App> {
    Router::new()
        .route(
            "/collections/{collection}/auth-with-password",
            post(auth_with_password),
        )
        .route("/collections/{collection}/auth-refresh", post(auth_refresh))
        .route("/collections/{collection}/auth-methods", get(auth_methods))
    // W4b-2: auth-with-oauth2, request-otp, auth-with-otp, impersonate,
    // request-verification, confirm-verification, request-password-reset,
    // confirm-password-reset, request-email-change, confirm-email-change,
    // list-external-auths, unlink-external-auth.
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
}

async fn auth_with_password(
    State(app): State<App>,
    Path(name): Path<String>,
    info: RequestInfo,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let collection = common::auth_collection_of(&app, &name)?;
    if !collection.auth.password_auth.enabled {
        return Err(ApiError::forbidden(PASSWORD_DISABLED));
    }
    let body: PasswordBody = serde_json::from_value(raw.clone())
        .map_err(|_| ApiError::bad_request(AppError::DEFAULT_BAD_REQUEST))?;

    let mut errors: std::collections::BTreeMap<String, FieldError> = Default::default();
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

    respond_with_token(&app, &collection, record, info, raw, |hooks| {
        &hooks.on_record_auth_with_password_request
    })
    .await
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
    hook: fn(&crate::hooks::Hooks) -> &crate::hooks::Hook<RecordRequestEvent>,
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
