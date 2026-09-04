//! PKCE (RFC 7636) helpers for the OAuth2 flow, matching what PocketBase
//! puts in `authMethods.oauth2.providers[]` (`codeVerifier`,
//! `codeChallenge`, `codeChallengeMethod: "S256"`, `state`).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::Rng;
use sha2::{Digest, Sha256};

const ALPHANUMERIC: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Length of the verifier PocketBase generates (`security.RandomString(43)`).
pub const CODE_VERIFIER_LENGTH: usize = 43;
/// Length of the OAuth2 `state` we generate.
pub const STATE_LENGTH: usize = 32;

/// A random alphanumeric string of `len` characters, using the OS CSPRNG.
pub fn random_alphanumeric(len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| char::from(ALPHANUMERIC[rng.gen_range(0..ALPHANUMERIC.len())]))
        .collect()
}

/// A fresh PKCE `code_verifier`: 43 random alphanumeric characters, which
/// sits inside RFC 7636's 43..=128 unreserved-character range.
pub fn code_verifier() -> String {
    random_alphanumeric(CODE_VERIFIER_LENGTH)
}

/// The `S256` `code_challenge` for `verifier`: base64url (no padding) of
/// its SHA-256 digest.
pub fn code_challenge_s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// A random OAuth2 `state` parameter (32 alphanumeric characters).
pub fn random_state() -> String {
    random_alphanumeric(STATE_LENGTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_is_43_alphanumeric_chars() {
        let v = code_verifier();
        assert_eq!(v.len(), 43);
        assert!(v.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(v, code_verifier());
    }

    #[test]
    fn state_is_32_alphanumeric_chars() {
        let s = random_state();
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn s256_challenge_matches_rfc7636_appendix_b() {
        // RFC 7636 Appendix B test vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            code_challenge_s256(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn challenge_has_no_padding_and_is_url_safe() {
        let c = code_challenge_s256(&code_verifier());
        assert_eq!(c.len(), 43);
        assert!(!c.contains('=') && !c.contains('+') && !c.contains('/'));
    }
}
