use rand::Rng;
use sha2::{Digest, Sha256};

/// Generates a fresh 6-digit numeric one-time code (`"000000"`–`"999999"`,
/// zero-padded), for the OTP-login and MFA-second-factor flows. Six digits
/// matches every SMS/authenticator-app OTP a user has already been trained
/// on; entropy is deliberately not the whole story here — short TTL and
/// single-use consumption (`cratebase_db::otp::verify_and_consume`) are
/// what actually keep a 10^6-code-space code safe.
pub fn generate_otp() -> String {
    let code: u32 = rand::thread_rng().gen_range(0..1_000_000);
    format!("{code:06}")
}

/// Hashes an OTP code for storage. Deliberately a fast, unsalted SHA-256
/// rather than Argon2 (`hash_password`): a password hash has to resist
/// offline brute-forcing indefinitely, but an OTP code is discarded within
/// minutes and consumed on first use either way — an Argon2 round trip on
/// every login attempt would just be wasted latency for no real gain.
pub fn hash_otp(code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_code_is_six_digits() {
        for _ in 0..100 {
            let code = generate_otp();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn hash_is_deterministic_and_distinct() {
        assert_eq!(hash_otp("123456"), hash_otp("123456"));
        assert_ne!(hash_otp("123456"), hash_otp("654321"));
    }
}
