//! Fingerprinting of the client for `_authOrigins` ("login from a new
//! location" alerts).

use sha2::{Digest, Sha256};

/// Fingerprint of an authentication origin, stored in
/// `_authOrigins.fingerprint` and compared on every login to decide
/// whether to send the `authAlert` email.
///
/// Computed as the first 32 hex characters of
/// `SHA-256(user_agent + "\n" + client_ip)`. This only needs to be stable
/// within a Cratebase instance (it is never exchanged with PocketBase), so
/// the exact recipe is ours; the newline separator keeps `("ab", "c")`
/// and `("a", "bc")` distinct, and truncating to 32 hex chars keeps the
/// column PocketBase-sized.
pub fn auth_origin_fingerprint(user_agent: &str, client_ip: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(user_agent.as_bytes());
    hasher.update(b"\n");
    hasher.update(client_ip.as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    hex[..32].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_stable_and_32_hex_chars() {
        let a = auth_origin_fingerprint("Mozilla/5.0", "127.0.0.1");
        assert_eq!(a, auth_origin_fingerprint("Mozilla/5.0", "127.0.0.1"));
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn differs_by_agent_and_ip_and_boundary() {
        let base = auth_origin_fingerprint("Mozilla/5.0", "127.0.0.1");
        assert_ne!(base, auth_origin_fingerprint("curl/8", "127.0.0.1"));
        assert_ne!(base, auth_origin_fingerprint("Mozilla/5.0", "10.0.0.1"));
        assert_ne!(
            auth_origin_fingerprint("ab", "c"),
            auth_origin_fingerprint("a", "bc")
        );
    }
}
