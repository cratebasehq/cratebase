//! One-time passwords for the `request-otp` / `auth-with-otp` and MFA
//! flows. Codes are stored hashed in `_otps` and consumed on first use.

use rand::Rng;
use sha2::{Digest, Sha256};

/// PocketBase's default `otp.length`.
pub const DEFAULT_OTP_LENGTH: usize = 8;

/// Generates a fresh numeric one-time code of exactly `length` digits
/// (zero-padded, so leading zeros are possible). A `length` of `0` is
/// treated as [`DEFAULT_OTP_LENGTH`]. Entropy is deliberately not the
/// whole story here — the short TTL and single-use consumption are what
/// actually keep a 10^length code space safe.
pub fn generate_otp(length: usize) -> String {
    let length = if length == 0 {
        DEFAULT_OTP_LENGTH
    } else {
        length
    };
    let mut rng = rand::thread_rng();
    (0..length)
        .map(|_| char::from(b'0' + rng.gen_range(0..10u8)))
        .collect()
}

/// Hashes an OTP code for storage. Deliberately a fast, unsalted SHA-256
/// (hex) rather than Argon2 (`hash_password`): a password hash has to
/// resist offline brute-forcing indefinitely, but an OTP code is discarded
/// within minutes and consumed on first use either way — an Argon2 round
/// trip on every attempt would just be wasted latency for no real gain.
pub fn hash_otp(code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_code_has_requested_length_and_only_digits() {
        for len in [4usize, 6, 8, 12] {
            for _ in 0..50 {
                let code = generate_otp(len);
                assert_eq!(code.len(), len);
                assert!(code.chars().all(|c| c.is_ascii_digit()), "{code}");
            }
        }
    }

    #[test]
    fn zero_length_falls_back_to_default() {
        assert_eq!(generate_otp(0).len(), DEFAULT_OTP_LENGTH);
    }

    #[test]
    fn codes_are_not_constant() {
        let codes: std::collections::HashSet<String> = (0..20).map(|_| generate_otp(8)).collect();
        assert!(codes.len() > 1);
    }

    #[test]
    fn hash_is_deterministic_and_distinct() {
        assert_eq!(hash_otp("123456"), hash_otp("123456"));
        assert_ne!(hash_otp("123456"), hash_otp("654321"));
        assert_eq!(hash_otp("123456").len(), 64);
    }
}
