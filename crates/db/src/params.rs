//! `_params`: a key/value table for app settings (JSON under
//! [`SETTINGS_KEY`]) and other small pieces of state.
//!
//! When `CB_ENCRYPTION` (exactly 32 characters) is set, values are
//! stored AES-256-GCM encrypted as `base64(nonce || ciphertext)`. Reads
//! are lenient: a value that does not decrypt is returned as-is, so a
//! database created without the key keeps working after the key is
//! added (and values are re-encrypted on their next save).

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use cratebase_core::{record_id, DateTime, Settings};
use rand::RngCore;

use crate::engine::{Executor, Sql};
use crate::error::{DbError, DbResult};

pub const SETTINGS_KEY: &str = "settings";
pub const ENCRYPTION_ENV: &str = "CB_ENCRYPTION";
const NONCE_LEN: usize = 12;

/// Optional at-rest encryption for `_params` values.
#[derive(Clone, Default)]
pub struct ParamCipher {
    key: Option<[u8; 32]>,
}

impl ParamCipher {
    /// No encryption.
    pub fn none() -> Self {
        Self::default()
    }

    /// From a 32-byte key.
    pub fn new(key: &[u8]) -> DbResult<Self> {
        let key: [u8; 32] = key.try_into().map_err(|_| {
            DbError::Other(format!("{ENCRYPTION_ENV} must be exactly 32 characters"))
        })?;
        Ok(ParamCipher { key: Some(key) })
    }

    /// From `CB_ENCRYPTION`; unset means no encryption, a malformed value
    /// is logged and ignored rather than locking the operator out.
    pub fn from_env() -> Self {
        match std::env::var(ENCRYPTION_ENV) {
            Ok(v) if !v.is_empty() => match Self::new(v.as_bytes()) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(error = %e, "ignoring {ENCRYPTION_ENV}");
                    Self::none()
                }
            },
            _ => Self::none(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.key.is_some()
    }

    /// Plaintext → stored form.
    pub fn encrypt(&self, plain: &str) -> DbResult<String> {
        let Some(key) = &self.key else {
            return Ok(plain.to_string());
        };
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|e| DbError::Other(format!("encryption key: {e}")))?;
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
            .map_err(|_| DbError::Other("encryption failed".into()))?;
        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(BASE64.encode(out))
    }

    /// Stored form → plaintext. Falls back to the input when it is not
    /// a ciphertext produced with this key.
    pub fn decrypt(&self, stored: &str) -> String {
        let Some(key) = &self.key else {
            return stored.to_string();
        };
        let Ok(bytes) = BASE64.decode(stored.trim()) else {
            return stored.to_string();
        };
        if bytes.len() <= NONCE_LEN {
            return stored.to_string();
        }
        let Ok(cipher) = Aes256Gcm::new_from_slice(key) else {
            return stored.to_string();
        };
        let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
        match cipher.decrypt(Nonce::from_slice(nonce), ciphertext) {
            Ok(plain) => String::from_utf8(plain).unwrap_or_else(|_| stored.to_string()),
            Err(_) => stored.to_string(),
        }
    }
}

/// Read a value (decrypted with the environment's key).
pub async fn get(ex: &dyn Executor, key: &str) -> DbResult<Option<String>> {
    get_with(ex, key, &ParamCipher::from_env()).await
}

/// Upsert a value (encrypted with the environment's key).
pub async fn set(ex: &dyn Executor, key: &str, value: &str) -> DbResult<()> {
    set_with(ex, key, value, &ParamCipher::from_env()).await
}

pub async fn get_with(
    ex: &dyn Executor,
    key: &str,
    cipher: &ParamCipher,
) -> DbResult<Option<String>> {
    let row = ex
        .query_one(
            r#"SELECT "value" FROM "_params" WHERE "key" = $1"#,
            &[Sql::from(key)],
        )
        .await?;
    Ok(row.and_then(|r| match r.values.into_iter().next() {
        Some(Sql::Null) | None => None,
        Some(v) => Some(cipher.decrypt(&v.into_string())),
    }))
}

pub async fn set_with(
    ex: &dyn Executor,
    key: &str,
    value: &str,
    cipher: &ParamCipher,
) -> DbResult<()> {
    let now = DateTime::now().to_pb_string();
    ex.execute(
        r#"INSERT INTO "_params" ("id", "key", "value", "created", "updated")
           VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT ("key") DO UPDATE SET "value" = excluded."value", "updated" = excluded."updated""#,
        &[
            Sql::Text(record_id()),
            Sql::from(key),
            Sql::Text(cipher.encrypt(value)?),
            Sql::Text(now.clone()),
            Sql::Text(now),
        ],
    )
    .await?;
    Ok(())
}

pub async fn remove(ex: &dyn Executor, key: &str) -> DbResult<bool> {
    Ok(ex
        .execute(
            r#"DELETE FROM "_params" WHERE "key" = $1"#,
            &[Sql::from(key)],
        )
        .await?
        > 0)
}

/// The stored [`Settings`], or the defaults when none were saved yet.
/// A corrupt JSON blob is reported rather than silently replaced.
pub async fn load_settings(ex: &dyn Executor) -> DbResult<Settings> {
    load_settings_with(ex, &ParamCipher::from_env()).await
}

pub async fn load_settings_with(ex: &dyn Executor, cipher: &ParamCipher) -> DbResult<Settings> {
    match get_with(ex, SETTINGS_KEY, cipher).await? {
        Some(raw) if !raw.trim().is_empty() => Ok(serde_json::from_str(&raw)?),
        _ => Ok(Settings::default()),
    }
}

pub async fn save_settings(ex: &dyn Executor, settings: &Settings) -> DbResult<()> {
    save_settings_with(ex, settings, &ParamCipher::from_env()).await
}

pub async fn save_settings_with(
    ex: &dyn Executor,
    settings: &Settings,
    cipher: &ParamCipher,
) -> DbResult<()> {
    let raw = serde_json::to_string(settings)?;
    set_with(ex, SETTINGS_KEY, &raw, cipher).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::SqliteEngine;
    use crate::system::ensure_system_tables;

    async fn engine() -> SqliteEngine {
        let e = SqliteEngine::open_memory().unwrap();
        ensure_system_tables(&e).await.unwrap();
        e
    }

    #[tokio::test]
    async fn set_get_upsert_and_remove() {
        let e = engine().await;
        let plain = ParamCipher::none();
        assert_eq!(get_with(&e, "k", &plain).await.unwrap(), None);
        set_with(&e, "k", "v1", &plain).await.unwrap();
        set_with(&e, "k", "v2", &plain).await.unwrap();
        assert_eq!(
            get_with(&e, "k", &plain).await.unwrap().as_deref(),
            Some("v2")
        );
        let rows = e.query("SELECT COUNT(*) FROM _params", &[]).await.unwrap();
        assert_eq!(rows[0].values[0], Sql::Int(1));
        assert!(remove(&e, "k").await.unwrap());
        assert!(!remove(&e, "k").await.unwrap());
    }

    #[tokio::test]
    async fn settings_round_trip() {
        let e = engine().await;
        let plain = ParamCipher::none();
        let loaded = load_settings_with(&e, &plain).await.unwrap();
        assert_eq!(loaded, Settings::default());
        let mut s = Settings::default();
        s.meta.app_name = "Cratebase".into();
        s.smtp.password = "hunter2".into();
        save_settings_with(&e, &s, &plain).await.unwrap();
        let back = load_settings_with(&e, &plain).await.unwrap();
        assert_eq!(back, s);
        assert_eq!(back.smtp.password, "hunter2");
    }

    #[tokio::test]
    async fn encryption_round_trip_and_plaintext_fallback() {
        let e = engine().await;
        let cipher = ParamCipher::new(b"0123456789abcdef0123456789abcdef").unwrap();
        assert!(ParamCipher::new(b"short").is_err());
        assert!(cipher.is_enabled());

        let stored = cipher.encrypt("secret").unwrap();
        assert_ne!(stored, "secret");
        assert_ne!(cipher.encrypt("secret").unwrap(), stored, "nonce is random");
        assert_eq!(cipher.decrypt(&stored), "secret");
        // Wrong key falls back to the raw value.
        let other = ParamCipher::new(b"fedcba9876543210fedcba9876543210").unwrap();
        assert_eq!(other.decrypt(&stored), stored);

        set_with(&e, "k", "hello", &cipher).await.unwrap();
        let raw = e
            .query("SELECT value FROM _params WHERE key = 'k'", &[])
            .await
            .unwrap();
        assert_ne!(raw[0].get_str("value"), Some("hello"));
        assert_eq!(
            get_with(&e, "k", &cipher).await.unwrap().as_deref(),
            Some("hello")
        );

        // A plaintext value written before the key existed still reads.
        set_with(&e, "legacy", "plain", &ParamCipher::none())
            .await
            .unwrap();
        assert_eq!(
            get_with(&e, "legacy", &cipher).await.unwrap().as_deref(),
            Some("plain")
        );

        let mut s = Settings::default();
        s.s3.secret = "s3cr3t".into();
        save_settings_with(&e, &s, &cipher).await.unwrap();
        assert_eq!(load_settings_with(&e, &cipher).await.unwrap(), s);
    }
}
