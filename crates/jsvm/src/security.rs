//! Pure-Rust implementations behind the `$security` global.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use cratebase_core::AppError;
use hmac::{Hmac, Mac};
use rand::Rng;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256, Sha512};

pub const DEFAULT_ALPHABET: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

pub fn random_string(len: usize, alphabet: &str) -> String {
    let chars: Vec<char> = alphabet.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| chars[rng.gen_range(0..chars.len())])
        .collect()
}

pub fn sha256(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

pub fn sha512(input: &str) -> String {
    hex::encode(Sha512::digest(input.as_bytes()))
}

pub fn sha1(input: &str) -> String {
    hex::encode(sha1::Sha1::digest(input.as_bytes()))
}

pub fn md5(input: &str) -> String {
    hex::encode(md5::Md5::digest(input.as_bytes()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HmacAlg {
    Hs256,
    Hs512,
}

impl HmacAlg {
    fn name(self) -> &'static str {
        match self {
            HmacAlg::Hs256 => "HS256",
            HmacAlg::Hs512 => "HS512",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "HS256" => Some(HmacAlg::Hs256),
            "HS512" => Some(HmacAlg::Hs512),
            _ => None,
        }
    }
}

pub fn hmac_hex(alg: HmacAlg, data: &[u8], key: &[u8]) -> String {
    hex::encode(hmac_raw(alg, data, key))
}

fn hmac_raw(alg: HmacAlg, data: &[u8], key: &[u8]) -> Vec<u8> {
    match alg {
        HmacAlg::Hs256 => {
            let mut mac =
                <Hmac<Sha256> as Mac>::new_from_slice(key).expect("hmac accepts any key size");
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        }
        HmacAlg::Hs512 => {
            let mut mac =
                <Hmac<Sha512> as Mac>::new_from_slice(key).expect("hmac accepts any key size");
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        }
    }
}

/// Create an HMAC-signed JWT. `duration_secs > 0` adds an `exp` claim.
pub fn create_jwt(
    alg: HmacAlg,
    payload: &Map<String, Value>,
    key: &str,
    duration_secs: i64,
) -> Result<String, AppError> {
    let mut claims = payload.clone();
    if duration_secs > 0 {
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
            + duration_secs;
        claims.insert("exp".into(), Value::from(exp));
    }
    let header = serde_json::json!({ "alg": alg.name(), "typ": "JWT" });
    let header = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap_or_default());
    let body = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&Value::Object(claims))
            .map_err(|e| AppError::bad_request(format!("invalid JWT payload: {e}")))?,
    );
    let signing_input = format!("{header}.{body}");
    let sig = URL_SAFE_NO_PAD.encode(hmac_raw(alg, signing_input.as_bytes(), key.as_bytes()));
    Ok(format!("{signing_input}.{sig}"))
}

/// Decode the claims of a JWT without verifying the signature.
pub fn parse_unverified_jwt(token: &str) -> Result<Map<String, Value>, AppError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(AppError::bad_request("invalid JWT: expected 3 segments"));
    }
    let payload = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| AppError::bad_request("invalid JWT payload encoding"))?;
    let value: Value = serde_json::from_slice(&payload)
        .map_err(|_| AppError::bad_request("invalid JWT payload JSON"))?;
    match value {
        Value::Object(m) => Ok(m),
        _ => Err(AppError::bad_request("invalid JWT payload")),
    }
}

/// Verify an HMAC JWT's signature and expiration, returning its claims.
pub fn parse_jwt(token: &str, key: &str) -> Result<Map<String, Value>, AppError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(AppError::bad_request("invalid JWT: expected 3 segments"));
    }
    let header_bytes = URL_SAFE_NO_PAD
        .decode(parts[0])
        .map_err(|_| AppError::bad_request("invalid JWT header encoding"))?;
    let header: Value = serde_json::from_slice(&header_bytes)
        .map_err(|_| AppError::bad_request("invalid JWT header JSON"))?;
    let alg = header
        .get("alg")
        .and_then(Value::as_str)
        .and_then(HmacAlg::from_name)
        .ok_or_else(|| AppError::bad_request("unsupported JWT algorithm"))?;
    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let expected = hmac_raw(alg, signing_input.as_bytes(), key.as_bytes());
    let actual = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| AppError::bad_request("invalid JWT signature encoding"))?;
    if !constant_time_eq(&expected, &actual) {
        return Err(AppError::bad_request("invalid JWT signature"));
    }
    let claims = parse_unverified_jwt(token)?;
    if let Some(exp) = claims.get("exp").and_then(Value::as_i64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if exp < now {
            return Err(AppError::bad_request("JWT is expired"));
        }
    }
    Ok(claims)
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn aes_key(key: &str) -> Result<Key<Aes256Gcm>, AppError> {
    if key.len() != 32 {
        return Err(AppError::bad_request(
            "the encryption key must be exactly 32 characters",
        ));
    }
    Ok(*Key::<Aes256Gcm>::from_slice(key.as_bytes()))
}

/// AES-256-GCM encrypt `data` with a 32-character key; the result is
/// base64 of `nonce || ciphertext`, matching PocketBase's `security.Encrypt`.
pub fn encrypt(data: &str, key: &str) -> Result<String, AppError> {
    let cipher = Aes256Gcm::new(&aes_key(key)?);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, data.as_bytes())
        .map_err(|_| AppError::internal("encryption failed"))?;
    let mut out = nonce.to_vec();
    out.extend(ciphertext);
    Ok(STANDARD.encode(out))
}

pub fn decrypt(cipher_text: &str, key: &str) -> Result<String, AppError> {
    let cipher = Aes256Gcm::new(&aes_key(key)?);
    let raw = STANDARD
        .decode(cipher_text)
        .map_err(|_| AppError::bad_request("invalid cipher text encoding"))?;
    if raw.len() < 12 {
        return Err(AppError::bad_request("invalid cipher text"));
    }
    let (nonce, ciphertext) = raw.split_at(12);
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| AppError::bad_request("decryption failed"))?;
    String::from_utf8(plain).map_err(|_| AppError::bad_request("decrypted data is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes() {
        assert_eq!(
            sha256("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(md5("abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn jwt_round_trip() {
        let mut claims = Map::new();
        claims.insert("id".into(), Value::from("abc"));
        let token = create_jwt(HmacAlg::Hs256, &claims, "secret", 60).unwrap();
        let parsed = parse_jwt(&token, "secret").unwrap();
        assert_eq!(parsed["id"], "abc");
        assert!(parsed.get("exp").is_some());
        assert!(parse_jwt(&token, "wrong").is_err());
        assert_eq!(parse_unverified_jwt(&token).unwrap()["id"], "abc");
    }

    #[test]
    fn aes_round_trip() {
        let key = "0123456789abcdef0123456789abcdef";
        let enc = encrypt("hello", key).unwrap();
        assert_eq!(decrypt(&enc, key).unwrap(), "hello");
        assert!(encrypt("x", "short").is_err());
    }
}
