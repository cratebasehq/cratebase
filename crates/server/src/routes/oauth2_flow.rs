//! `GET .../oauth2/{provider}/start` and `.../oauth2/{provider}/callback`:
//! the server-driven OAuth2 login flow. No popup, no client-side code
//! exchange — the browser is redirected straight to the provider and
//! straight back, and the session lands in an httpOnly cookie.
//! Bearer-only deployments are unaffected: they keep using
//! `auth-methods` + `POST .../auth-with-oauth2`
//! (`crate::routes::auth::auth_with_oauth2`), which shares this module's
//! [`crate::routes::auth::complete_oauth2`] so the two flows can never
//! diverge on the security-critical parts of an OAuth2 login
//! (`resolve_oauth2_record`'s pre-hijacking protection, `createRule`
//! enforcement, `_externalAuths` linking).
//!
//! # Requires cookie sessions
//!
//! This flow only exists to land a session in a cookie, so it refuses
//! outright — via `?cb_error=cookie_sessions_disabled` — when
//! `Config::session_cookie` is off. A bearer-only deployment has no use
//! for it.
//!
//! # Flow state, without a server-side session table
//!
//! `start` never touches the database. The provider `state`, the PKCE
//! verifier, the caller's `redirect` target and `createData` are packed
//! into a short-lived (10 minute) JWT — `type: "oauth2State"`, signed
//! with `signing_key(app_secret, "", "oauth2State")` (no record is
//! involved, unlike an ordinary session token, since this token
//! authenticates nothing by itself) — carried in the `cb_oauth_state`
//! cookie. `callback` verifies that cookie instead of a lookup.
//!
//! # `redirect` must be trusted
//!
//! `start`'s `redirect` query parameter is where the browser lands after
//! login — an unchecked value would make this endpoint an open redirect.
//! It is accepted only when its origin equals `settings.meta.appURL`'s
//! origin or is an exact member of `Config::origins` (a `"*"` entry does
//! not qualify, since the default config ships with exactly that
//! wildcard). This is the one case that answers `400` JSON instead of
//! redirecting — redirecting to an untrusted target *is* the open
//! redirect this check exists to prevent. Every other failure past that
//! point redirects back to the now-trusted `redirect` with
//! `?cb_error=<code>`, so `signIn.social()` can be a single
//! `location.assign()` with no probe request.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cratebase_auth::{Claims, TokenType};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::app::App;
use crate::extract::RequestInfo;
use crate::http_error::{ApiError, ApiQuery, ApiResult};
use crate::routes::auth::{complete_oauth2, record_login_origin, MfaGate};
use crate::routes::common;

pub fn router() -> Router<App> {
    Router::new()
        .route(
            "/collections/{collection}/oauth2/{provider}/start",
            get(start),
        )
        .route(
            "/collections/{collection}/oauth2/{provider}/callback",
            get(callback),
        )
}

/// `Claims::token_type` for the short-lived flow-state JWT.
const STATE_TOKEN_TYPE: &str = "oauth2State";
/// Ten minutes — long enough for a real login, short enough that a
/// captured `cb_oauth_state` cookie is worthless soon after.
const STATE_TTL: i64 = 600;
const STATE_COOKIE: &str = "cb_oauth_state";

#[derive(Debug, Deserialize)]
struct StartQuery {
    redirect: String,
    /// Base64url (no padding) JSON object, matching `auth-with-oauth2`'s
    /// `createData` body field.
    #[serde(rename = "createData")]
    create_data: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct CallbackQuery {
    #[serde(default)]
    code: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    error: String,
}

/// A URL's `scheme://host[:port]`, for same-origin comparison. `None`
/// for anything that doesn't parse as an absolute URL.
fn origin_of(raw: &str) -> Option<String> {
    let u = reqwest::Url::parse(raw).ok()?;
    (u.scheme() == "http" || u.scheme() == "https").then(|| u.origin().ascii_serialization())
}

/// Whether `redirect` is same-origin with `app_url` (when configured) or
/// an exact member of `extra_origins` (`"*"` never qualifies).
fn redirect_is_trusted(redirect: &str, app_url: &str, extra_origins: &[String]) -> bool {
    let Some(target) = origin_of(redirect) else {
        return false;
    };
    if !app_url.trim().is_empty() && origin_of(app_url).as_deref() == Some(target.as_str()) {
        return true;
    }
    extra_origins
        .iter()
        .any(|o| o != "*" && origin_of(o).as_deref() == Some(target.as_str()))
}

/// The exact callback URL a provider is registered against and that
/// `start`/`callback` must each independently reconstruct identically —
/// providers reject a token exchange whose `redirect_uri` doesn't match
/// the one the authorize request used byte-for-byte.
fn callback_url(app_url: &str, collection: &str, provider: &str) -> String {
    format!("{}/api/collections/{collection}/oauth2/{provider}/callback", app_url.trim_end_matches('/'))
}

fn append_query(url: &str, key: &str, value: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(mut u) => {
            u.query_pairs_mut().append_pair(key, value);
            u.to_string()
        }
        Err(_) => url.to_string(),
    }
}

async fn start(
    State(app): State<App>,
    Path((name, provider)): Path<(String, String)>,
    ApiQuery(q): ApiQuery<StartQuery>,
) -> ApiResult<Response> {
    let collection = common::auth_collection_of(&app, &name)?;
    let cfg = app.config();
    let settings = app.settings();

    // The one failure that must answer JSON, not redirect.
    if !redirect_is_trusted(&q.redirect, &settings.meta.app_url, &cfg.origins) {
        return Err(ApiError::bad_request("Untrusted redirect target."));
    }

    let bail = |code: &str| Ok(Redirect::to(&append_query(&q.redirect, "cb_error", code)).into_response());

    if !cfg.session_cookie {
        return bail("cookie_sessions_disabled");
    }
    if settings.meta.app_url.trim().is_empty() {
        return bail("app_url_not_configured");
    }
    if !collection.auth.oauth2.enabled {
        return bail("oauth2_disabled");
    }
    let Some(config) = collection
        .auth
        .oauth2
        .providers
        .iter()
        .find(|p| p.name == provider)
    else {
        return bail("provider_not_enabled");
    };

    let state = cratebase_auth::random_state();
    let pkce = config.pkce.unwrap_or(true);
    let (code_verifier, code_challenge) = if pkce {
        let verifier = cratebase_auth::code_verifier();
        let challenge = cratebase_auth::code_challenge_s256(&verifier);
        (verifier, challenge)
    } else {
        (String::new(), String::new())
    };

    let redirect_uri = callback_url(&settings.meta.app_url, &name, &provider);
    let Some(auth_url) =
        crate::routes::auth::provider_auth_url(config, &state, &code_challenge, &redirect_uri)
    else {
        return bail("provider_not_enabled");
    };

    let create_data = q.create_data.as_deref().unwrap_or("");
    let claims = Claims::new(
        state,
        TokenType::Custom(STATE_TOKEN_TYPE.into()),
        collection.id.clone(),
        STATE_TTL,
    )
    .with_extra("provider", provider)
    .with_extra("codeVerifier", code_verifier)
    .with_extra("redirect", q.redirect)
    .with_extra("createData", create_data);
    let key = cratebase_auth::signing_key(&cfg.secret, "", STATE_TOKEN_TYPE);
    let token =
        cratebase_auth::sign(&claims, &key).map_err(|e| ApiError::internal(e.to_string()))?;

    let mut response = Redirect::to(&auth_url).into_response();
    crate::cookie::attach(
        response.headers_mut(),
        crate::cookie::build(cfg, STATE_COOKIE, &token, STATE_TTL, "/api"),
    );
    Ok(response)
}

async fn callback(
    State(app): State<App>,
    Path((name, provider)): Path<(String, String)>,
    parts: axum::http::request::Parts,
    peer: crate::middleware::client_ip::PeerAddr,
    ApiQuery(q): ApiQuery<CallbackQuery>,
    info: RequestInfo,
) -> ApiResult<Response> {
    let collection = common::auth_collection_of(&app, &name)?;
    let cfg = app.config();

    // No trusted `redirect` is recoverable without a valid state token —
    // there is nowhere safe to send the browser, so this is the one
    // other case (besides `start`'s untrusted-redirect check) that
    // answers JSON.
    let raw_state_token = crate::cookie::get(&parts, STATE_COOKIE)
        .ok_or_else(|| ApiError::bad_request("Missing or expired OAuth2 state."))?
        .to_string();
    let key = cratebase_auth::signing_key(&cfg.secret, "", STATE_TOKEN_TYPE);
    let claims = cratebase_auth::verify(&raw_state_token, &key)
        .map_err(|_| ApiError::bad_request("Invalid or expired OAuth2 state."))?;
    if claims.token_type != TokenType::Custom(STATE_TOKEN_TYPE.into())
        || claims.id != q.state
        || claims.collection_id != collection.id
    {
        return Err(ApiError::bad_request("Invalid or expired OAuth2 state."));
    }
    let state_provider = claims.extra_str("provider").unwrap_or_default().to_string();
    let code_verifier = claims.extra_str("codeVerifier").unwrap_or_default().to_string();
    let redirect = claims.extra_str("redirect").unwrap_or_default().to_string();
    let create_data_b64 = claims.extra_str("createData").unwrap_or_default().to_string();
    if state_provider != provider {
        return Err(ApiError::bad_request("Invalid or expired OAuth2 state."));
    }

    let clear_state = |mut response: Response| -> Response {
        crate::cookie::attach(
            response.headers_mut(),
            crate::cookie::build(cfg, STATE_COOKIE, "", 0, "/api"),
        );
        response
    };

    if !q.error.is_empty() {
        let r = Redirect::to(&append_query(&redirect, "cb_error", &q.error)).into_response();
        return Ok(clear_state(r));
    }

    let create_data: Map<String, Value> = if create_data_b64.is_empty() {
        Map::new()
    } else {
        URL_SAFE_NO_PAD
            .decode(&create_data_b64)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    };
    let redirect_uri = callback_url(&app.settings().meta.app_url, &name, &provider);

    let outcome = complete_oauth2(
        &app,
        &collection,
        &provider,
        &q.code,
        &code_verifier,
        &redirect_uri,
        create_data,
        None,
        None,
    )
    .await?;

    let record = match outcome.mfa {
        MfaGate::Pending(mfa_id) => {
            let r = Redirect::to(&append_query(&redirect, "cb_mfa", &mfa_id)).into_response();
            return Ok(clear_state(r));
        }
        MfaGate::Passed => outcome.record,
    };

    let origin =
        record_login_origin(&app, &collection, &record, &parts.headers, peer).await;

    let (token, record) = crate::routes::auth::mint_and_record(
        &app,
        &collection,
        record,
        info,
        Value::Object(Map::new()),
        |hooks| &hooks.on_record_auth_with_oauth2_request,
        Some(("oauth2", origin)),
    )
    .await?;
    let _ = record;

    let ttl = collection.auth.auth_token.duration.max(1);
    let mut response = clear_state(Redirect::to(&redirect).into_response());
    crate::cookie::attach(
        response.headers_mut(),
        crate::cookie::session(cfg, &token, ttl),
    );
    Ok(response)
}
