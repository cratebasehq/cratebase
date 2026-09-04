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
//! # OAuth2
//!
//! `auth-with-oauth2` is the authorization-code grant the SDK's
//! `authWithOAuth2Code` expects: the SDK opens the provider's
//! `authURL` (from `auth-methods`) itself with a redirect URI it
//! controls, then posts the resulting `code`/`codeVerifier`/`redirectURL`
//! here. The server exchanges `code` for a token server-side (never
//! trusting anything the client says about the provider beyond the
//! code), fetches the provider's userinfo, and either signs in the
//! `_externalAuths`-linked record or creates one — see
//! [`resolve_oauth2_record`]. Google and GitHub are recognized presets
//! ([`cratebase_auth::KnownProvider`]) with baked-in endpoints/scopes; any
//! other `name` is a hand-configured provider using whatever
//! `authURL`/`tokenURL`/`userInfoURL` the collection sets. External auths
//! otherwise need no dedicated routes: the SDK's
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
use cratebase_core::{codes, AppError, Collection, FieldError, OAuth2MappedFields, Record};
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
const OAUTH2_DISABLED: &str = "The collection is not configured to allow OAuth2 authentication.";
/// PocketBase's exact code for an unrecognized/disabled `provider` name.
const OAUTH2_INVALID_PROVIDER: &str = "validation_invalid_provider";
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
        .route(
            "/collections/{collection}/auth-with-oauth2",
            post(auth_with_oauth2),
        )
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
    let providers = if auth.oauth2.enabled {
        auth.oauth2
            .providers
            .iter()
            .filter(|p| !p.client_id.is_empty() && !p.client_secret.is_empty())
            .map(oauth2_provider_info)
            .collect()
    } else {
        Vec::new()
    };
    Ok(Json(json!({
        "password": {
            "enabled": auth.password_auth.enabled,
            "identityFields": collection.identity_fields(),
        },
        "oauth2": {
            "enabled": auth.oauth2.enabled,
            "providers": providers,
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

/// One `oauth2.providers[]` entry — everything the SDK's
/// `authWithOAuth2` popup flow needs to send the browser to the
/// provider itself: a fresh `state`, freshly generated PKCE
/// `codeVerifier`/`codeChallenge` (the SDK echoes `codeVerifier` back
/// verbatim in the `auth-with-oauth2` POST, so cratebase needs no
/// server-side session to remember it), and an `authURL` with every
/// query param except `redirect_uri` already filled in — the SDK
/// appends its own before sending the browser there, exactly like
/// PocketBase's own `authURL + "&redirect_uri="`.
fn oauth2_provider_info(config: &cratebase_core::OAuth2Provider) -> Value {
    let known = cratebase_auth::KnownProvider::from_name(&config.name);
    let auth_url = if !config.auth_url.is_empty() {
        config.auth_url.as_str()
    } else {
        known
            .map(cratebase_auth::KnownProvider::auth_url)
            .unwrap_or("")
    };
    let scope = config
        .extra
        .get("scope")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| known.map(|k| k.default_scope().to_string()))
        .unwrap_or_default();
    let display_name = if !config.display_name.is_empty() {
        config.display_name.clone()
    } else {
        known
            .map(|k| k.display_name().to_string())
            .unwrap_or_else(|| config.name.clone())
    };
    let state = cratebase_auth::random_state();
    let pkce = config.pkce.unwrap_or(true);
    let mut url = reqwest::Url::parse(auth_url).ok();
    if let Some(url) = url.as_mut() {
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &config.client_id)
            .append_pair("state", &state);
        if !scope.is_empty() {
            url.query_pairs_mut().append_pair("scope", &scope);
        }
    }
    let (code_verifier, code_challenge, code_challenge_method) = if pkce {
        let verifier = cratebase_auth::code_verifier();
        let challenge = cratebase_auth::code_challenge_s256(&verifier);
        if let Some(url) = url.as_mut() {
            url.query_pairs_mut()
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256");
        }
        (verifier, challenge, "S256".to_string())
    } else {
        (String::new(), String::new(), String::new())
    };
    // Empty `redirect_uri=` left for the SDK to append its own value to,
    // matching PocketBase's `authURL` shape exactly.
    let auth_url = url
        .map(|u| format!("{u}&redirect_uri="))
        .unwrap_or_default();
    json!({
        "name": config.name,
        "displayName": display_name,
        "state": state,
        "authURL": auth_url,
        "codeVerifier": code_verifier,
        "codeChallenge": code_challenge,
        "codeChallengeMethod": code_challenge_method,
    })
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

// ------------------------------------------------------------------ OAuth2

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuth2Body {
    #[serde(default)]
    provider: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    code_verifier: String,
    /// PocketBase renamed this from `redirectUrl` to `redirectURL`;
    /// both are accepted since the SDK versions in the wild send either.
    #[serde(default, rename = "redirectURL", alias = "redirectUrl")]
    redirect_url: String,
    /// Extra fields to seed a brand-new record with, same as the field
    /// of the same name on `authWithOAuth2Code`.
    #[serde(default)]
    create_data: Map<String, Value>,
    /// A pending MFA session id from a previous `401 {mfaId}` response.
    #[serde(default)]
    mfa_id: Option<String>,
}

/// The client for both outbound legs of the flow (token exchange,
/// userinfo fetch). Provider URLs are admin-configured, not
/// caller-supplied, so this skips the SSRF host-blocking
/// `webhooks.rs`'s client needs for untrusted URLs — but still disables
/// redirects on principle: a provider that answers a token exchange with
/// a 3xx is not one this request should blindly follow with client
/// credentials attached.
fn oauth2_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("static client config is valid")
    });
    &CLIENT
}

/// A configured provider's endpoint, falling back to the
/// [`cratebase_auth::KnownProvider`] default when the collection left it
/// blank — an admin enabling "google"/"github" only has to supply
/// `clientId`/`clientSecret`, exactly like PocketBase's own presets.
fn effective_url(configured: &str, known: Option<&'static str>) -> String {
    if !configured.is_empty() {
        configured.to_string()
    } else {
        known.unwrap_or_default().to_string()
    }
}

/// GETs `url` with a bearer token, returning its body only on a 2xx.
async fn get_bearer(client: &reqwest::Client, url: &str, access_token: &str) -> Option<Vec<u8>> {
    let res = client
        .get(url)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        // GitHub's API 403s any request with no User-Agent.
        .header("User-Agent", "cratebase")
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    res.bytes().await.ok().map(|b| b.to_vec())
}

/// Fetches the provider's userinfo, plus GitHub's second `/user/emails`
/// call when that provider's primary response might not carry one (see
/// [`cratebase_auth::KnownProvider::emails_url`]).
async fn fetch_oauth2_user(
    client: &reqwest::Client,
    user_info_url: &str,
    known: Option<cratebase_auth::KnownProvider>,
    access_token: &str,
) -> ApiResult<cratebase_auth::OAuth2User> {
    let user_body = get_bearer(client, user_info_url, access_token)
        .await
        .ok_or_else(|| ApiError::bad_request("Failed to fetch OAuth2 user."))?;
    let emails_body = match known.and_then(cratebase_auth::KnownProvider::emails_url) {
        Some(url) => get_bearer(client, url, access_token).await,
        None => None,
    };
    let parsed = match known {
        Some(cratebase_auth::KnownProvider::Google) => {
            cratebase_auth::parse_google_userinfo(&user_body)
        }
        Some(cratebase_auth::KnownProvider::GitHub) => {
            cratebase_auth::parse_github_userinfo(&user_body, emails_body.as_deref())
        }
        None => cratebase_auth::parse_generic_userinfo(&user_body),
    }
    .map_err(|_| ApiError::bad_request("Failed to fetch OAuth2 user."))?;
    if parsed.id.is_empty() {
        return Err(ApiError::bad_request("Failed to fetch OAuth2 user."));
    }
    Ok(parsed)
}

async fn auth_with_oauth2(
    State(app): State<App>,
    Path(name): Path<String>,
    caller: MaybeAuth,
    info: RequestInfo,
    headers: HeaderMap,
    peer: crate::middleware::client_ip::PeerAddr,
    ApiJson(raw): ApiJson<Value>,
) -> Result<Response, ApiError> {
    let collection = common::auth_collection_of(&app, &name)?;
    if !collection.auth.oauth2.enabled {
        return Err(ApiError::forbidden(OAUTH2_DISABLED));
    }
    let body: OAuth2Body = serde_json::from_value(raw.clone())
        .map_err(|_| ApiError::bad_request(AppError::DEFAULT_BAD_REQUEST))?;

    let mut errors: BTreeMap<String, FieldError> = Default::default();
    if body.provider.trim().is_empty() {
        errors.insert(
            "provider".into(),
            FieldError::new(codes::REQUIRED, "Cannot be blank."),
        );
    }
    if body.code.trim().is_empty() {
        errors.insert(
            "code".into(),
            FieldError::new(codes::REQUIRED, "Cannot be blank."),
        );
    }
    if !errors.is_empty() {
        return Err(ApiError(AppError::validation(VALIDATION_FAILED, errors)));
    }

    let Some(config) = collection
        .auth
        .oauth2
        .providers
        .iter()
        .find(|p| p.name == body.provider)
    else {
        let mut errors = BTreeMap::new();
        errors.insert(
            "provider".into(),
            FieldError::new(
                OAUTH2_INVALID_PROVIDER,
                format!(
                    "Provider with name \"{}\" is missing or is not enabled.",
                    body.provider
                ),
            ),
        );
        return Err(ApiError(AppError::validation(VALIDATION_FAILED, errors)));
    };

    let known = cratebase_auth::KnownProvider::from_name(&body.provider);
    let token_url = effective_url(
        &config.token_url,
        known.map(cratebase_auth::KnownProvider::token_url),
    );
    let user_info_url = effective_url(
        &config.user_info_url,
        known.map(cratebase_auth::KnownProvider::user_info_url),
    );
    if config.client_id.is_empty()
        || config.client_secret.is_empty()
        || token_url.is_empty()
        || user_info_url.is_empty()
    {
        return Err(ApiError::internal(
            "Missing or invalid provider config.".to_string(),
        ));
    }

    let exchange = cratebase_auth::TokenExchange {
        code: &body.code,
        client_id: &config.client_id,
        client_secret: &config.client_secret,
        redirect_uri: &body.redirect_url,
        code_verifier: (!body.code_verifier.is_empty()).then_some(body.code_verifier.as_str()),
    };
    let client = oauth2_client();
    let token_res = client
        .post(&token_url)
        .header("Accept", "application/json")
        .form(&exchange.form())
        .send()
        .await
        .map_err(|_| ApiError::bad_request("Failed to fetch OAuth2 token."))?;
    if !token_res.status().is_success() {
        return Err(ApiError::bad_request("Failed to fetch OAuth2 token."));
    }
    let token_body = token_res.bytes().await.unwrap_or_default();
    let token = cratebase_auth::parse_token_response(&token_body)
        .map_err(|_| ApiError::bad_request("Failed to fetch OAuth2 token."))?;

    let oauth_user = fetch_oauth2_user(client, &user_info_url, known, &token.access_token).await?;

    // A caller already signed in to *this* collection links a second
    // provider onto their own record instead of creating (or matching
    // by email into) a different one.
    let fallback = caller
        .0
        .as_ref()
        .filter(|a| a.collection.id == collection.id)
        .map(|a| a.record.clone());

    let (record, is_new) = resolve_oauth2_record(
        &app,
        &collection,
        &body.provider,
        &oauth_user,
        &collection.auth.oauth2.mapped_fields,
        &body.create_data,
        fallback.as_ref(),
    )
    .await?;

    if !passes_auth_rule(&app, &collection, &record).await? {
        return Err(ApiError::forbidden(AUTH_RULE_FAILED));
    }

    match mfa_gate(&app, &collection, &record, "oauth2", body.mfa_id.as_deref()).await? {
        MfaGate::Pending(mfa_id) => return Ok(mfa_pending_response(mfa_id)),
        MfaGate::Passed => {}
    }

    record_login_origin(&app, &collection, &record, &headers, peer).await;

    let response = respond_with_token(&app, &collection, record, info, raw, |hooks| {
        &hooks.on_record_auth_with_oauth2_request
    })
    .await?;
    let mut rendered = response.0;
    if let Some(obj) = rendered.as_object_mut() {
        obj.insert(
            "meta".into(),
            json!({
                "id": oauth_user.id,
                "name": oauth_user.name,
                "username": oauth_user.username,
                "email": oauth_user.email,
                "avatarURL": oauth_user.avatar_url,
                "isNew": is_new,
            }),
        );
    }
    Ok(Json(rendered).into_response())
}

/// Finds or creates the record `oauth_user` should sign in as, and makes
/// sure an `_externalAuths` row links `provider`+`oauth_user.id` to it —
/// PocketBase's exact precedence: an existing link wins outright; then a
/// record already logged into `collection` in this request (linking a
/// second provider to one account); then a same-email match in
/// `collection` (marked verified, since the provider vouches for the
/// address); and only then a brand-new record via
/// [`create_oauth2_record`].
async fn resolve_oauth2_record(
    app: &App,
    collection: &Arc<Collection>,
    provider: &str,
    oauth_user: &cratebase_auth::OAuth2User,
    mapped: &OAuth2MappedFields,
    create_data: &Map<String, Value>,
    fallback: Option<&Record>,
) -> ApiResult<(Record, bool)> {
    let externals = app
        .db()
        .collections
        .get("_externalAuths")
        .expect("_externalAuths is a default system collection");

    let mut link_params = Map::new();
    link_params.insert("collectionRef".into(), Value::String(collection.id.clone()));
    link_params.insert("provider".into(), Value::String(provider.to_string()));
    link_params.insert("providerId".into(), Value::String(oauth_user.id.clone()));
    let existing_link = records::find_first_by_filter(
        app.db(),
        &app.db().collections,
        &externals,
        "collectionRef = {:collectionRef} && provider = {:provider} && providerId = {:providerId}",
        &link_params,
    )
    .await
    .map_err(|e| ApiError(e.into()))?;

    if let Some(link) = existing_link {
        let record = records::find_by_id_raw(app.db(), collection, &link.get_string("recordRef"))
            .await
            .map_err(|e| ApiError(e.into()))?;
        return Ok((record, false));
    }

    let (record, is_new) = if let Some(fallback) = fallback {
        (fallback.clone(), false)
    } else if !oauth_user.email.is_empty() {
        match find_by_email(app, collection, &oauth_user.email).await? {
            Some(mut found) => {
                if !found.verified() {
                    found.set("verified", Value::Bool(true));
                    records::update(app.db(), &app.db().collections, &mut found)
                        .await
                        .map_err(|e| ApiError(e.into()))?;
                }
                (found, false)
            }
            None => (
                create_oauth2_record(app, collection, mapped, oauth_user, create_data).await?,
                true,
            ),
        }
    } else {
        (
            create_oauth2_record(app, collection, mapped, oauth_user, create_data).await?,
            true,
        )
    };

    let mut link = Record::new(externals.clone());
    link.set("collectionRef", Value::String(collection.id.clone()));
    link.set("recordRef", Value::String(record.id().to_string()));
    link.set("provider", Value::String(provider.to_string()));
    link.set("providerId", Value::String(oauth_user.id.clone()));
    records::create(app.db(), &app.db().collections, &mut link)
        .await
        .map_err(|e| ApiError(e.into()))?;

    Ok((record, is_new))
}

/// Creates a fresh record in `collection` for a first-time OAuth2
/// sign-in: `create_data` first (caller-supplied, e.g. extra custom
/// fields), then whichever of the collection's `oauth2.mappedFields`
/// aren't already set from it, then a random unguessable password (this
/// record only ever signs back in through OAuth2 or a password reset)
/// and `verified: true` — the provider vouched for this identity, which
/// is a stronger claim than cratebase's own click-the-link verification.
async fn create_oauth2_record(
    app: &App,
    collection: &Arc<Collection>,
    mapped: &OAuth2MappedFields,
    oauth_user: &cratebase_auth::OAuth2User,
    create_data: &Map<String, Value>,
) -> ApiResult<Record> {
    if collection.name == cratebase_core::SUPERUSERS_COLLECTION {
        return Err(ApiError::bad_request(
            "Superusers are not allowed to sign up with OAuth2.",
        ));
    }
    let mut body = create_data.clone();
    body.entry("email".to_string())
        .or_insert_with(|| Value::String(oauth_user.email.clone()));
    let mut assign = |field: &str, value: &str| {
        if !field.is_empty()
            && !value.is_empty()
            && collection.has_field(field)
            && !body.contains_key(field)
        {
            body.insert(field.to_string(), Value::String(value.to_string()));
        }
    };
    assign(&mapped.id, &oauth_user.id);
    assign(&mapped.name, &oauth_user.name);
    assign(&mapped.username, &oauth_user.username);
    assign(&mapped.avatar_url, &oauth_user.avatar_url);
    body.insert(
        "password".into(),
        Value::String(cratebase_auth::random_alphanumeric(40)),
    );
    body.insert("verified".into(), Value::Bool(true));

    let mut record = records::from_body(collection.clone(), &body);
    record.set("tokenKey", Value::String(crate::app::new_token_key()));
    records::create(app.db(), &app.db().collections, &mut record)
        .await
        .map_err(|e| ApiError(e.into()))?;
    Ok(record)
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

#[cfg(test)]
mod oauth2_tests {
    //! [`resolve_oauth2_record`] is where the "new record vs. an existing
    //! linked/matched one" decision actually lives, and it takes an
    //! already-parsed [`cratebase_auth::OAuth2User`] — no HTTP. Exercising
    //! it directly against a real (in-memory) database covers the exact
    //! branching PocketBase's own `oauth2Submit` implements, without
    //! needing to mock `reqwest` or reach a real provider; the
    //! token-exchange/userinfo-parsing half of the flow is covered in
    //! `cratebase_auth::oauth2`'s own tests.
    use cratebase_core::OAuth2Provider;
    use serde_json::Map;

    use super::*;

    /// A bootstrapped app over an in-memory database, mirroring
    /// `api_keys`'s test harness.
    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(crate::config::Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    /// Enables OAuth2 on `name` with one hand-configured "custom"
    /// provider, the way the dashboard's auth-options editor would save
    /// it, and hands back the updated collection.
    async fn oauth2_enabled(app: &App, name: &str) -> Arc<Collection> {
        let mut collection = (*app
            .db()
            .collections
            .get_by_name(name)
            .unwrap_or_else(|| panic!("{name} is a default collection")))
        .clone();
        collection.auth.oauth2.enabled = true;
        collection.auth.oauth2.providers.push(OAuth2Provider {
            name: "custom".into(),
            client_id: "cid".into(),
            client_secret: "csecret".into(),
            auth_url: "https://provider.example/authorize".into(),
            token_url: "https://provider.example/token".into(),
            user_info_url: "https://provider.example/userinfo".into(),
            display_name: "Custom".into(),
            pkce: Some(true),
            extra: Map::new(),
        });
        app.db()
            .collections
            .update(&*app.db().engine, &collection)
            .await
            .expect("enable oauth2");
        app.db().collections.get_by_name(name).expect("reload")
    }

    fn oauth_user(id: &str, email: &str) -> cratebase_auth::OAuth2User {
        cratebase_auth::OAuth2User {
            id: id.into(),
            name: "Jo March".into(),
            username: String::new(),
            email: email.into(),
            avatar_url: String::new(),
        }
    }

    #[tokio::test]
    async fn first_login_creates_a_new_verified_linked_record() {
        let (app, _dir) = test_app().await;
        let collection = oauth2_enabled(&app, "users").await;
        let mapped = collection.auth.oauth2.mapped_fields.clone();

        let (record, is_new) = resolve_oauth2_record(
            &app,
            &collection,
            "custom",
            &oauth_user("provider-1", "jo@example.com"),
            &mapped,
            &Map::new(),
            None,
        )
        .await
        .expect("resolve");

        assert!(is_new);
        assert_eq!(record.email(), "jo@example.com");
        assert!(record.verified());
        // A random password was set, so the OAuth2-only account is still
        // a well-formed auth record (never left with an empty hash).
        assert!(!record.password_hash().is_empty());
    }

    #[tokio::test]
    async fn a_repeat_login_from_the_same_provider_reuses_the_linked_record() {
        let (app, _dir) = test_app().await;
        let collection = oauth2_enabled(&app, "users").await;
        let mapped = collection.auth.oauth2.mapped_fields.clone();
        let user = oauth_user("provider-1", "jo@example.com");

        let (first, first_is_new) = resolve_oauth2_record(
            &app,
            &collection,
            "custom",
            &user,
            &mapped,
            &Map::new(),
            None,
        )
        .await
        .expect("first login");
        let (second, second_is_new) = resolve_oauth2_record(
            &app,
            &collection,
            "custom",
            &user,
            &mapped,
            &Map::new(),
            None,
        )
        .await
        .expect("second login");

        assert!(first_is_new);
        assert!(!second_is_new);
        assert_eq!(first.id(), second.id());
    }

    #[tokio::test]
    async fn a_different_provider_with_the_same_email_links_onto_the_existing_record() {
        let (app, _dir) = test_app().await;
        let collection = oauth2_enabled(&app, "users").await;
        let mapped = collection.auth.oauth2.mapped_fields.clone();

        let (first, _) = resolve_oauth2_record(
            &app,
            &collection,
            "custom",
            &oauth_user("custom-1", "jo@example.com"),
            &mapped,
            &Map::new(),
            None,
        )
        .await
        .expect("first provider");
        let (second, second_is_new) = resolve_oauth2_record(
            &app,
            &collection,
            "github",
            &oauth_user("github-1", "jo@example.com"),
            &mapped,
            &Map::new(),
            None,
        )
        .await
        .expect("second provider, same email");

        assert!(!second_is_new);
        assert_eq!(first.id(), second.id());
    }

    #[tokio::test]
    async fn an_already_logged_in_caller_links_a_second_provider_to_their_own_record() {
        let (app, _dir) = test_app().await;
        let collection = oauth2_enabled(&app, "users").await;
        let mapped = collection.auth.oauth2.mapped_fields.clone();

        let (existing, _) = resolve_oauth2_record(
            &app,
            &collection,
            "custom",
            &oauth_user("custom-1", "jo@example.com"),
            &mapped,
            &Map::new(),
            None,
        )
        .await
        .expect("seed record");

        // A different provider *and* a different email — only the fact
        // that the caller is already signed in as `existing` should
        // matter here, never the incoming email.
        let (linked, is_new) = resolve_oauth2_record(
            &app,
            &collection,
            "github",
            &oauth_user("github-1", "someone-else@example.com"),
            &mapped,
            &Map::new(),
            Some(&existing),
        )
        .await
        .expect("link second provider");

        assert!(!is_new);
        assert_eq!(linked.id(), existing.id());
    }

    #[tokio::test]
    async fn create_data_and_mapped_fields_seed_a_new_record() {
        let (app, _dir) = test_app().await;
        let collection = oauth2_enabled(&app, "users").await;
        let mut mapped = collection.auth.oauth2.mapped_fields.clone();
        mapped.name = "name".into();

        let mut create_data = Map::new();
        create_data.insert("name".into(), Value::String("Explicit Name".into()));

        let (record, _) = resolve_oauth2_record(
            &app,
            &collection,
            "custom",
            &oauth_user("provider-2", "explicit@example.com"),
            &mapped,
            &create_data,
            None,
        )
        .await
        .expect("resolve");

        // `createData` wins over the OAuth2-mapped value for a field it
        // already sets.
        assert_eq!(record.get_string("name"), "Explicit Name");
    }

    #[tokio::test]
    async fn superusers_cannot_sign_up_via_oauth2() {
        let (app, _dir) = test_app().await;
        let collection = oauth2_enabled(&app, cratebase_core::SUPERUSERS_COLLECTION).await;
        let mapped = collection.auth.oauth2.mapped_fields.clone();

        let result = resolve_oauth2_record(
            &app,
            &collection,
            "custom",
            &oauth_user("provider-1", "new-superuser@example.com"),
            &mapped,
            &Map::new(),
            None,
        )
        .await;

        assert!(result.is_err());
    }
}
