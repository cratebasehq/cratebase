#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("failed to hash password: {0}")]
    Hash(String),
    #[error("invalid or expired token")]
    InvalidToken,
}

pub type AuthResult<T> = Result<T, AuthError>;
