//! Request extractors: who is calling ([`Auth`], [`RequireSuperuser`])
//! and the `@request.*` context that API rules are evaluated against
//! ([`RequestInfo`]).
//!
//! # Token resolution
//!
//! PocketBase signs a record's token with `secret + record.tokenKey`, so
//! the key cannot be known until the record is loaded — and the record id
//! is inside the token. Resolution is therefore three steps:
//!
//! 1. decode the JWT **without** verifying, purely to read `id` and
//!    `collectionId`;
//! 2. load that record and read its `tokenKey`;
//! 3. re-derive the signing key and *verify* for real.
//!
//! Step 1's claims are never trusted for anything but the lookup. The
//! upside of the scheme is that rotating a record's `tokenKey` (on a
//! password or email change) invalidates every outstanding session for
//! it, with no server-side session table.
//!
//! The result is cached in the request extensions, because the logging
//! and rate-limit layers both resolve the caller before the handler does
//! and each resolution costs a verify plus a database round trip.

use std::collections::BTreeMap;

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use cratebase_core::AppError;
use serde_json::{Map, Value};

use crate::app::App;
use crate::http_error::ApiError;

/// The resolved caller.
#[derive(Clone, Debug)]
pub struct Auth {
    pub id: String,
    pub collection_id: String,
    /// `_superusers`, `users`, ... — what the request log's `auth` field
    /// and `@request.auth.collectionName` report.
    pub collection_name: String,
    pub is_superuser: bool,
    /// The record's stored columns. W4b: replace with the decoded
    /// `cratebase_core::Record` once `cratebase_db::records` lands, so
    /// rules see typed values rather than raw columns.
    pub record: Map<String, Value>,
}

impl Auth {
    /// `@request.auth` as the filter compiler expects it.
    pub fn to_filter_value(&self) -> Value {
        let mut map = self.record.clone();
        map.insert("id".into(), Value::String(self.id.clone()));
        map.insert(
            "collectionId".into(),
            Value::String(self.collection_id.clone()),
        );
        map.insert(
            "collectionName".into(),
            Value::String(self.collection_name.clone()),
        );
        // Never exposed to rules, per spec §6.
        map.remove("password");
        map.remove("tokenKey");
        Value::Object(map)
    }
}

/// Cached resolution result; `None` means anonymous.
#[derive(Clone, Debug, Default)]
struct CachedAuth(Option<Auth>);

fn bearer_token(parts: &Parts) -> Option<&str> {
    let raw = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    // PocketBase accepts both `Bearer <token>` and a bare token.
    Some(raw.strip_prefix("Bearer ").unwrap_or(raw).trim()).filter(|t| !t.is_empty())
}

/// Resolve the caller once and cache it on the request.
pub async fn resolve_and_cache(parts: &mut Parts, app: &App) -> Option<Auth> {
    if let Some(cached) = parts.extensions.get::<CachedAuth>() {
        return cached.0.clone();
    }
    let resolved = resolve(parts, app).await;
    parts.extensions.insert(CachedAuth(resolved.clone()));
    resolved
}

async fn resolve(parts: &Parts, app: &App) -> Option<Auth> {
    let token = bearer_token(parts)?;
    // Everything past this point costs a signature check and a query, so
    // this is where "how often did we really resolve?" is counted.
    app.note_auth_resolution();
    // Unverified: only ever used to find which record's key to use.
    let unverified = cratebase_auth::decode_unverified(token).ok()?;
    if unverified.token_type != cratebase_auth::TokenType::Auth {
        // A verification / reset / file token is single-purpose and must
        // never authenticate an ordinary request.
        return None;
    }
    let collection = app.db().collections.get_by_id(&unverified.collection_id)?;

    if collection.name != cratebase_core::SUPERUSERS_COLLECTION {
        // W4b: load the record through `records::find_by_id_raw`, verify
        // against its `tokenKey`, and build `Auth` from the decoded
        // record. Until W3 lands only superusers can authenticate, which
        // is all the routes live in W4a require.
        return None;
    }

    let row = app.find_superuser_by_id(&unverified.id).await.ok()??;
    let mut record = Map::new();
    for (column, value) in row.into_pairs() {
        record.insert(column, sql_to_json(value));
    }
    let token_key = record.get("tokenKey").and_then(Value::as_str).unwrap_or("");
    let key = app.token_signing_key(token_key, &collection.auth.auth_token.secret);
    let claims = cratebase_auth::verify(token, &key).ok()?;
    if claims.id != unverified.id {
        return None;
    }

    Some(Auth {
        id: claims.id,
        collection_id: collection.id.clone(),
        collection_name: collection.name.clone(),
        is_superuser: true,
        record,
    })
}

fn sql_to_json(value: cratebase_db::Sql) -> Value {
    use cratebase_db::Sql;
    match value {
        Sql::Null => Value::Null,
        Sql::Int(i) => Value::from(i),
        Sql::Real(f) => Value::from(f),
        Sql::Text(t) => Value::String(t),
        Sql::Blob(b) => Value::from(b.len()),
    }
}

impl<S> FromRequestParts<S> for Auth
where
    App: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = App::from_ref(state);
        resolve_and_cache(parts, &app)
            .await
            .ok_or_else(|| ApiError(AppError::unauthorized("")))
    }
}

/// The caller, or `None` when the request is anonymous. Never rejects —
/// most collection rules are perfectly happy with a public caller.
#[derive(Clone, Debug, Default)]
pub struct MaybeAuth(pub Option<Auth>);

impl<S> FromRequestParts<S> for MaybeAuth
where
    App: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = App::from_ref(state);
        Ok(MaybeAuth(resolve_and_cache(parts, &app).await))
    }
}

/// Rejects anything but an authenticated `_superusers` record. Also
/// enforces `settings.superuserIPs`, PocketBase's allowlist for
/// superuser traffic.
pub struct RequireSuperuser(pub Auth);

impl<S> FromRequestParts<S> for RequireSuperuser
where
    App: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = App::from_ref(state);
        // PocketBase distinguishes "no token" (401, the generic message
        // the SDK asserts on) from "a token, but not a superuser's" (403).
        let auth = match resolve_and_cache(parts, &app).await {
            Some(auth) if auth.is_superuser => auth,
            Some(_) => {
                return Err(ApiError(AppError::Forbidden(
                    "The request requires superuser authorization token to be set.".into(),
                )))
            }
            None => return Err(ApiError(AppError::unauthorized(""))),
        };

        let settings = app.settings();
        if !settings.superuser_ips.is_empty() {
            let peer = parts
                .extensions
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|ci| ci.0);
            let ip = crate::middleware::client_ip::client_ip(
                &parts.headers,
                peer,
                &settings.trusted_proxy,
            );
            if !settings.superuser_ips.contains(&ip) {
                return Err(ApiError(AppError::forbidden("")));
            }
        }
        Ok(RequireSuperuser(auth))
    }
}

/// The `@request.*` context an API rule is compiled against.
///
/// W4b: convert into `cratebase_db::context::RequestContext` once W3
/// lands — the field set here is deliberately the same so the conversion
/// is a `From` impl and nothing above has to change.
#[derive(Clone, Debug, Default)]
pub struct RequestInfo {
    pub method: String,
    pub path: String,
    pub query: Map<String, Value>,
    /// Header names lowercased with `-` replaced by `_`, PocketBase's
    /// convention (`@request.headers.x_token`).
    pub headers: Map<String, Value>,
    /// Parsed request body; filled by the routes that read one.
    pub body: Map<String, Value>,
    /// `default`, `realtime`, `protectedFile`, `oauth2` or `batch`.
    pub context: String,
    pub auth: Option<Auth>,
}

/// PocketBase's default request context.
pub const CONTEXT_DEFAULT: &str = "default";

impl RequestInfo {
    pub fn with_body(mut self, body: Map<String, Value>) -> Self {
        self.body = body;
        self
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = context.into();
        self
    }

    /// `@request.*` as one JSON object, handy for logs and for the JS
    /// runtime's `e.requestInfo()`.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "method": self.method,
            "query": self.query,
            "headers": self.headers,
            "body": self.body,
            "context": self.context,
            "auth": self.auth.as_ref().map(Auth::to_filter_value),
        })
    }
}

impl<S> FromRequestParts<S> for RequestInfo
where
    App: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = App::from_ref(state);
        let auth = resolve_and_cache(parts, &app).await;
        Ok(RequestInfo {
            method: parts.method.to_string(),
            path: parts.uri.path().to_string(),
            query: query_map(parts.uri.query().unwrap_or_default()),
            headers: header_map(&parts.headers),
            body: Map::new(),
            context: CONTEXT_DEFAULT.to_string(),
            auth,
        })
    }
}

/// Query string → a flat map. Repeated keys keep the last value, as
/// PocketBase's `echo` binding does.
fn query_map(raw: &str) -> Map<String, Value> {
    let mut out = Map::new();
    for pair in raw.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(percent_decode(key), Value::String(percent_decode(value)));
    }
    out
}

/// Header names lowercased with `-` → `_`, values joined by `, `.
fn header_map(headers: &axum::http::HeaderMap) -> Map<String, Value> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, value) in headers {
        let Ok(value) = value.to_str() else { continue };
        grouped
            .entry(name.as_str().replace('-', "_"))
            .or_default()
            .push(value.to_string());
    }
    grouped
        .into_iter()
        .map(|(k, v)| (k, Value::String(v.join(", "))))
        .collect()
}

/// Minimal `application/x-www-form-urlencoded` decoding (`+` is a space,
/// `%XX` a byte). Invalid escapes are left as written, matching Go's
/// lenient behaviour closely enough for rule inputs.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&raw[i + 1..i + 3], 16) {
                Ok(byte) => {
                    out.push(byte);
                    i += 3;
                }
                Err(_) => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn query_strings_are_decoded() {
        let q = query_map("filter=title%3D%22a+b%22&page=2");
        assert_eq!(q["filter"], "title=\"a b\"");
        assert_eq!(q["page"], "2");
        assert!(query_map("").is_empty());
    }

    #[test]
    fn header_names_use_pocketbase_spelling() {
        let mut h = HeaderMap::new();
        h.insert("X-Token", HeaderValue::from_static("abc"));
        h.append("X-Token", HeaderValue::from_static("def"));
        let map = header_map(&h);
        assert_eq!(map["x_token"], "abc, def");
    }

    #[test]
    fn auth_filter_value_hides_credentials() {
        let auth = Auth {
            id: "u1".into(),
            collection_id: "c1".into(),
            collection_name: "users".into(),
            is_superuser: false,
            record: serde_json::from_value(serde_json::json!({
                "password": "hash", "tokenKey": "k", "email": "a@b.c"
            }))
            .unwrap(),
        };
        let v = auth.to_filter_value();
        assert_eq!(v["id"], "u1");
        assert_eq!(v["collectionName"], "users");
        assert_eq!(v["email"], "a@b.c");
        assert!(v.get("password").is_none());
        assert!(v.get("tokenKey").is_none());
    }
}
