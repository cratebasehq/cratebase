//! Password hashing (Argon2id) and stateless JWT session tokens shared by
//! the admin panel and per-collection auth records.

mod error;
mod otp;
mod password;
mod token;

pub use error::{AuthError, AuthResult};
pub use otp::{generate_otp, hash_otp};
pub use password::{hash_password, verify_password};
pub use token::{
    issue_action_token, issue_file_token, issue_token, verify_token, TokenClaims, TokenKind,
};
