use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::error::{AuthError, AuthResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    /// A superuser (admin panel) session.
    Admin,
    /// A record from an `Auth`-typed collection.
    Auth,
}

/// The decoded, verified payload of a Cratebase access token. Signed with
/// HS256 using the server's configured secret; there is deliberately no
/// server-side revocation list (tradeoff for a stateless, dependency-free
/// auth layer) — rotate `AUTH_SECRET` to invalidate every outstanding token
/// at once, or rely on the (short, configurable) expiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenClaims {
    /// Admin id or auth-record id.
    pub sub: String,
    #[serde(rename = "type")]
    pub kind: TokenKind,
    /// The auth collection the record belongs to; empty for admin tokens.
    #[serde(default)]
    pub collection_id: String,
    pub iat: i64,
    pub exp: i64,
}

pub fn issue_token(
    sub: &str,
    kind: TokenKind,
    collection_id: &str,
    secret: &str,
    ttl_seconds: i64,
) -> AuthResult<String> {
    let now = chrono_now();
    let claims = TokenClaims {
        sub: sub.to_string(),
        kind,
        collection_id: collection_id.to_string(),
        iat: now,
        exp: now + ttl_seconds,
    };
    encode(&Header::new(Algorithm::HS256), &claims, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|_| AuthError::InvalidToken)
}

pub fn verify_token(token: &str, secret: &str) -> AuthResult<TokenClaims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.leeway = 5;
    decode::<TokenClaims>(token, &DecodingKey::from_secret(secret.as_bytes()), &validation)
        .map(|data| data.claims)
        .map_err(|_| AuthError::InvalidToken)
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_and_verify_round_trip() {
        let token = issue_token("user-1", TokenKind::Auth, "users", "secret", 3600).unwrap();
        let claims = verify_token(&token, "secret").unwrap();
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.kind, TokenKind::Auth);
        assert_eq!(claims.collection_id, "users");
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let token = issue_token("user-1", TokenKind::Auth, "users", "secret", 3600).unwrap();
        assert!(verify_token(&token, "other-secret").is_err());
    }

    #[test]
    fn expired_token_is_rejected() {
        let token = issue_token("user-1", TokenKind::Admin, "", "secret", -120).unwrap();
        assert!(verify_token(&token, "secret").is_err());
    }
}
