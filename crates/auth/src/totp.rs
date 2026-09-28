//! TOTP 2FA (RFC 6238, built on RFC 4226's HOTP): secret generation, the
//! `otpauth://` provisioning URI an authenticator app scans as a QR
//! code, code verification with a ±1 step window, and backup codes.
//!
//! Everything here is pure computation — no clock is read except what a
//! caller passes in as `now`, so verification is deterministic and
//! trivially testable. Storage (the secret encrypted with `CB_ENCRYPTION`,
//! backup codes hashed, replay protection via a persisted last-used
//! step) is `cratebase_server::routes::totp`'s job, same division of
//! labor as every other crate in this module.

use hmac::{Hmac, Mac};
use rand::Rng;
use sha1::Sha1;

use crate::error::{AuthError, AuthResult};

/// RFC 6238's defaults, which this module hardcodes rather than
/// exposing as options — a consumer app has no reason to want SHA-256
/// TOTP or a 15-second step, and every authenticator app in the wild
/// assumes these anyway.
pub const DIGITS: u32 = 6;
pub const PERIOD_SECS: i64 = 30;
/// How many steps on either side of "now" a submitted code is still
/// accepted for — RFC 6238 §5.2's recommended single-step tolerance,
/// covering ordinary clock drift between the server and the phone.
pub const WINDOW_STEPS: i64 = 1;

const BASE32_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// A fresh random TOTP secret: 20 bytes (160 bits, matching RFC 4226's
/// own recommended HMAC-SHA1 key size) as unpadded base32 — the form
/// every authenticator app expects to scan or type in.
pub fn generate_secret() -> String {
    let mut bytes = [0u8; 20];
    rand::thread_rng().fill(&mut bytes);
    base32_encode(&bytes)
}

/// RFC 4648 §6 base32, no padding.
pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    for chunk in data.chunks(5) {
        let mut buf = [0u8; 5];
        buf[..chunk.len()].copy_from_slice(chunk);
        let bits = chunk.len() * 8;
        let n_chars = bits.div_ceil(5);
        let value = u64::from_be_bytes([0, 0, 0, buf[0], buf[1], buf[2], buf[3], buf[4]]);
        for i in 0..n_chars {
            let shift = 35 - i * 5;
            let idx = ((value >> shift) & 0x1f) as usize;
            out.push(BASE32_ALPHABET[idx] as char);
        }
    }
    out
}

/// Decodes base32 (with or without `=` padding, case-insensitive, and
/// tolerant of the spaces/dashes authenticator apps' own display format
/// sometimes adds when a user copies a secret by hand).
pub fn base32_decode(input: &str) -> AuthResult<Vec<u8>> {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '=')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let mut bits: u64 = 0;
    let mut n_bits: u32 = 0;
    let mut out = Vec::with_capacity(cleaned.len() * 5 / 8);
    for c in cleaned.chars() {
        let value = BASE32_ALPHABET
            .iter()
            .position(|&b| b as char == c)
            .ok_or_else(|| AuthError::InvalidOAuth2Response("invalid base32 secret".into()))?
            as u64;
        bits = (bits << 5) | value;
        n_bits += 5;
        if n_bits >= 8 {
            n_bits -= 8;
            out.push((bits >> n_bits) as u8);
        }
    }
    Ok(out)
}

/// RFC 4226's HOTP: HMAC-SHA1 the 8-byte big-endian `counter`, then the
/// dynamic-truncation step that turns the digest into a `digits`-long
/// decimal code.
fn hotp(key: &[u8], counter: u64, digits: u32) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let binary = ((u32::from(digest[offset]) & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    let code = binary % 10u32.pow(digits);
    format!("{code:0width$}", width = digits as usize)
}

/// The counter for RFC 6238 TOTP: elapsed [`PERIOD_SECS`]-second steps
/// since the Unix epoch (T0 = 0, matching every real-world
/// authenticator app).
fn step_for(unix_time: i64) -> u64 {
    (unix_time.max(0) / PERIOD_SECS) as u64
}

/// The 6-digit code for `secret_base32` at `unix_time`'s step — what an
/// authenticator app currently displays. Errors only on a malformed
/// secret (never a valid base32 string cratebase itself generated).
pub fn totp_at(secret_base32: &str, unix_time: i64) -> AuthResult<String> {
    let key = base32_decode(secret_base32)?;
    Ok(hotp(&key, step_for(unix_time), DIGITS))
}

/// Checks `code` against every step in `[now - WINDOW_STEPS, now +
/// WINDOW_STEPS]`, skipping any step at or before `last_used_step`
/// (replay protection: a code already accepted, in this or an earlier
/// window, is rejected even if it's still numerically valid). Returns
/// the step it matched — the caller persists this as the new
/// `last_used_step` so the same code can never be replayed.
pub fn verify(
    secret_base32: &str,
    code: &str,
    now: i64,
    last_used_step: i64,
) -> AuthResult<Option<i64>> {
    let key = base32_decode(secret_base32)?;
    let code = code.trim();
    if code.len() != DIGITS as usize || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(None);
    }
    let now_step = step_for(now) as i64;
    for delta in -WINDOW_STEPS..=WINDOW_STEPS {
        let step = now_step + delta;
        if step <= last_used_step || step < 0 {
            continue;
        }
        if hotp(&key, step as u64, DIGITS) == code {
            return Ok(Some(step));
        }
    }
    Ok(None)
}

/// The `otpauth://` provisioning URI an authenticator app's QR scanner
/// (or manual-entry screen) expects — Google Authenticator's
/// `KeyUriFormat`, which every other authenticator app also implements.
/// `account_name` and `issuer` are URL-encoded; a `:` in either would
/// otherwise be ambiguous with the `label`'s own `issuer:account`
/// separator.
pub fn otpauth_uri(secret_base32: &str, account_name: &str, issuer: &str) -> String {
    let encode = |s: &str| {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                    c.to_string()
                } else {
                    c.encode_utf8(&mut [0; 4])
                        .bytes()
                        .map(|b| format!("%{b:02X}"))
                        .collect()
                }
            })
            .collect::<String>()
    };
    let label = format!("{}:{}", encode(issuer), encode(account_name));
    format!(
        "otpauth://totp/{label}?secret={secret_base32}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={PERIOD_SECS}",
        encode(issuer)
    )
}

const BACKUP_CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

/// `count` backup codes, each `XXXXX-XXXXX` from a 32-character alphabet
/// that drops visually ambiguous characters (`0`/`O`, `1`/`I`/`L`) —
/// meant to be read off a screen and typed, not pasted.
pub fn generate_backup_codes(count: usize) -> Vec<String> {
    let mut rng = rand::thread_rng();
    (0..count)
        .map(|_| {
            let group = |rng: &mut rand::rngs::ThreadRng| -> String {
                (0..5)
                    .map(|_| BACKUP_CODE_ALPHABET[rng.gen_range(0..BACKUP_CODE_ALPHABET.len())] as char)
                    .collect()
            };
            format!("{}-{}", group(&mut rng), group(&mut rng))
        })
        .collect()
}

/// Normalizes a caller-submitted backup code before hashing/comparing:
/// uppercased, whitespace stripped — dashes are kept, since
/// [`generate_backup_codes`] always includes them and a hash comparison
/// needs an exact match either way.
pub fn normalize_backup_code(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_round_trips_arbitrary_bytes() {
        for len in [0, 1, 4, 5, 9, 20, 32] {
            let bytes: Vec<u8> = (0..len as u8).collect();
            let encoded = base32_encode(&bytes);
            assert!(encoded.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
            assert_eq!(base32_decode(&encoded).unwrap(), bytes, "len={len}");
        }
    }

    #[test]
    fn base32_matches_rfc4648_test_vectors() {
        // RFC 4648 §10, padding stripped (this module never pads).
        assert_eq!(base32_encode(b"f"), "MY");
        assert_eq!(base32_encode(b"fo"), "MZXQ");
        assert_eq!(base32_encode(b"foo"), "MZXW6");
        assert_eq!(base32_encode(b"foob"), "MZXW6YQ");
        assert_eq!(base32_encode(b"fooba"), "MZXW6YTB");
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI");
        for (plain, encoded) in [
            ("f", "MY"),
            ("fo", "MZXQ"),
            ("foo", "MZXW6"),
            ("foob", "MZXW6YQ"),
            ("fooba", "MZXW6YTB"),
            ("foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(base32_decode(encoded).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn base32_decode_is_case_and_punctuation_tolerant() {
        let bytes = base32_decode("jbsw y3dp-ehpk3pxp").unwrap();
        let canonical = base32_decode("JBSWY3DPEHPK3PXP").unwrap();
        assert_eq!(bytes, canonical);
    }

    #[test]
    fn generated_secret_is_20_bytes_of_base32_and_never_constant() {
        let a = generate_secret();
        let b = generate_secret();
        assert_ne!(a, b);
        assert_eq!(base32_decode(&a).unwrap().len(), 20);
    }

    /// RFC 6238 Appendix B's own test vectors (SHA1, 8-digit truncation)
    /// against the fixed ASCII secret `"12345678901234567890"` —
    /// verifies this module's HOTP/step-counter math against the spec's
    /// published values, independent of the 6-digit width cratebase
    /// actually uses in production.
    #[test]
    fn hotp_matches_rfc6238_appendix_b_test_vectors_at_8_digits() {
        let key = b"12345678901234567890";
        let cases: [(i64, &str); 5] = [
            (59, "94287082"),
            (1_111_111_109, "07081804"),
            (1_111_111_111, "14050471"),
            (1_234_567_890, "89005924"),
            (2_000_000_000, "69279037"),
        ];
        for (time, expected) in cases {
            assert_eq!(hotp(key, step_for(time), 8), expected, "time={time}");
        }
    }

    #[test]
    fn verify_accepts_the_current_code_and_rejects_a_wrong_one() {
        let secret = generate_secret();
        let now = 1_700_000_000;
        let code = totp_at(&secret, now).unwrap();
        assert_eq!(verify(&secret, &code, now, 0).unwrap(), Some(step_for(now) as i64));
        assert_eq!(verify(&secret, "000000", now, 0).unwrap(), None);
    }

    #[test]
    fn verify_accepts_the_adjacent_step_within_the_window() {
        let secret = generate_secret();
        let now = 1_700_000_000;
        let earlier = totp_at(&secret, now - PERIOD_SECS).unwrap();
        let later = totp_at(&secret, now + PERIOD_SECS).unwrap();
        assert!(verify(&secret, &earlier, now, 0).unwrap().is_some());
        assert!(verify(&secret, &later, now, 0).unwrap().is_some());
    }

    #[test]
    fn verify_rejects_a_code_two_steps_away() {
        let secret = generate_secret();
        let now = 1_700_000_000;
        let too_old = totp_at(&secret, now - 2 * PERIOD_SECS).unwrap();
        assert_eq!(verify(&secret, &too_old, now, 0).unwrap(), None);
    }

    #[test]
    fn verify_rejects_replaying_an_already_used_step() {
        let secret = generate_secret();
        let now = 1_700_000_000;
        let code = totp_at(&secret, now).unwrap();
        let used_step = verify(&secret, &code, now, 0).unwrap().unwrap();
        // Same code, same instant, but the step is now the caller's
        // persisted `last_used_step` -- must not verify again.
        assert_eq!(verify(&secret, &code, now, used_step).unwrap(), None);
    }

    #[test]
    fn verify_rejects_malformed_codes_without_erroring() {
        let secret = generate_secret();
        assert_eq!(verify(&secret, "12345", 0, 0).unwrap(), None);
        assert_eq!(verify(&secret, "12a456", 0, 0).unwrap(), None);
        assert_eq!(verify(&secret, "", 0, 0).unwrap(), None);
    }

    #[test]
    fn otpauth_uri_has_the_expected_shape() {
        let uri = otpauth_uri("JBSWY3DPEHPK3PXP", "jo@example.com", "Cratebase App");
        assert!(uri.starts_with("otpauth://totp/Cratebase%20App:jo%40example.com?"));
        assert!(uri.contains("secret=JBSWY3DPEHPK3PXP"));
        assert!(uri.contains("issuer=Cratebase%20App"));
        assert!(uri.contains("algorithm=SHA1"));
        assert!(uri.contains("digits=6"));
        assert!(uri.contains("period=30"));
    }

    #[test]
    fn generates_the_requested_number_of_distinct_backup_codes() {
        let codes = generate_backup_codes(10);
        assert_eq!(codes.len(), 10);
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), 10, "backup codes should not collide");
        for code in &codes {
            assert_eq!(code.len(), 11, "XXXXX-XXXXX");
            assert!(code.chars().all(|c| c == '-' || c.is_ascii_alphanumeric()));
        }
    }

    #[test]
    fn normalize_backup_code_ignores_case_and_whitespace() {
        assert_eq!(normalize_backup_code(" ab3de-fgh4j "), "AB3DE-FGH4J");
    }
}
