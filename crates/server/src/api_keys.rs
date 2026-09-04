//! Incoming API-key authentication (`_api_keys`): a superuser mints a
//! `cb_<random>` token to hand to a script, CI job, or MCP client, and any
//! request presenting it as a Bearer token resolves to a superuser
//! identity — see [`crate::extract::resolve_and_cache`]'s private
//! `resolve` for where this plugs into the ordinary session-JWT
//! resolution path.
//!
//! # Why this can't be a session JWT
//!
//! A session token is signed with a *record's own* `tokenKey`
//! (`crate::extract`'s module doc explains why), which only exists for
//! auth-collection records. An API key has no backing user — it's the
//! identity — so it is a bearer secret checked directly against a stored
//! hash, the same shape as a password, not a signed claim.
//!
//! # Hashing: no second scheme
//!
//! `_api_keys.key` is hashed with [`cratebase_auth::hash_password`]/
//! [`cratebase_auth::verify_password`] — the exact Argon2id (with
//! bcrypt-import compatibility) scheme every password in this codebase
//! already goes through. There is nothing bespoke here to audit; the raw
//! key is only ever visible once, in the response [`crate::routes::api_keys`]'s
//! `POST /api/api-keys` returns.
//!
//! # Lookup: why `prefix` exists
//!
//! A hash can't be looked up by an index — verifying means trying
//! candidates against the plaintext. `_api_keys.prefix` (the first 8
//! characters of the random suffix, stored in the clear) narrows a
//! lookup from "every key" to "keys sharing this prefix", which with 8
//! alphanumeric characters of entropy is in practice exactly one row.
//! [`resolve`] still verifies the full hash rather than trusting the
//! prefix match alone, and a shared prefix is handled safely — the wrong
//! candidate simply fails verification, same as any other guess.

use serde_json::{Map, Value};

use cratebase_core::Collection;
use cratebase_db::engine::{Executor, Sql};

use crate::app::App;
use crate::extract::Auth;

pub const COLLECTION: &str = "_api_keys";

/// Every minted key starts with this, so [`looks_like_api_key`] can tell
/// an API key from a session JWT before either is parsed.
pub const KEY_PREFIX: &str = "cb_";

/// Random characters after [`KEY_PREFIX`]. 32 alphanumeric characters
/// from a 62-character alphabet is close to 190 bits of entropy —
/// comfortably beyond brute force, matching the order of magnitude
/// `cratebase_auth::pkce`'s code verifiers use elsewhere in this
/// codebase.
const RANDOM_LEN: usize = 32;

/// How much of the random suffix is kept in the clear as `prefix`, for
/// dashboard display (`cb_a1b2c3d4…`) and to narrow [`resolve`]'s
/// lookup.
const PREFIX_LEN: usize = 8;

/// A freshly minted key. [`GeneratedKey::raw`] is the only copy that
/// will ever exist outside this process's memory and the caller's
/// clipboard — nothing stores it; the caller hashes it before writing
/// the row.
pub struct GeneratedKey {
    pub raw: String,
    pub prefix: String,
}

/// Generate a new random key. Pure computation — the caller hashes
/// [`GeneratedKey::raw`] and inserts the `_api_keys` row.
pub fn generate() -> GeneratedKey {
    let random = cratebase_auth::random_alphanumeric(RANDOM_LEN);
    let raw = format!("{KEY_PREFIX}{random}");
    let prefix: String = random.chars().take(PREFIX_LEN).collect();
    GeneratedKey { raw, prefix }
}

/// Whether `token` is shaped like an API key rather than a session JWT,
/// so the resolver can go straight to [`resolve`] instead of first
/// trying (and failing) to decode it as one.
pub fn looks_like_api_key(token: &str) -> bool {
    token.starts_with(KEY_PREFIX)
}

/// Resolve a `cb_...` bearer token to a superuser [`Auth`], or `None`
/// for anything that doesn't check out. An unknown prefix, a hash
/// mismatch, and a disabled key all return the same `None` here — the
/// caller (`crate::extract`) turns that into the same 401 a missing or
/// garbage session token gets, so none of the three is distinguishable
/// from the outside.
pub async fn resolve(app: &App, token: &str) -> Option<Auth> {
    let random = token.strip_prefix(KEY_PREFIX)?;
    if random.len() < PREFIX_LEN {
        return None;
    }
    let prefix: String = random.chars().take(PREFIX_LEN).collect();

    let collection = app.db().collections.get_by_name(COLLECTION)?;
    let mut params = Map::new();
    params.insert("prefix".into(), Value::String(prefix));
    let candidate = match cratebase_db::records::find_first_by_filter(
        app.db(),
        &app.db().collections,
        &collection,
        "prefix = {:prefix}",
        &params,
    )
    .await
    {
        Ok(Some(record)) => record,
        _ => return None,
    };

    let stored_hash = candidate.get("key").and_then(Value::as_str)?.to_string();
    if !cratebase_auth::verify_password_async(token, &stored_hash).await {
        return None;
    }
    if !matches!(candidate.get("enabled"), Some(Value::Bool(true))) {
        return None;
    }

    touch_last_used(app, &collection, candidate.id()).await;

    let id = candidate.id().to_string();
    Some(Auth {
        id,
        collection_id: collection.id.clone(),
        collection_name: collection.name.clone(),
        is_superuser: true,
        collection,
        record: candidate,
    })
}

/// Best-effort `lastUsedAt` stamp. A raw `UPDATE`, not `records::update`
/// — same reasoning as `_webhooks`'/`_cron_jobs`' own status write-backs
/// (see `crate::webhooks`'s module doc's "Why the status write-back
/// bypasses the record API" section): this runs on every authenticated
/// request the key makes, so it must not pay for full record validation
/// or ever fail the request it's piggybacking on.
async fn touch_last_used(app: &App, collection: &Collection, id: &str) {
    let now = cratebase_core::DateTime::now().to_pb_string();
    let sql = format!(
        "UPDATE {} SET \"lastUsedAt\" = $1 WHERE \"id\" = $2",
        cratebase_db::quote_ident(collection.table_name())
    );
    if let Err(e) = app
        .db()
        .execute(&sql, &[Sql::Text(now), Sql::Text(id.to_string())])
        .await
    {
        tracing::warn!(error = %e, "failed to update _api_keys.lastUsedAt");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_shape() {
        let key = generate();
        assert!(key.raw.starts_with(KEY_PREFIX));
        assert_eq!(key.raw.len(), KEY_PREFIX.len() + RANDOM_LEN);
        assert_eq!(key.prefix.len(), PREFIX_LEN);
        assert!(key.raw[KEY_PREFIX.len()..].starts_with(&key.prefix));
    }

    #[test]
    fn generated_keys_are_unique() {
        let a = generate();
        let b = generate();
        assert_ne!(a.raw, b.raw);
    }

    #[test]
    fn looks_like_api_key_matches_prefix_only() {
        assert!(looks_like_api_key("cb_abc123"));
        assert!(!looks_like_api_key("eyJhbGciOiJIUzI1NiJ9.x.y"));
        assert!(!looks_like_api_key(""));
    }

    /// A bootstrapped app over an in-memory database, mirroring
    /// `routes::schema`'s test harness.
    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(crate::config::Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    /// Insert an enabled `_api_keys` row for `key`, exactly the way
    /// `routes::api_keys::create` does (generate, hash, store), and hand
    /// back the raw key so a test can present it as a bearer token.
    async fn insert_key(app: &App, name: &str, enabled: bool) -> String {
        let collection = app
            .db()
            .collections
            .get_by_name(COLLECTION)
            .expect("_api_keys");
        let generated = generate();
        let hash = cratebase_auth::hash_password_async(&generated.raw)
            .await
            .expect("hash");
        let mut record = cratebase_core::Record::new(collection);
        record.set("name", Value::String(name.to_string()));
        record.set("key", Value::String(hash));
        record.set("prefix", Value::String(generated.prefix.clone()));
        record.set("enabled", Value::Bool(enabled));
        cratebase_db::records::create(app.db(), &app.db().collections, &mut record)
            .await
            .expect("insert api key");
        generated.raw
    }

    #[tokio::test]
    async fn valid_key_resolves_to_a_superuser_identity() {
        let (app, _dir) = test_app().await;
        let raw = insert_key(&app, "ci", true).await;

        let auth = resolve(&app, &raw).await.expect("should resolve");
        assert!(auth.is_superuser);
        assert_eq!(auth.collection_name, COLLECTION);
    }

    #[tokio::test]
    async fn disabled_key_is_rejected() {
        let (app, _dir) = test_app().await;
        let raw = insert_key(&app, "revoked", false).await;

        assert!(resolve(&app, &raw).await.is_none());
    }

    #[tokio::test]
    async fn unknown_key_is_rejected() {
        let (app, _dir) = test_app().await;
        insert_key(&app, "unrelated", true).await;

        let bogus = format!("{KEY_PREFIX}{}", "z".repeat(RANDOM_LEN));
        assert!(resolve(&app, &bogus).await.is_none());
    }

    /// A key sharing another key's random suffix except for a single
    /// trailing character (so the prefix — the first 8 chars — matches,
    /// forcing the lookup to fall through to a real hash comparison and
    /// reject it) must still fail.
    #[tokio::test]
    async fn key_with_matching_prefix_but_wrong_secret_is_rejected() {
        let (app, _dir) = test_app().await;
        let raw = insert_key(&app, "ci", true).await;
        let mut tampered = raw.clone();
        tampered.push('x');
        assert_ne!(tampered, raw);
        assert!(resolve(&app, &tampered).await.is_none());
    }

    #[tokio::test]
    async fn resolving_stamps_prefix_and_last_used_at() {
        let (app, _dir) = test_app().await;
        let raw = insert_key(&app, "ci", true).await;
        let expected_prefix: String = raw[KEY_PREFIX.len()..].chars().take(PREFIX_LEN).collect();

        let auth = resolve(&app, &raw).await.expect("should resolve");
        assert_eq!(
            auth.record.get("prefix").and_then(Value::as_str),
            Some(expected_prefix.as_str())
        );
        assert!(
            auth.record
                .get("lastUsedAt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .is_empty(),
            "the in-memory record predates the write-back, unlike the row now on disk"
        );

        let collection = app.db().collections.get_by_name(COLLECTION).unwrap();
        let reloaded = cratebase_db::records::find_by_id_raw(app.db(), &collection, &auth.id)
            .await
            .expect("reload");
        let stamped = reloaded
            .get("lastUsedAt")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(
            !stamped.is_empty(),
            "lastUsedAt should be stamped after a successful resolve"
        );
    }
}
