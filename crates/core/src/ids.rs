//! Id generation matching PocketBase's conventions so ids produced by
//! either system are interchangeable in shape.

use rand::rngs::OsRng;
use rand::Rng;

const RECORD_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
pub const RECORD_ID_LENGTH: usize = 15;

/// A new record id: 15 lowercase alphanumeric characters from the OS
/// CSPRNG (PocketBase's `[a-z0-9]{15}` default `autogeneratePattern`).
pub fn record_id() -> String {
    random_string(RECORD_ID_LENGTH, RECORD_ALPHABET)
}

/// `len` characters drawn uniformly from `alphabet`.
pub fn random_string(len: usize, alphabet: &[u8]) -> String {
    let mut rng = OsRng;
    (0..len)
        .map(|_| alphabet[rng.gen_range(0..alphabet.len())] as char)
        .collect()
}

/// PocketBase derives a collection id from its type and name:
/// `pbc_<crc32(type + name)>`, e.g. `pbc_2279338944` for base `_mfas`.
pub fn collection_id(collection_type: &str, name: &str) -> String {
    let mut h = crc32fast::Hasher::new();
    h.update(collection_type.as_bytes());
    h.update(name.as_bytes());
    format!("pbc_{}", h.finalize())
}

/// PocketBase derives a field id from its type and name:
/// `<type><crc32(name)>`, e.g. `text3208210256` for `id`.
pub fn field_id(field_type: &str, name: &str) -> String {
    format!("{field_type}{}", crc32fast::hash(name.as_bytes()))
}

/// Generate a string from a restricted regex-like pattern as PocketBase's
/// `autogeneratePattern` does. Supported: literal characters, character
/// classes (`[a-z0-9]`, `[A-Za-z]`, `[^...]` is not supported), escaped
/// characters (`\d`, `\w`), and quantifiers `{n}` / `{n,m}` / `+` / `*`
/// applied to the preceding atom. Anything else is emitted literally.
pub fn autogenerate(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut rng = OsRng;

    while i < chars.len() {
        // Parse one atom into a candidate alphabet.
        let alphabet: Vec<char> = match chars[i] {
            '[' => {
                let mut j = i + 1;
                let mut set = Vec::new();
                while j < chars.len() && chars[j] != ']' {
                    if j + 2 < chars.len() && chars[j + 1] == '-' && chars[j + 2] != ']' {
                        let (a, b) = (chars[j] as u32, chars[j + 2] as u32);
                        if a <= b {
                            set.extend((a..=b).filter_map(char::from_u32));
                        }
                        j += 3;
                    } else if chars[j] == '\\' && j + 1 < chars.len() {
                        set.extend(escape_class(chars[j + 1]));
                        j += 2;
                    } else {
                        set.push(chars[j]);
                        j += 1;
                    }
                }
                i = j + 1;
                set
            }
            '\\' if i + 1 < chars.len() => {
                let set = escape_class(chars[i + 1]);
                i += 2;
                set
            }
            '.' => {
                i += 1;
                (b'a'..=b'z')
                    .chain(b'A'..=b'Z')
                    .chain(b'0'..=b'9')
                    .map(|b| b as char)
                    .collect()
            }
            c => {
                i += 1;
                vec![c]
            }
        };

        // Quantifier.
        let (min, max) = if i < chars.len() && chars[i] == '{' {
            let close = chars[i..].iter().position(|c| *c == '}').map(|p| p + i);
            match close {
                Some(close) => {
                    let body: String = chars[i + 1..close].iter().collect();
                    i = close + 1;
                    let mut parts = body.split(',');
                    let min = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(1);
                    let max = parts
                        .next()
                        .and_then(|p| p.trim().parse().ok())
                        .unwrap_or(min);
                    (min, max.max(min))
                }
                None => (1, 1),
            }
        } else if i < chars.len() && chars[i] == '+' {
            i += 1;
            (1, 8)
        } else if i < chars.len() && chars[i] == '*' {
            i += 1;
            (0, 8)
        } else {
            (1, 1)
        };

        if alphabet.is_empty() {
            continue;
        }
        let count = if max > min {
            rng.gen_range(min..=max)
        } else {
            min
        };
        for _ in 0..count {
            out.push(alphabet[rng.gen_range(0..alphabet.len())]);
        }
    }
    out
}

fn escape_class(c: char) -> Vec<char> {
    match c {
        'd' => ('0'..='9').collect(),
        'w' => ('a'..='z')
            .chain('A'..='Z')
            .chain('0'..='9')
            .chain(std::iter::once('_'))
            .collect(),
        's' => vec![' '],
        other => vec![other],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_ids_are_15_lowercase_alphanumerics() {
        let id = record_id();
        assert_eq!(id.len(), 15);
        assert!(id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
        assert_ne!(record_id(), record_id());
    }

    #[test]
    fn collection_and_field_ids_match_pocketbase() {
        // Values observed from PocketBase v0.40.2 (see pb-fixtures).
        assert_eq!(collection_id("base", "_mfas"), "pbc_2279338944");
        assert_eq!(collection_id("base", "_otps"), "pbc_1638494021");
        assert_eq!(collection_id("auth", "_superusers"), "pbc_3142635823");
        assert_eq!(collection_id("base", "posts"), "pbc_1125843985");
        assert_eq!(field_id("text", "id"), "text3208210256");
        assert_eq!(field_id("autodate", "created"), "autodate2990389176");
        assert_eq!(field_id("password", "password"), "password901924565");
    }

    #[test]
    fn autogenerate_honours_class_and_quantifier() {
        let s = autogenerate("[a-z0-9]{15}");
        assert_eq!(s.len(), 15);
        assert!(s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));

        let s = autogenerate("[A-Z]{2}-\\d{3}");
        assert_eq!(s.len(), 6);
        assert!(s.as_bytes()[2] == b'-');
        assert!(s[3..].bytes().all(|b| b.is_ascii_digit()));

        let s = autogenerate("[a-zA-Z0-9]{50}");
        assert_eq!(s.len(), 50);
    }
}
