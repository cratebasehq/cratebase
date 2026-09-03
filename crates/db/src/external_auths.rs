use cratebase_core::{new_id, now};
use sqlx::Row;

use crate::error::DbResult;
use crate::pool::Db;

/// Finds the record id linked to `(collection_id, provider, provider_user_id)`,
/// if any provider account has ever been linked to a record for this login.
pub async fn find_linked_record(
    db: &Db,
    collection_id: &str,
    provider: &str,
    provider_user_id: &str,
) -> DbResult<Option<String>> {
    let row = sqlx::query(
        "SELECT record_id FROM _external_auths
         WHERE collection_id = $1 AND provider = $2 AND provider_user_id = $3",
    )
    .bind(collection_id)
    .bind(provider)
    .bind(provider_user_id)
    .fetch_optional(&db.pool)
    .await?;
    Ok(match row {
        Some(r) => Some(r.try_get("record_id")?),
        None => None,
    })
}

/// Links a provider identity to `record_id`. Idempotent in practice: the
/// caller only reaches this after `find_linked_record` came back empty, so
/// a unique-constraint conflict here means a genuine race, not a normal
/// repeat login (which short-circuits through `find_linked_record`).
pub async fn link(
    db: &Db,
    collection_id: &str,
    record_id: &str,
    provider: &str,
    provider_user_id: &str,
) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO _external_auths (id, collection_id, record_id, provider, provider_user_id, created)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(new_id())
    .bind(collection_id)
    .bind(record_id)
    .bind(provider)
    .bind(provider_user_id)
    .bind(now())
    .execute(&db.pool)
    .await?;
    Ok(())
}
