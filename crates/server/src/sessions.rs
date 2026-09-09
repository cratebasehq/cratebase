//! The `_sessions` ledger and O(1) in-memory revocation.
//!
//! Auth tokens stay stateless (`crates/server/src/extract.rs`'s module
//! doc): verification never touches the database. This module adds a
//! *parallel*, best-effort record of who is logged in — one `_sessions`
//! row per minted token — so a caller can list/revoke sessions, without
//! turning verification itself into a database round trip.
//!
//! # Revocation without a DB hit on every request
//!
//! A revoked token's digest lives in [`App`]'s in-memory
//! `revoked_sessions` set (loaded at boot by [`load_revoked`], kept in
//! sync by every function here that flips a row's `revoked` flag).
//! [`is_revoked`] checks an atomic counter first: when nothing has ever
//! been revoked, it returns `false` without hashing the token or taking
//! the lock at all — the overwhelmingly common case for an install that
//! never revokes anything.
//!
//! # Session identity
//!
//! `tokenHash` is `sha256(token)`, hex-encoded — never the raw token,
//! and never a `sid`/`jti` claim (the JWT claim set is frozen; see
//! `crates/server/src/routes/auth.rs`'s module doc). A caller re-derives
//! the same hash from the bearer token it already has, so no session
//! identifier ever needs to travel anywhere new.

use cratebase_core::codes;
use cratebase_core::{Collection, Record};
use cratebase_db::{records, DbError, Executor, Sql};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::app::App;

/// `sha256(token)`.
pub fn digest(token: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize().into()
}

/// Lowercase hex, as stored in `_sessions.tokenHash`.
pub fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let byte = std::str::from_utf8(chunk).ok()?;
        out[i] = u8::from_str_radix(byte, 16).ok()?;
    }
    Some(out)
}

/// Everything a `_sessions` row needs about the login that minted the
/// token, beyond the token itself.
#[derive(Debug, Clone, Default)]
pub struct OriginContext {
    pub fingerprint: String,
    pub ip: String,
    pub user_agent: String,
}

fn insert_local(app: &App, d: [u8; 32]) {
    app.revoked_sessions().write().insert(d);
    app.sync_revoked_len();
}

/// Best-effort insert of a `_sessions` row for a freshly minted token. A
/// failure here must never fail the login it rides along with — logged
/// and swallowed, same contract as `routes::auth::record_login_origin`.
/// No-op when `session_tracking` is off.
pub async fn record(
    app: &App,
    collection: &Collection,
    record: &Record,
    token: &str,
    kind: &str,
    origin: &OriginContext,
    expires_at: i64,
) {
    if !app.config().session_tracking {
        return;
    }
    let Some(sessions) = app.db().collections.get("_sessions") else {
        tracing::warn!("_sessions collection missing; skipping session record");
        return;
    };
    let mut row = Record::new(sessions.clone());
    row.set("collectionRef", Value::String(collection.id.clone()));
    row.set("recordRef", Value::String(record.id().to_string()));
    row.set("tokenHash", Value::String(hex(&digest(token))));
    row.set("kind", Value::String(kind.to_string()));
    row.set("fingerprint", Value::String(origin.fingerprint.clone()));
    row.set("ip", Value::String(origin.ip.clone()));
    row.set("userAgent", Value::String(origin.user_agent.clone()));
    let expires = chrono::DateTime::from_timestamp(expires_at, 0).unwrap_or_else(chrono::Utc::now);
    row.set(
        "expiresAt",
        Value::String(cratebase_core::DateTime::from_utc(expires).to_pb_string()),
    );
    row.set("revoked", Value::Bool(false));
    if let Err(e) = records::create(app.db(), &app.db().collections, &mut row).await {
        if is_same_token(&e) {
            // The claim set is frozen and `exp` has 1-second resolution
            // (`routes::auth`'s module doc), so two mints for the same
            // record inside the same wall-clock second produce
            // byte-identical tokens. The ledger row for that exact token
            // already exists and is still live — a duplicate insert is
            // the token being re-recorded, not a failure worth a WARN.
            tracing::debug!("session already recorded for this token");
        } else {
            tracing::warn!(error = describe(&e), "failed to record session");
        }
    }
}

/// Whether the insert failed only because `tokenHash` is already in the
/// ledger — the unique index `idx_sessions_token`, mapped onto a
/// `validation_not_unique` error by the write path.
fn is_same_token(e: &DbError) -> bool {
    match e {
        DbError::Validation(fields) => {
            fields.len() == 1
                && fields
                    .get("tokenHash")
                    .is_some_and(|f| f.code == codes::NOT_UNIQUE)
        }
        _ => false,
    }
}

/// `DbError::Validation`'s Display is the bare string "validation failed";
/// a swallowed session record is diagnosed only from this one log line, so
/// spell out which field failed and why.
fn describe(e: &DbError) -> String {
    match e {
        DbError::Validation(fields) => fields
            .iter()
            .map(|(name, fe)| format!("{name}: {} ({})", fe.code, fe.message))
            .collect::<Vec<_>>()
            .join("; "),
        other => other.to_string(),
    }
}

/// Revoke the single row matching an exact `(collection, record, token)`
/// triple — used by sign-out and by `auth-refresh` retiring the token it
/// is about to rotate away from. Returns whether a row actually matched.
pub async fn revoke_digest(
    app: &App,
    collection_id: &str,
    record_id: &str,
    d: &[u8; 32],
) -> anyhow::Result<bool> {
    let n = app
        .db()
        .execute(
            r#"UPDATE "_sessions" SET "revoked" = 1 WHERE "collectionRef" = $1 AND "recordRef" = $2 AND "tokenHash" = $3"#,
            &[
                Sql::Text(collection_id.to_string()),
                Sql::Text(record_id.to_string()),
                Sql::Text(hex(d)),
            ],
        )
        .await?;
    insert_local(app, *d);
    Ok(n > 0)
}

/// Revoke one `_sessions` row by its own id — used by
/// `DELETE .../sessions/{id}`.
pub async fn revoke_row(app: &App, row_id: &str) -> anyhow::Result<()> {
    let Some(row) = app
        .db()
        .query_one(
            r#"SELECT "tokenHash" FROM "_sessions" WHERE "id" = $1"#,
            &[Sql::Text(row_id.to_string())],
        )
        .await?
    else {
        return Ok(());
    };
    let Some(hash) = row.get_str("tokenHash").map(str::to_string) else {
        return Ok(());
    };
    app.db()
        .execute(
            r#"UPDATE "_sessions" SET "revoked" = 1 WHERE "id" = $1"#,
            &[Sql::Text(row_id.to_string())],
        )
        .await?;
    if let Some(d) = from_hex(&hash) {
        insert_local(app, d);
    }
    Ok(())
}

/// Revoke every still-live session for `(collection_id, record_id)`,
/// optionally sparing one digest (the caller's own current session).
/// Returns how many rows were revoked.
pub async fn revoke_all_for(
    app: &App,
    collection_id: &str,
    record_id: &str,
    except: Option<&[u8; 32]>,
) -> anyhow::Result<usize> {
    let rows = app
        .db()
        .query(
            r#"SELECT "tokenHash" FROM "_sessions" WHERE "collectionRef" = $1 AND "recordRef" = $2 AND "revoked" = 0"#,
            &[
                Sql::Text(collection_id.to_string()),
                Sql::Text(record_id.to_string()),
            ],
        )
        .await?;
    let except_hex = except.map(hex);
    let targets: Vec<String> = rows
        .iter()
        .filter_map(|r| r.get_str("tokenHash").map(str::to_string))
        .filter(|h| except_hex.as_deref() != Some(h.as_str()))
        .collect();
    if targets.is_empty() {
        return Ok(0);
    }

    match &except_hex {
        Some(eh) => {
            app.db()
                .execute(
                    r#"UPDATE "_sessions" SET "revoked" = 1 WHERE "collectionRef" = $1 AND "recordRef" = $2 AND "revoked" = 0 AND "tokenHash" != $3"#,
                    &[
                        Sql::Text(collection_id.to_string()),
                        Sql::Text(record_id.to_string()),
                        Sql::Text(eh.clone()),
                    ],
                )
                .await?;
        }
        None => {
            app.db()
                .execute(
                    r#"UPDATE "_sessions" SET "revoked" = 1 WHERE "collectionRef" = $1 AND "recordRef" = $2 AND "revoked" = 0"#,
                    &[
                        Sql::Text(collection_id.to_string()),
                        Sql::Text(record_id.to_string()),
                    ],
                )
                .await?;
        }
    }

    {
        let mut set = app.revoked_sessions().write();
        for hash in &targets {
            if let Some(d) = from_hex(hash) {
                set.insert(d);
            }
        }
    }
    app.sync_revoked_len();
    Ok(targets.len())
}

/// Boot-time load of every still-relevant revoked digest, so the
/// in-memory set survives a restart. Expired rows are skipped — they are
/// unforgeable (the signature itself would already reject them) and get
/// dropped from the table by [`sweep_expired`] anyway.
pub async fn load_revoked(app: &App) -> anyhow::Result<()> {
    let now = cratebase_core::DateTime::now().to_pb_string();
    let rows = app
        .db()
        .query(
            r#"SELECT "tokenHash" FROM "_sessions" WHERE "revoked" = 1 AND "expiresAt" > $1"#,
            &[Sql::Text(now)],
        )
        .await?;
    {
        let mut set = app.revoked_sessions().write();
        for row in &rows {
            if let Some(d) = row.get_str("tokenHash").and_then(from_hex) {
                set.insert(d);
            }
        }
    }
    app.sync_revoked_len();
    Ok(())
}

/// `is_revoked`'s fast path check plus the real one: a zero counter
/// means nothing has ever been revoked, so the token is never even
/// hashed.
pub fn is_revoked(app: &App, token: &str) -> bool {
    if app.revoked_len() == 0 {
        return false;
    }
    let d = digest(token);
    app.revoked_sessions().read().contains(&d)
}

/// Hourly cron body (`cron::JOB_SESSION_SWEEP`): drop expired rows and
/// their digests, so the ledger and the in-memory set don't grow
/// unbounded.
pub async fn sweep_expired(app: &App) {
    let now = cratebase_core::DateTime::now().to_pb_string();
    let rows = match app
        .db()
        .query(
            r#"SELECT "tokenHash" FROM "_sessions" WHERE "expiresAt" < $1"#,
            &[Sql::Text(now.clone())],
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "session sweep query failed");
            return;
        }
    };
    if let Err(e) = app
        .db()
        .execute(
            r#"DELETE FROM "_sessions" WHERE "expiresAt" < $1"#,
            &[Sql::Text(now)],
        )
        .await
    {
        tracing::warn!(error = %e, "session sweep delete failed");
        return;
    }
    if rows.is_empty() {
        return;
    }
    {
        let mut set = app.revoked_sessions().write();
        for row in &rows {
            if let Some(d) = row.get_str("tokenHash").and_then(from_hex) {
                set.remove(&d);
            }
        }
    }
    app.sync_revoked_len();
}

/// One row of `sessions.list()` — a record's own live sessions.
#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    pub kind: String,
    pub fingerprint: String,
    pub ip: String,
    pub user_agent: String,
    pub token_hash: String,
    pub created: String,
    pub last_seen_at: Option<String>,
    pub expires_at: String,
}

/// A record's own live (non-revoked) sessions, newest first.
pub async fn list_for(app: &App, collection_id: &str, record_id: &str) -> anyhow::Result<Vec<Row>> {
    let rows = app
        .db()
        .query(
            r#"SELECT "id", "kind", "fingerprint", "ip", "userAgent", "tokenHash", "created", "lastSeenAt", "expiresAt" FROM "_sessions" WHERE "collectionRef" = $1 AND "recordRef" = $2 AND "revoked" = 0 ORDER BY "created" DESC LIMIT 200"#,
            &[
                Sql::Text(collection_id.to_string()),
                Sql::Text(record_id.to_string()),
            ],
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| Row {
            id: r.get_str("id").unwrap_or_default().to_string(),
            kind: r.get_str("kind").unwrap_or_default().to_string(),
            fingerprint: r.get_str("fingerprint").unwrap_or_default().to_string(),
            ip: r.get_str("ip").unwrap_or_default().to_string(),
            user_agent: r.get_str("userAgent").unwrap_or_default().to_string(),
            token_hash: r.get_str("tokenHash").unwrap_or_default().to_string(),
            created: r.get_str("created").unwrap_or_default().to_string(),
            last_seen_at: r.get_str("lastSeenAt").map(str::to_string),
            expires_at: r.get_str("expiresAt").unwrap_or_default().to_string(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cratebase_core::FieldError;
    use std::collections::BTreeMap;

    #[test]
    fn a_duplicate_token_hash_is_the_same_token() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "tokenHash".to_string(),
            FieldError::new(codes::NOT_UNIQUE, "Value must be unique."),
        );
        assert!(is_same_token(&DbError::Validation(fields)));

        // Any other validation failure is a real one: a required field
        // left blank (the auth-refresh empty fingerprint, for one) must
        // keep warning.
        let mut fields = BTreeMap::new();
        fields.insert(
            "fingerprint".to_string(),
            FieldError::new(codes::REQUIRED, "Cannot be blank."),
        );
        assert!(!is_same_token(&DbError::Validation(fields)));
        assert!(!is_same_token(&DbError::NotFound));
    }

    #[test]
    fn validation_errors_are_described_field_by_field() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "fingerprint".to_string(),
            FieldError::new(codes::REQUIRED, "Cannot be blank."),
        );
        assert_eq!(
            describe(&DbError::Validation(fields)),
            "fingerprint: validation_required (Cannot be blank.)"
        );
        assert_eq!(describe(&DbError::NotFound), "not found");
    }
}
