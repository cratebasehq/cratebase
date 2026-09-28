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
mod oauth2;
mod oidc;
mod otp;
mod password;
mod pkce;
mod token;
mod totp;

pub use error::{AuthError, AuthResult};
pub use fingerprint::auth_origin_fingerprint;
pub use oauth2::{
    apple_client_secret, parse_apple_first_login_name, parse_apple_id_token_claims,
    parse_discord_userinfo, parse_facebook_userinfo, parse_generic_userinfo,
    parse_github_userinfo, parse_gitlab_userinfo, parse_google_userinfo, parse_linkedin_userinfo,
    parse_microsoft_userinfo, parse_slack_userinfo, parse_spotify_userinfo,
    parse_token_response, parse_twitch_userinfo, parse_twitter_userinfo, KnownProvider,
    OAuth2User, TokenExchange, TokenResponse,
};
pub use oidc::{verify_id_token, IdTokenChecks};
pub use otp::{generate_otp, hash_otp, DEFAULT_OTP_LENGTH};
pub use password::{
    hash_password, hash_password_async, needs_rehash, verify_password, verify_password_async,
};
pub use pkce::{
    code_challenge_s256, code_verifier, random_alphanumeric, random_state, CODE_VERIFIER_LENGTH,
    STATE_LENGTH,
};
pub use totp::{
    base32_decode, generate_backup_codes, generate_secret as generate_totp_secret,
    normalize_backup_code, otpauth_uri, totp_at, verify as verify_totp, DIGITS as TOTP_DIGITS,
    PERIOD_SECS as TOTP_PERIOD_SECS,
};
pub use token::{
    decode_unverified, new_auth_claims, new_email_change_claims, new_file_claims,
    new_password_reset_claims, new_verification_claims, sign, signing_key, verify, Claims,
    TokenType,
};
