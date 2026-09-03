use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;

use crate::error::{AuthError, AuthResult};

/// Hash a plaintext password with Argon2id and a fresh random salt. The
/// returned string is self-describing (PHC format) so `verify_password`
/// needs no separately-stored salt or algorithm parameters.
pub fn hash_password(password: &str) -> AuthResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AuthError::Hash(e.to_string()))
}

/// Verify a plaintext password against a previously hashed value. Returns
/// `false` (not an error) for any mismatch, including a corrupt hash, so
/// callers can't accidentally treat a hashing bug as "password accepted".
pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_round_trip() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn same_password_hashes_differently() {
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();
        assert_ne!(a, b, "salts should differ between hashes");
    }

    #[test]
    fn corrupt_hash_fails_closed() {
        assert!(!verify_password("anything", "not-a-real-hash"));
    }
}
