//! The optional per-user storage quota (`settings.storage.userQuotaBytes`,
//! `0` = disabled, the default) — see the module doc on
//! [`crate::presign`] for the presigned-upload flow this gates.
//!
//! # Scope (read this before assuming it counts everything)
//!
//! This deliberately does **not** track every byte a user has ever
//! uploaded anywhere in the app — that would mean summing real object
//! sizes (a storage `HEAD` per file, for every file, on every check) or
//! maintaining a running counter updated from every create/update/delete
//! path across every collection, for every file field. Both are exactly
//! the kind of complexity the brief this shipped against explicitly says
//! is fine to route around ("skip if it gets complicated ... keep it
//! simple and document").
//!
//! What it actually does: usage is the sum of `size` already recorded on
//! this collection's **consumed presigned-upload tickets**
//! (`_pendingUploads`, `status = 'consumed'`) whose `recordRef` is a
//! record this collection's [`Collection::owner_field`] points at the
//! requesting auth record — a single indexed-enough query, no storage
//! calls, using data already being written for an unrelated reason
//! (`presign::resolve_one` marks a ticket consumed). Enforced only at
//! `POST /api/files/presign`, before a ticket is minted: an ordinary
//! (non-presigned) multipart upload through the normal record
//! create/update endpoint is **not currently counted against or gated
//! by** this quota. A collection with no `ownerField` configured is
//! never gated at all, whatever `userQuotaBytes` is set to.
use cratebase_db::{quote_ident, Executor, Sql};

use crate::app::App;

/// Bytes already consumed, per [`Collection::owner_field`]'s definition,
/// by `owner_id`'s presigned uploads into `collection`. `None` when the
/// collection has no `ownerField` (quota never applies to it).
pub async fn consumed_bytes(
    app: &App,
    collection: &cratebase_core::Collection,
    owner_id: &str,
) -> Option<i64> {
    // `owner_field` must actually name a field on this collection —
    // refuse to build SQL around an operator typo or a stale setting
    // left over from a renamed/removed field, rather than quoting
    // whatever string is there and letting Postgres/SQLite reject it (or
    // worse, silently match nothing) at query time.
    let owner_field = collection.owner_field.as_ref()?;
    collection.field(owner_field)?;
    let quoted_owner = quote_ident(owner_field);
    let quoted_table = quote_ident(&collection.name);
    let sql = format!(
        r#"SELECT COALESCE(SUM("size"), 0) AS total FROM "_pendingUploads"
           WHERE "status" = 'consumed' AND "collectionRef" = $1
             AND "recordRef" IN (SELECT "id" FROM {quoted_table} WHERE {quoted_owner} = $2)"#
    );
    let total = app
        .db()
        .query_scalar(
            &sql,
            &[
                Sql::from(collection.id.clone()),
                Sql::from(owner_id.to_string()),
            ],
        )
        .await
        .ok()??;
    total.as_i64()
}

/// `Some(message)` when minting a ticket for `additional_bytes` more
/// would push `owner_id` over `settings.storage.userQuotaBytes` — the
/// caller turns that into a `400`. `None` means "not gated, or under
/// quota": quota disabled (`userQuotaBytes == 0`), no `ownerField`
/// configured on this collection, or an anonymous request (nothing to
/// meter against).
pub async fn exceeded(
    app: &App,
    collection: &cratebase_core::Collection,
    owner_id: Option<&str>,
    additional_bytes: i64,
) -> Option<String> {
    let limit = app.settings().storage.user_quota_bytes;
    if limit <= 0 {
        return None;
    }
    let owner_id = owner_id?;
    let used = consumed_bytes(app, collection, owner_id).await?;
    if used + additional_bytes > limit {
        Some(format!(
            "Storage quota exceeded: {used} of {limit} bytes already used, this upload needs {additional_bytes} more."
        ))
    } else {
        None
    }
}
