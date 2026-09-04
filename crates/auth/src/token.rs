//! PocketBase-compatible HS256 JWTs.
//!
//! A token's payload is exactly PocketBase's:
//!
//! ```json
//! {"collectionId":"pbc_3142635823","exp":1788526100,"id":"l5rhsibvzk6xatx","refreshable":true,"type":"auth"}
//! ```
//!
//! and its signing key is the concatenation `app_secret + record.tokenKey +
//! per-type secret suffix` (see [`signing_key`]), so rotating a record's
//! `tokenKey` invalidates every token that record has outstanding without
//! any server-side revocation list. Because the key depends on the record,
//! verification is a two-step dance: [`decode_unverified`] to learn the
//! `id`/`collectionId`, load that record, then [`verify`] with the derived
//! key.

use std::collections::BTreeMap;
use std::fmt;

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AuthError, AuthResult};

/// The `type` claim. String values match PocketBase's `core.Token*Type`
/// constants exactly, since they end up inside the signed payload.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TokenType {
    /// A session token (`"auth"`).
    Auth,
    /// A short-lived token granting access to protected files (`"file"`).
    File,
    /// Email verification (`"verification"`).
    Verification,
    /// Password reset (`"passwordReset"`).
    PasswordReset,
    /// Email change confirmation (`"emailChange"`).
    EmailChange,
    /// Any other value; kept so foreign or future token kinds round-trip
    /// through [`decode_unverified`] without being rejected up front.
    Custom(String),
}

impl TokenType {
    /// The exact string stored in the `type` claim.
    pub fn as_str(&self) -> &str {
        match self {
            TokenType::Auth => "auth",
            TokenType::File => "file",
            TokenType::Verification => "verification",
            TokenType::PasswordReset => "passwordReset",
            TokenType::EmailChange => "emailChange",
            TokenType::Custom(s) => s.as_str(),
        }
    }
}

impl From<&str> for TokenType {
    fn from(s: &str) -> Self {
        match s {
            "auth" => TokenType::Auth,
            "file" => TokenType::File,
            "verification" => TokenType::Verification,
            "passwordReset" => TokenType::PasswordReset,
            "emailChange" => TokenType::EmailChange,
            other => TokenType::Custom(other.to_string()),
        }
    }
}

impl fmt::Display for TokenType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for TokenType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TokenType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(TokenType::from(s.as_str()))
    }
}

/// The JWT payload. Field names serialize exactly as PocketBase writes
/// them (`id`, `type`, `collectionId`, `refreshable`, `exp`); anything
/// else (`email`, `newEmail`, ...) lives in [`Claims::extra`] and is
/// flattened into the same JSON object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claims {
    /// The auth record's id.
    pub id: String,
    /// See [`TokenType`].
    #[serde(rename = "type")]
    pub token_type: TokenType,
    /// The auth collection the record belongs to.
    #[serde(rename = "collectionId")]
    pub collection_id: String,
    /// Only present on `auth` tokens: whether `auth-refresh` may extend
    /// it. Impersonation tokens are minted with `Some(false)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshable: Option<bool>,
    /// Expiry as unix seconds.
    pub exp: i64,
    /// Additional claims; `email` on verification / passwordReset /
    /// emailChange tokens, plus `newEmail` on emailChange.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl Claims {
    /// A bare claim set with no extra claims and `refreshable` unset.
    pub fn new(
        id: impl Into<String>,
        token_type: TokenType,
        collection_id: impl Into<String>,
        duration_secs: i64,
    ) -> Self {
        Claims {
            id: id.into(),
            token_type,
            collection_id: collection_id.into(),
            refreshable: None,
            exp: unix_now() + duration_secs,
            extra: BTreeMap::new(),
        }
    }

    /// Adds (or overwrites) an extra claim.
    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }

    /// The string value of an extra claim, if present.
    pub fn extra_str(&self, key: &str) -> Option<&str> {
        self.extra.get(key).and_then(Value::as_str)
    }

    /// The `email` claim carried by verification / passwordReset /
    /// emailChange tokens.
    pub fn email(&self) -> Option<&str> {
        self.extra_str("email")
    }

    /// The `newEmail` claim carried by emailChange tokens.
    pub fn new_email(&self) -> Option<&str> {
        self.extra_str("newEmail")
    }

    /// `true` only for auth tokens explicitly marked refreshable.
    pub fn is_refreshable(&self) -> bool {
        self.token_type == TokenType::Auth && self.refreshable == Some(true)
    }

    /// Whether the token has already expired (no leeway).
    pub fn is_expired(&self) -> bool {
        self.exp <= unix_now()
    }
}

/// Session (`auth`) token claims.
pub fn new_auth_claims(
    record_id: &str,
    collection_id: &str,
    duration_secs: i64,
    refreshable: bool,
) -> Claims {
    let mut c = Claims::new(record_id, TokenType::Auth, collection_id, duration_secs);
    c.refreshable = Some(refreshable);
    c
}

/// Protected-file access (`file`) token claims.
pub fn new_file_claims(record_id: &str, collection_id: &str, duration_secs: i64) -> Claims {
    Claims::new(record_id, TokenType::File, collection_id, duration_secs)
}

/// Email verification token claims; `email` is the address being
/// verified, so a token minted before an email change can't verify the
/// new address.
pub fn new_verification_claims(
    record_id: &str,
    collection_id: &str,
    duration_secs: i64,
    email: &str,
) -> Claims {
    Claims::new(
        record_id,
        TokenType::Verification,
        collection_id,
        duration_secs,
    )
    .with_extra("email", email)
}

/// Password reset token claims, bound to the record's current `email`.
pub fn new_password_reset_claims(
    record_id: &str,
    collection_id: &str,
    duration_secs: i64,
    email: &str,
) -> Claims {
    Claims::new(
        record_id,
        TokenType::PasswordReset,
        collection_id,
        duration_secs,
    )
    .with_extra("email", email)
}

/// Email change token claims: `email` is the current address, `newEmail`
/// the one being confirmed. Carrying the target in the token means the
/// pending change needs no column of its own and can't outlive the token.
pub fn new_email_change_claims(
    record_id: &str,
    collection_id: &str,
    duration_secs: i64,
    email: &str,
    new_email: &str,
) -> Claims {
    Claims::new(
        record_id,
        TokenType::EmailChange,
        collection_id,
        duration_secs,
    )
    .with_extra("email", email)
    .with_extra("newEmail", new_email)
}

/// Derives the HS256 key for a record's token, PocketBase style: the
/// app-wide secret, the record's `tokenKey`, and the per-collection
/// per-type secret suffix (`TokenConfig.secret`, usually empty), as raw
/// bytes concatenated in that order.
pub fn signing_key(app_secret: &str, record_token_key: &str, type_secret: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(app_secret.len() + record_token_key.len() + type_secret.len());
    key.extend_from_slice(app_secret.as_bytes());
    key.extend_from_slice(record_token_key.as_bytes());
    key.extend_from_slice(type_secret.as_bytes());
    key
}

/// Signs `claims` with HS256 using `key` (see [`signing_key`]).
pub fn sign(claims: &Claims, key: &[u8]) -> AuthResult<String> {
    encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(key),
    )
    .map_err(|e| AuthError::TokenEncode(e.to_string()))
}

/// Decodes the payload **without** checking the signature or expiry.
/// Only ever use the result to look up which record's `tokenKey` to
/// derive the real key from, then call [`verify`].
pub fn decode_unverified(token: &str) -> AuthResult<Claims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.insecure_disable_signature_validation();
    validation.validate_exp = false;
    validation.validate_aud = false;
    decode::<Claims>(token, &DecodingKey::from_secret(&[]), &validation)
        .map(|data| data.claims)
        .map_err(|_| AuthError::InvalidToken)
}

/// Verifies the HS256 signature with `key` and that the token has not
/// expired (5 seconds of clock-skew leeway), returning the claims.
pub fn verify(token: &str, key: &[u8]) -> AuthResult<Claims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.validate_aud = false;
    validation.leeway = 5;
    decode::<Claims>(token, &DecodingKey::from_secret(key), &validation)
        .map(|data| data.claims)
        .map_err(|_| AuthError::InvalidToken)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_SECRET: &str = "app-secret-0123456789";
    const TOKEN_KEY: &str = "record-token-key-abcdef";

    fn key() -> Vec<u8> {
        signing_key(APP_SECRET, TOKEN_KEY, "")
    }

    /// The payload of a token captured from PocketBase 0.40.
    #[test]
    fn claims_serialize_with_pocketbase_key_names() {
        let claims = Claims {
            id: "l5rhsibvzk6xatx".into(),
            token_type: TokenType::Auth,
            collection_id: "pbc_3142635823".into(),
            refreshable: Some(true),
            exp: 1_788_526_100,
            extra: BTreeMap::new(),
        };
        let json = serde_json::to_value(&claims).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "collectionId": "pbc_3142635823",
                "exp": 1_788_526_100,
                "id": "l5rhsibvzk6xatx",
                "refreshable": true,
                "type": "auth"
            })
        );
        let back: Claims = serde_json::from_value(json).unwrap();
        assert_eq!(back, claims);
    }

    #[test]
    fn pocketbase_style_token_round_trips() {
        let claims = new_auth_claims("l5rhsibvzk6xatx", "pbc_3142635823", 3600, true);
        let token = sign(&claims, &key()).unwrap();
        assert_eq!(token.split('.').count(), 3);

        let unverified = decode_unverified(&token).unwrap();
        assert_eq!(unverified.id, "l5rhsibvzk6xatx");
        assert_eq!(unverified.collection_id, "pbc_3142635823");

        let verified = verify(&token, &key()).unwrap();
        assert_eq!(verified, claims);
        assert!(verified.is_refreshable());
        assert!(!verified.is_expired());
    }

    #[test]
    fn different_token_key_fails_verification() {
        let claims = new_auth_claims("rec", "pbc_1", 3600, true);
        let token = sign(&claims, &key()).unwrap();
        let rotated = signing_key(APP_SECRET, "some-other-token-key", "");
        assert!(matches!(
            verify(&token, &rotated),
            Err(AuthError::InvalidToken)
        ));
        // ...but the payload is still readable without the key.
        assert_eq!(decode_unverified(&token).unwrap().id, "rec");
    }

    #[test]
    fn type_secret_suffix_is_part_of_the_key() {
        let claims = new_file_claims("rec", "pbc_1", 180);
        let token = sign(&claims, &signing_key(APP_SECRET, TOKEN_KEY, "file-suffix")).unwrap();
        assert!(verify(&token, &key()).is_err());
        assert!(verify(&token, &signing_key(APP_SECRET, TOKEN_KEY, "file-suffix")).is_ok());
    }

    #[test]
    fn expired_token_is_rejected_but_still_decodable() {
        let claims = new_auth_claims("rec", "pbc_1", -120, true);
        let token = sign(&claims, &key()).unwrap();
        assert!(verify(&token, &key()).is_err());
        assert!(decode_unverified(&token).unwrap().is_expired());
    }

    #[test]
    fn non_auth_tokens_omit_refreshable() {
        let claims = new_verification_claims("rec", "pbc_1", 60, "a@example.com");
        let json = serde_json::to_value(&claims).unwrap();
        assert!(json.get("refreshable").is_none());
        assert_eq!(json["type"], "verification");
        assert_eq!(json["email"], "a@example.com");
        assert!(!claims.is_refreshable());
    }

    #[test]
    fn action_claims_carry_emails() {
        let reset = new_password_reset_claims("rec", "pbc_1", 60, "a@example.com");
        assert_eq!(reset.email(), Some("a@example.com"));
        assert_eq!(reset.token_type, TokenType::PasswordReset);

        let change = new_email_change_claims("rec", "pbc_1", 60, "a@example.com", "b@example.com");
        assert_eq!(change.email(), Some("a@example.com"));
        assert_eq!(change.new_email(), Some("b@example.com"));
        let token = sign(&change, &key()).unwrap();
        let back = verify(&token, &key()).unwrap();
        assert_eq!(back.new_email(), Some("b@example.com"));
        assert_eq!(back.token_type, TokenType::EmailChange);
    }

    #[test]
    fn token_type_string_values() {
        for (ty, s) in [
            (TokenType::Auth, "auth"),
            (TokenType::File, "file"),
            (TokenType::Verification, "verification"),
            (TokenType::PasswordReset, "passwordReset"),
            (TokenType::EmailChange, "emailChange"),
            (TokenType::Custom("mfa".into()), "mfa"),
        ] {
            assert_eq!(ty.as_str(), s);
            assert_eq!(TokenType::from(s), ty);
            assert_eq!(serde_json::to_value(&ty).unwrap(), s);
        }
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(decode_unverified("not.a.jwt").is_err());
        assert!(verify("", &key()).is_err());
    }
}
