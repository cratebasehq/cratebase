//! Backing store for `_otp_codes`, shared by the OTP-login flow
//! (`request-otp`/`auth-with-otp`) and the MFA second factor
//! (`auth-with-password` + `mfa/confirm`) in `cratebase-server`. Both
//! flows only ever deal in pre-hashed codes (`cratebase_auth::hash_otp`) —
//! this module never sees a plaintext code.

use cratebase_core::new_id;
use sqlx::Row;

use crate::error::DbResult;
use crate::pool::Db;

/// RFC3339 with millisecond precision, matching `cratebase_core::now()` —
/// keeping both columns in the same string format is what makes the plain
/// `expires_at > $now` comparison in [`verify_and_consume`] correct on
/// both SQLite and Postgres.
fn timestamp(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Stores a hashed OTP code for `record_id` in `collection_id`, expiring
/// `ttl_seconds` from now. Doesn't first delete any still-live code for
/// the same record — a record can have several outstanding codes (e.g. a
/// second `request-otp` before the first code expired); `verify_and_consume`
/// accepts any of them and consumes them all together.
pub async fn create(
    db: &Db,
    collection_id: &str,
    record_id: &str,
    code_hash: &str,
    ttl_seconds: i64,
) -> DbResult<()> {
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO _otp_codes (id, collection_id, record_id, code_hash, expires_at, created)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(new_id())
    .bind(collection_id)
    .bind(record_id)
    .bind(code_hash)
    .bind(timestamp(now + chrono::Duration::seconds(ttl_seconds)))
    .bind(timestamp(now))
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// Checks `code_hash` against every unexpired OTP row for `(collection_id,
/// record_id)`, then deletes every row for that pair regardless of the
/// outcome — a wrong guess burns the code exactly like a right one does,
/// so a caller can't keep guessing against a still-valid code and a stale
/// code can't accumulate forever even if it's never checked again.
pub async fn verify_and_consume(
    db: &Db,
    collection_id: &str,
    record_id: &str,
    code_hash: &str,
) -> DbResult<bool> {
    let now = timestamp(chrono::Utc::now());
    let row = sqlx::query(
        "SELECT id FROM _otp_codes
         WHERE collection_id = $1 AND record_id = $2 AND code_hash = $3 AND expires_at > $4",
    )
    .bind(collection_id)
    .bind(record_id)
    .bind(code_hash)
    .bind(&now)
    .fetch_optional(&db.pool)
    .await?;
    let matched = row.map(|r| r.try_get::<String, _>("id")).transpose()?.is_some();

    sqlx::query("DELETE FROM _otp_codes WHERE collection_id = $1 AND record_id = $2")
        .bind(collection_id)
        .bind(record_id)
        .execute(&db.pool)
        .await?;

    Ok(matched)
}
