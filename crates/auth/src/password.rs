//! Password hashing with Argon2id. [`needs_rehash`] tells the login path
//! to regenerate any stored hash that is not Argon2id.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::{Algorithm, Argon2, Params, Version};

use crate::error::{AuthError, AuthResult};

/// Argon2id with the OWASP-recommended first configuration: 19 MiB of
/// memory, 2 iterations, 1 lane. Roughly 10-15ms per hash on a modern
/// core — deliberately expensive, which is why every server-side caller
/// goes through [`hash_password_async`] / [`verify_password_async`] so the
/// work runs on tokio's blocking pool instead of stalling an async worker
/// thread (a login burst would otherwise starve every other request).
fn argon2() -> Argon2<'static> {
    let params = Params::new(19_456, 2, 1, None).expect("valid argon2 params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Hash a plaintext password with Argon2id and a fresh random salt. The
/// returned string is self-describing (PHC format) so `verify_password`
/// needs no separately-stored salt or algorithm parameters.
///
/// Synchronous and CPU-bound; inside an async context use
/// [`hash_password_async`].
pub fn hash_password(password: &str) -> AuthResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    argon2()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AuthError::Hash(e.to_string()))
}

/// Verify a plaintext password against a stored `$argon2...` hash. Returns
/// `false` (not an error) for any mismatch, including a corrupt or unknown
/// hash, so callers can't accidentally treat a hashing bug as "password
/// accepted".
pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    argon2()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Whether a stored hash should be regenerated with [`hash_password`]
/// after a successful login: `true` for anything that isn't Argon2id.
pub fn needs_rehash(hash: &str) -> bool {
    !hash.starts_with("$argon2id$")
}

/// [`hash_password`] on tokio's blocking thread pool.
pub async fn hash_password_async(password: &str) -> AuthResult<String> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|e| AuthError::Hash(format!("hashing task failed: {e}")))?
}

/// [`verify_password`] on tokio's blocking thread pool.
pub async fn verify_password_async(password: &str, hash: &str) -> bool {
    let password = password.to_owned();
    let hash = hash.to_owned();
    tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_round_trip() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
        assert!(!needs_rehash(&hash));
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
        assert!(!verify_password("anything", "$2a$garbage"));
        assert!(needs_rehash("not-a-real-hash"));
    }

    #[test]
    fn only_argon2id_hashes_verify() {
        // A bcrypt-shaped hash is not a supported format and fails closed.
        let hash = "$2a$05$CCCCCCCCCCCCCCCCCCCCC.E5YPO9kmyuRGyh0XouQYb4YMJKvyOeW";
        assert!(!verify_password("U*U", hash));
        assert!(needs_rehash(hash));
    }

    #[tokio::test]
    async fn async_wrappers_round_trip() {
        let hash = hash_password_async("pw").await.unwrap();
        assert!(verify_password_async("pw", &hash).await);
        assert!(!verify_password_async("no", &hash).await);
    }
}
