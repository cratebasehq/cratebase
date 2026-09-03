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
    VerifyEmail,
    ResetPassword,
    ChangeEmail,
    /// A pending second factor, minted by a successful password login on
    /// an `mfaRequired` collection in place of a real session token. Not
    /// a session kind: it only ever authorizes one thing, exchanging
    /// itself plus a matching OTP code for a real [`TokenKind::Auth`]
    /// token at `POST /collections/{c}/mfa/confirm`.
    Mfa,
    /// A short-lived, single-purpose token minted for the *current*
    /// caller by `POST /api/files/token`, used to authenticate a
    /// protected-file download (`?token=`) from a context that can't
    /// send an `Authorization` header — an `<img src>` tag, a shared
    /// link. Carries the same identity as the caller's session but with
    /// a much shorter TTL and no other authority.
    FileToken,
}

impl TokenKind {
    /// Whether this kind may authenticate an ordinary API request. Action
    /// tokens are single-purpose and only ever consumed by their matching
    /// `confirm-*` endpoint.
    pub fn is_session_kind(self) -> bool {
        matches!(self, TokenKind::Admin | TokenKind::Auth)
    }
}

/// The decoded, verified payload of a Cratebase token — a session token
/// (`Admin`/`Auth`) or a one-time action token (`VerifyEmail`/
/// `ResetPassword`/`ChangeEmail`). Signed with HS256 using the server's
/// configured secret; there is deliberately no server-side revocation list
/// (tradeoff for a stateless, dependency-free auth layer) — rotate
/// `AUTH_SECRET` to invalidate every outstanding token at once, or rely on
/// the (short, configurable) expiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenClaims {
    /// Admin id or auth-record id.
    pub sub: String,
    #[serde(rename = "type")]
    pub kind: TokenKind,
    /// The auth collection the record belongs to; empty for admin tokens.
    #[serde(default)]
    pub collection_id: String,
    /// `ChangeEmail` tokens only: the address the record is changing to.
    /// Carrying it in the token (rather than a DB column) means the
    /// pending change needs no schema of its own and can't outlive the
    /// token's own expiry.
    #[serde(default)]
    pub new_email: Option<String>,
    /// `FileToken` only: whether the caller was a superuser at mint time
    /// (an admin token has no `collection_id`, so this is how a resolved
    /// file token tells "admin" apart from "no matching auth record").
    /// Always `false` for every other kind.
    #[serde(default)]
    pub is_superuser: bool,
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
    issue_action_token(sub, kind, collection_id, None, secret, ttl_seconds)
}

/// Like [`issue_token`] but can carry a `new_email` (only meaningful for
/// [`TokenKind::ChangeEmail`] — every other kind passes `None`).
pub fn issue_action_token(
    sub: &str,
    kind: TokenKind,
    collection_id: &str,
    new_email: Option<&str>,
    secret: &str,
    ttl_seconds: i64,
) -> AuthResult<String> {
    let now = chrono_now();
    let claims = TokenClaims {
        sub: sub.to_string(),
        kind,
        collection_id: collection_id.to_string(),
        new_email: new_email.map(str::to_string),
        is_superuser: false,
        iat: now,
        exp: now + ttl_seconds,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|_| AuthError::InvalidToken)
}

/// Mints a [`TokenKind::FileToken`]. Unlike [`issue_token`]/
/// [`issue_action_token`] this carries `is_superuser` explicitly rather
/// than deriving it from `kind`, since a file token's identity is copied
/// from whatever session (admin or auth record) minted it.
pub fn issue_file_token(
    sub: &str,
    collection_id: &str,
    is_superuser: bool,
    secret: &str,
    ttl_seconds: i64,
) -> AuthResult<String> {
    let now = chrono_now();
    let claims = TokenClaims {
        sub: sub.to_string(),
        kind: TokenKind::FileToken,
        collection_id: collection_id.to_string(),
        new_email: None,
        is_superuser,
        iat: now,
        exp: now + ttl_seconds,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|_| AuthError::InvalidToken)
}

pub fn verify_token(token: &str, secret: &str) -> AuthResult<TokenClaims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.leeway = 5;
    decode::<TokenClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
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

    #[test]
    fn action_token_kinds_are_not_session_kinds() {
        assert!(TokenKind::Admin.is_session_kind());
        assert!(TokenKind::Auth.is_session_kind());
        assert!(!TokenKind::VerifyEmail.is_session_kind());
        assert!(!TokenKind::ResetPassword.is_session_kind());
        assert!(!TokenKind::ChangeEmail.is_session_kind());
    }

    #[test]
    fn change_email_token_carries_new_email() {
        let token = issue_action_token(
            "user-1",
            TokenKind::ChangeEmail,
            "users",
            Some("new@example.com"),
            "secret",
            3600,
        )
        .unwrap();
        let claims = verify_token(&token, "secret").unwrap();
        assert_eq!(claims.new_email.as_deref(), Some("new@example.com"));
    }
}
