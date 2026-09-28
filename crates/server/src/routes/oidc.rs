//! Generic OpenID Connect provider support: discovery-document + JWKS
//! fetch-and-cache for an issuer-only-configured provider (any
//! `OAuth2Provider` with a non-empty `extra.issuer` — conventionally
//! named `oidc`/`oidc2`/`oidc3`, matching PocketBase's own convention for
//! more than one hand-configured OIDC issuer), and for Apple's fixed
//! JWKS (`https://appleid.apple.com/auth/keys`) used to verify its own
//! `id_token`.
//!
//! Caching is process-local and unbounded (an admin configures at most a
//! handful of providers), keyed by issuer/JWKS URL, with a 1 hour TTL —
//! long enough that steady-state logins never hit the network, short
//! enough that a provider's outage or a config change is noticed the
//! same day. [`fetch_jwks`]'s `force_refresh` handles the case a normal
//! TTL wouldn't: a signing key rotated *within* the cache window, which
//! shows up as a `kid` [`cratebase_auth::verify_id_token`] doesn't
//! recognize.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::http_error::{ApiError, ApiResult};

const CACHE_TTL: Duration = Duration::from_secs(3600);

/// The subset of a `/.well-known/openid-configuration` response cratebase
/// needs to run an authorization-code + PKCE login against the issuer.
#[derive(Debug, Clone, Deserialize)]
pub struct OidcDiscovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub userinfo_endpoint: String,
    pub jwks_uri: String,
}

struct CacheEntry<T> {
    at: Instant,
    value: T,
}

static DISCOVERY_CACHE: LazyLock<Mutex<HashMap<String, CacheEntry<OidcDiscovery>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static JWKS_CACHE: LazyLock<Mutex<HashMap<String, CacheEntry<Vec<u8>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `https://issuer` → its discovery document, cached. `issuer` is
/// normalized by trimming a trailing slash before the well-known path is
/// appended, matching how every real-world issuer publishes it.
pub async fn discover(client: &reqwest::Client, issuer: &str) -> ApiResult<OidcDiscovery> {
    let issuer = issuer.trim_end_matches('/').to_string();
    if let Some(entry) = DISCOVERY_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&issuer)
    {
        if entry.at.elapsed() < CACHE_TTL {
            return Ok(entry.value.clone());
        }
    }
    let url = format!("{issuer}/.well-known/openid-configuration");
    let res = client
        .get(&url)
        .send()
        .await
        .map_err(|_| ApiError::bad_request("Failed to fetch the OIDC discovery document."))?;
    if !res.status().is_success() {
        return Err(ApiError::bad_request(
            "Failed to fetch the OIDC discovery document.",
        ));
    }
    let doc: OidcDiscovery = res
        .json()
        .await
        .map_err(|_| ApiError::bad_request("Invalid OIDC discovery document."))?;
    DISCOVERY_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            issuer,
            CacheEntry {
                at: Instant::now(),
                value: doc.clone(),
            },
        );
    Ok(doc)
}

/// Fetches `jwks_uri`'s raw JSON body, using a cached copy unless it's
/// stale or `force_refresh` is set.
pub async fn fetch_jwks(
    client: &reqwest::Client,
    jwks_uri: &str,
    force_refresh: bool,
) -> ApiResult<Vec<u8>> {
    if !force_refresh {
        if let Some(entry) = JWKS_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(jwks_uri)
        {
            if entry.at.elapsed() < CACHE_TTL {
                return Ok(entry.value.clone());
            }
        }
    }
    let res = client
        .get(jwks_uri)
        .send()
        .await
        .map_err(|_| ApiError::bad_request("Failed to fetch the provider's JWKS."))?;
    if !res.status().is_success() {
        return Err(ApiError::bad_request(
            "Failed to fetch the provider's JWKS.",
        ));
    }
    let body = res.bytes().await.unwrap_or_default().to_vec();
    JWKS_CACHE.lock().unwrap_or_else(|e| e.into_inner()).insert(
        jwks_uri.to_string(),
        CacheEntry {
            at: Instant::now(),
            value: body.clone(),
        },
    );
    Ok(body)
}

/// Apple's fixed (never discovered — it has no `/.well-known/...`
/// document PocketBase-style providers rely on) JWKS + issuer.
pub const APPLE_JWKS_URL: &str = "https://appleid.apple.com/auth/keys";
pub const APPLE_ISSUER: &str = "https://appleid.apple.com";

/// Verifies `id_token` against `jwks_uri`, retrying once with a forced
/// cache refresh when the first attempt fails — covers both a genuinely
/// invalid token and a same-cache-window key rotation, since
/// [`cratebase_auth::verify_id_token`] doesn't distinguish the two and a
/// wrong signature fails the retry identically anyway.
pub async fn verify_id_token_cached(
    client: &reqwest::Client,
    jwks_uri: &str,
    id_token: &str,
    checks: &cratebase_auth::IdTokenChecks<'_>,
) -> ApiResult<serde_json::Value> {
    let jwks = fetch_jwks(client, jwks_uri, false).await?;
    if let Ok(claims) = cratebase_auth::verify_id_token(id_token, &jwks, checks) {
        return Ok(claims);
    }
    let jwks = fetch_jwks(client, jwks_uri, true).await?;
    cratebase_auth::verify_id_token(id_token, &jwks, checks)
        .map_err(|e| ApiError::bad_request(format!("Invalid OAuth2 identity token: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_KID: &str = "test-key-1";
    const TEST_PRIVATE_KEY_PEM: &str = include_str!("../../testdata/oidc_test_rsa_private.pem");
    const TEST_JWKS_JSON: &str = include_str!("../../testdata/oidc_test_jwks.json");

    fn sign_test_token(issuer: &str, audience: &str) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(TEST_KID.to_string());
        let key = EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).unwrap();
        let claims = json!({
            "iss": issuer,
            "aud": audience,
            "sub": "user-1",
            "email": "jo@example.com",
            "email_verified": true,
            "exp": 9_999_999_999i64,
            "iat": 1_700_000_000,
        });
        jsonwebtoken::encode(&header, &claims, &key).unwrap()
    }

    /// End to end: a discovered issuer's `/.well-known/...` document
    /// points at a JWKS endpoint, and a token signed with that JWKS's
    /// key verifies successfully against it — the full path a real
    /// "just an issuer URL" OIDC provider login exercises, minus the
    /// authorization-code exchange itself (`fetch_oidc_user`'s own unit
    /// coverage, which mocks the HTTP layer at a higher level, covers
    /// that).
    #[tokio::test]
    async fn discovers_and_verifies_against_a_mocked_issuer() {
        let server = MockServer::start().await;
        let issuer = server.uri();
        let jwks_uri = format!("{issuer}/jwks");
        let discovery_doc = json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{issuer}/token"),
            "userinfo_endpoint": format!("{issuer}/userinfo"),
            "jwks_uri": jwks_uri,
        });
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&discovery_doc))
            .mount(&server)
            .await;
        let jwks: serde_json::Value = serde_json::from_str(TEST_JWKS_JSON).unwrap();
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let discovered = discover(&client, &issuer).await.unwrap();
        assert_eq!(discovered.jwks_uri, jwks_uri);
        assert_eq!(discovered.token_endpoint, format!("{issuer}/token"));

        let token = sign_test_token(&issuer, "test-client");
        let checks = cratebase_auth::IdTokenChecks {
            issuer: &discovered.issuer,
            audience: "test-client",
            nonce: None,
        };
        let claims = verify_id_token_cached(&client, &discovered.jwks_uri, &token, &checks)
            .await
            .unwrap();
        assert_eq!(claims["sub"], "user-1");
        assert_eq!(claims["email"], "jo@example.com");
    }

    #[tokio::test]
    async fn discovery_is_cached_and_only_fetched_once() {
        let server = MockServer::start().await;
        let issuer = server.uri();
        let discovery_doc = json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{issuer}/token"),
            "jwks_uri": format!("{issuer}/jwks"),
        });
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&discovery_doc))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        discover(&client, &issuer).await.unwrap();
        discover(&client, &issuer).await.unwrap();
        // wiremock's `expect(1)` is verified on drop; a second network
        // hit here would fail the test.
    }

    #[tokio::test]
    async fn rejects_a_token_from_an_unrelated_issuer() {
        let server = MockServer::start().await;
        let jwks_uri = format!("{}/jwks", server.uri());
        let jwks: serde_json::Value = serde_json::from_str(TEST_JWKS_JSON).unwrap();
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
            .mount(&server)
            .await;

        let token = sign_test_token("https://not-the-expected-issuer.example", "test-client");
        let client = reqwest::Client::new();
        let checks = cratebase_auth::IdTokenChecks {
            issuer: "https://issuer.example",
            audience: "test-client",
            nonce: None,
        };
        assert!(verify_id_token_cached(&client, &jwks_uri, &token, &checks)
            .await
            .is_err());
    }
}
