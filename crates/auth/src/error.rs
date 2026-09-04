/// Errors from password hashing and token signing/verification.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// Argon2 refused to hash (bad params, RNG failure) or the blocking
    /// task was cancelled.
    #[error("failed to hash password: {0}")]
    Hash(String),
    /// The token is malformed, has a bad signature, or has expired.
    #[error("invalid or expired token")]
    InvalidToken,
    /// The claims could not be serialized into a JWT.
    #[error("failed to encode token: {0}")]
    TokenEncode(String),
    /// An OAuth2 provider's token or userinfo response wasn't the JSON
    /// shape expected of it.
    #[error("invalid oauth2 provider response: {0}")]
    InvalidOAuth2Response(String),
}

pub type AuthResult<T> = Result<T, AuthError>;
