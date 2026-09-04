//! Authentication primitives shared by the server: password hashing
//! (Argon2id, with bcrypt verification for hashes imported from
//! PocketBase), PocketBase-compatible HS256 record tokens, OTP codes,
//! PKCE helpers for OAuth2, and the `_authOrigins` fingerprint.
//!
//! Everything here is pure computation over strings and bytes; the only
//! async surface is the `*_async` password helpers, which run the
//! CPU-bound hashing on tokio's blocking pool.

mod error;
mod fingerprint;
mod otp;
mod password;
mod pkce;
mod token;

pub use error::{AuthError, AuthResult};
pub use fingerprint::auth_origin_fingerprint;
pub use otp::{generate_otp, hash_otp, DEFAULT_OTP_LENGTH};
pub use password::{
    hash_password, hash_password_async, needs_rehash, verify_password, verify_password_async,
};
pub use pkce::{
    code_challenge_s256, code_verifier, random_alphanumeric, random_state, CODE_VERIFIER_LENGTH,
    STATE_LENGTH,
};
pub use token::{
    decode_unverified, new_auth_claims, new_email_change_claims, new_file_claims,
    new_password_reset_claims, new_verification_claims, sign, signing_key, verify, Claims,
    TokenType,
};
