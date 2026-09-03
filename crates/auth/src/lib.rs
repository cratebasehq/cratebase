//! Password hashing (Argon2id) and stateless JWT session tokens shared by
//! the admin panel and per-collection auth records.

mod error;
mod password;
mod token;

pub use error::{AuthError, AuthResult};
pub use password::{hash_password, verify_password};
pub use token::{issue_token, verify_token, TokenClaims, TokenKind};
