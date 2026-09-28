//! Provider-agnostic OpenID Connect id_token verification (RFC 7519 +
//! OpenID Connect Core §3.1.3.7): given an issuer's already-fetched JWKS
//! document and an id_token, checks the signature, `iss`, `aud`, `exp`
//! and (when supplied) `nonce`.
//!
//! Nothing here makes an HTTP request — discovering an issuer's
//! `/.well-known/openid-configuration` and fetching/caching its JWKS
//! (with rotation handling) is `cratebase_server::routes::oidc`'s job,
//! same division of labor as this crate's OAuth2 token exchange vs.
//! `cratebase_server::routes::auth`. This is also what Apple's
//! `parse_apple_id_token_claims` id_token is verified with, against
//! Apple's fixed `https://appleid.apple.com/auth/keys` JWKS — a generic
//! OIDC provider and Apple's Sign in with Apple share exactly this
//! verification step.

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde_json::Value;

use crate::error::{AuthError, AuthResult};

/// What the id_token is checked against. `nonce` is `None` for a flow
/// that never sent one (Apple's server-driven flow currently doesn't);
/// when `Some`, the token's `nonce` claim must match exactly.
pub struct IdTokenChecks<'a> {
    pub issuer: &'a str,
    pub audience: &'a str,
    pub nonce: Option<&'a str>,
}

/// Verifies `id_token`'s signature against `jwks` (a raw JWKS JSON
/// document, e.g. Apple's or a discovered OIDC provider's) and its
/// standard claims per `checks`. Returns the token's claims as a raw
/// JSON object on success — callers pull out whatever provider-specific
/// fields they need (`sub`/`email`/`email_verified`/... ) from that.
pub fn verify_id_token(id_token: &str, jwks: &[u8], checks: &IdTokenChecks) -> AuthResult<Value> {
    let jwk_set: JwkSet = serde_json::from_slice(jwks)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("jwks: {e}")))?;
    let header = decode_header(id_token)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("id_token header: {e}")))?;
    let kid = header
        .kid
        .ok_or_else(|| AuthError::InvalidOAuth2Response("id_token: missing kid".into()))?;
    let jwk = jwk_set.find(&kid).ok_or_else(|| {
        AuthError::InvalidOAuth2Response("id_token: unknown kid (rotated key?)".into())
    })?;
    let alg = jwk
        .common
        .key_algorithm
        .map(|a| {
            a.to_string()
                .parse::<Algorithm>()
                .map_err(|_| AuthError::InvalidOAuth2Response("id_token: unsupported alg".into()))
        })
        .unwrap_or(Ok(header.alg))?;
    let key = DecodingKey::from_jwk(jwk)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("id_token key: {e}")))?;

    let mut validation = Validation::new(alg);
    validation.set_issuer(&[checks.issuer]);
    validation.set_audience(&[checks.audience]);

    let data = decode::<Value>(id_token, &key, &validation)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("id_token: {e}")))?;

    if let Some(expected_nonce) = checks.nonce {
        let actual = data.claims.get("nonce").and_then(Value::as_str);
        if actual != Some(expected_nonce) {
            return Err(AuthError::InvalidOAuth2Response(
                "id_token: nonce mismatch".into(),
            ));
        }
    }

    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm as JwtAlgorithm, EncodingKey, Header};
    use serde_json::json;

    /// A fixed RSA-2048 test keypair (NOT used anywhere real), so the
    /// signature-verification path is exercised end to end without a
    /// network round trip.
    const TEST_PRIVATE_KEY_PEM: &str = include_str!("../testdata/oidc_test_rsa_private.pem");
    const TEST_KID: &str = "test-key-1";

    fn sign_test_token(claims: Value) -> String {
        let mut header = Header::new(JwtAlgorithm::RS256);
        header.kid = Some(TEST_KID.to_string());
        let key = EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).unwrap();
        jsonwebtoken::encode(&header, &claims, &key).unwrap()
    }

    fn test_jwks() -> Vec<u8> {
        std::fs::read("testdata/oidc_test_jwks.json").unwrap()
    }

    fn valid_claims() -> Value {
        json!({
            "iss": "https://issuer.example",
            "aud": "client-123",
            "sub": "user-1",
            "email": "jo@example.com",
            "email_verified": true,
            "exp": 9_999_999_999i64,
            "iat": 1_700_000_000,
            "nonce": "abc-nonce",
        })
    }

    fn checks() -> IdTokenChecks<'static> {
        IdTokenChecks {
            issuer: "https://issuer.example",
            audience: "client-123",
            nonce: Some("abc-nonce"),
        }
    }

    #[test]
    fn verifies_a_correctly_signed_token_with_matching_claims() {
        let token = sign_test_token(valid_claims());
        let claims = verify_id_token(&token, &test_jwks(), &checks()).unwrap();
        assert_eq!(claims["sub"], "user-1");
        assert_eq!(claims["email"], "jo@example.com");
    }

    #[test]
    fn rejects_a_wrong_issuer() {
        let mut c = valid_claims();
        c["iss"] = json!("https://evil.example");
        let token = sign_test_token(c);
        assert!(verify_id_token(&token, &test_jwks(), &checks()).is_err());
    }

    #[test]
    fn rejects_a_wrong_audience() {
        let mut c = valid_claims();
        c["aud"] = json!("someone-elses-client");
        let token = sign_test_token(c);
        assert!(verify_id_token(&token, &test_jwks(), &checks()).is_err());
    }

    #[test]
    fn rejects_an_expired_token() {
        let mut c = valid_claims();
        c["exp"] = json!(1);
        let token = sign_test_token(c);
        assert!(verify_id_token(&token, &test_jwks(), &checks()).is_err());
    }

    #[test]
    fn rejects_a_nonce_mismatch() {
        let mut c = valid_claims();
        c["nonce"] = json!("wrong-nonce");
        let token = sign_test_token(c);
        assert!(verify_id_token(&token, &test_jwks(), &checks()).is_err());
    }

    #[test]
    fn passes_when_no_nonce_was_expected() {
        let mut c = valid_claims();
        c.as_object_mut().unwrap().remove("nonce");
        let token = sign_test_token(c);
        let mut no_nonce_checks = checks();
        no_nonce_checks.nonce = None;
        assert!(verify_id_token(&token, &test_jwks(), &no_nonce_checks).is_ok());
    }

    #[test]
    fn rejects_an_unknown_kid_as_a_rotated_key() {
        let mut header = Header::new(JwtAlgorithm::RS256);
        header.kid = Some("some-other-kid".to_string());
        let key = EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).unwrap();
        let token = jsonwebtoken::encode(&header, &valid_claims(), &key).unwrap();
        assert!(verify_id_token(&token, &test_jwks(), &checks()).is_err());
    }

    #[test]
    fn rejects_a_token_signed_by_a_different_key_entirely() {
        // Same kid the JWKS advertises, but signed with an unrelated key
        // — this is exactly what a forged token looks like.
        let other_key_pem = include_str!("../testdata/oidc_other_rsa_private.pem");
        let mut header = Header::new(JwtAlgorithm::RS256);
        header.kid = Some(TEST_KID.to_string());
        let key = EncodingKey::from_rsa_pem(other_key_pem.as_bytes()).unwrap();
        let token = jsonwebtoken::encode(&header, &valid_claims(), &key).unwrap();
        assert!(verify_id_token(&token, &test_jwks(), &checks()).is_err());
    }
}
