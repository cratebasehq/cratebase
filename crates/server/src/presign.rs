//! Presigned direct uploads (`POST /api/files/presign`): the client
//! uploads bytes straight to the object store (S3) — or, for the local
//! driver, to a same-origin route this server handles itself
//! (`PUT /api/files/presign-upload/{token}`) — then claims the upload by
//! sending the opaque token back as a normal file field's value in a
//! record create/update.
//!
//! # Token shape and lifecycle
//!
//! The token is `"CBUP_" + 32 random alphanumeric chars`. The uppercase
//! prefix can never collide with a real stored file name
//! (`crate::routes::common::stored_file_name` only ever produces
//! lowercase `[a-z0-9_]` names), so a file field's incoming string value
//! is unambiguously either a token to resolve or an ordinary filename.
//! Only `sha256(token)` is ever persisted (`_pendingUploads.tokenHash`),
//! the same convention as `_sessions`/`_magicLinks`.
//!
//! `POST /api/files/presign` validates the target field's rules
//! (create/update rule, `maxSize`, `mimeTypes`) up front, so a client
//! can't waste an upload on a request that would be rejected anyway,
//! then returns a presigned PUT URL (S3) or a same-origin upload URL
//! (local) plus the token. [`resolve_body_tokens`] — called from
//! `crate::routes::records`'s create/update handlers — resolves the
//! token, verifying the object actually landed at the expected key with
//! (at least) the declared size via [`cratebase_storage::Storage::size`],
//! and marks the ticket consumed so it can't be replayed onto a second
//! record.
//!
//! A create presign has no record to attach the rule check to yet, so
//! its `recordId` is minted up front (before the object exists) and
//! returned in the response; the caller must send that same id as the
//! new record's own `id` when it later creates the record, or the
//! ticket's `recordRef` won't match and the token is refused.
//!
//! Expired, never-claimed tickets (and their orphaned objects) are swept
//! hourly by [`sweep_expired`] (`cron::JOB_PENDING_UPLOADS_SWEEP`).

use cratebase_core::{FieldKind, Record};
use cratebase_db::context::CollectionResolver;
use cratebase_db::{records, rules, Executor, Sql};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::extract::RequestInfo;
use crate::http_error::{ApiError, ApiResult};
use crate::routes::common;

/// How long a presigned upload ticket is valid before the client must
/// have claimed it in a create/update call.
pub const TTL_SECONDS: i64 = 30 * 60;

const TOKEN_ALPHABET: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const TOKEN_RANDOM_LEN: usize = 32;
/// Uppercase and never produced by [`common::stored_file_name`] (always
/// lowercase), so a file field's string value is unambiguous.
pub const TOKEN_PREFIX: &str = "CBUP_";

fn new_token() -> String {
    format!(
        "{TOKEN_PREFIX}{}",
        cratebase_core::ids::random_string(TOKEN_RANDOM_LEN, TOKEN_ALPHABET)
    )
}

/// Whether `value` looks like a presign token rather than a stored file
/// name.
pub fn is_token(value: &str) -> bool {
    value.starts_with(TOKEN_PREFIX)
}

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// `POST /api/files/presign`'s body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresignRequest {
    pub collection: String,
    pub field: String,
    pub filename: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub size: i64,
    /// Required to presign an *update* to an existing record's field;
    /// omit to presign ahead of a *create* — see the module doc on why
    /// the response's `recordId` then has to travel back into the
    /// create call.
    #[serde(default)]
    pub record_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresignResponse {
    pub token: String,
    pub record_id: String,
    pub upload_url: String,
    pub upload_method: String,
    /// The final file name — what the token resolves to once claimed,
    /// and what the record's field value will read.
    pub filename: String,
    pub expires_at: String,
}

/// A record's own fields as a plain JSON map, the shape
/// [`rules::check_rule_against_row`] wants — `record.data()` minus the
/// `IndexMap`/`serde_json::Map` type difference.
fn row_map(record: &cratebase_core::Record) -> Map<String, Value> {
    record.data().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Core logic behind `POST /api/files/presign`, split out so it takes
/// plain values rather than axum extractors.
pub async fn create_presign(
    app: &App,
    info: &RequestInfo,
    req: PresignRequest,
) -> ApiResult<PresignResponse> {
    let collection = common::collection_of(app, &req.collection)?;
    let field = collection
        .field(&req.field)
        .ok_or_else(|| ApiError::bad_request(format!("Unknown field '{}'.", req.field)))?;
    let FieldKind::File {
        max_size,
        mime_types,
        ..
    } = &field.kind
    else {
        return Err(ApiError::bad_request(format!(
            "'{}' is not a file field.",
            req.field
        )));
    };
    if req.filename.trim().is_empty() {
        return Err(ApiError::bad_request("filename is required."));
    }
    if *max_size > 0 && req.size > *max_size {
        return Err(ApiError::bad_request(format!(
            "The maximum allowed file size is {max_size} bytes."
        )));
    }
    if !mime_types.is_empty() && !mime_types.iter().any(|m| m == &req.content_type) {
        return Err(ApiError::bad_request(format!(
            "content type must be one of: {}.",
            mime_types.join(", ")
        )));
    }

    let ctx = info.to_context();
    let resolver = CollectionResolver::new(
        collection.clone(),
        &app.db().collections,
        &ctx,
        app.db().dialect(),
    );

    let record_id = match &req.record_id {
        Some(id) => {
            let existing = records::find_by_id_raw(app.db(), &collection, id)
                .await
                .map_err(|_| ApiError::not_found(""))?;
            let allowed = rules::check_rule_against_row(
                app.db(),
                &resolver,
                &collection.update_rule,
                &row_map(&existing),
            )
            .await
            .map_err(|e| ApiError(crate::routes::records::read_error(e)))?;
            if !allowed {
                return Err(ApiError::not_found(""));
            }
            id.clone()
        }
        None => {
            let allowed = rules::check_rule_against_row(
                app.db(),
                &resolver,
                &collection.create_rule,
                &Map::new(),
            )
            .await
            .map_err(|e| ApiError(crate::routes::records::read_error(e)))?;
            if !allowed {
                return Err(ApiError::bad_request("Failed to create record."));
            }
            cratebase_core::record_id()
        }
    };

    let stored_name = common::stored_file_name(&req.filename);
    let key = common::file_key(&collection.id, &record_id, &stored_name);
    let token = new_token();
    let expires_at = cratebase_core::DateTime::from_utc(
        chrono::Utc::now() + chrono::Duration::seconds(TTL_SECONDS),
    );

    let pending = app
        .db()
        .collections
        .get("_pendingUploads")
        .ok_or_else(|| ApiError::internal("_pendingUploads collection missing"))?;
    let mut row = Record::new(pending);
    row.set("collectionRef", Value::String(collection.id.clone()));
    row.set("field", Value::String(field.name.clone()));
    row.set("recordRef", Value::String(record_id.clone()));
    row.set("filename", Value::String(stored_name.clone()));
    row.set("key", Value::String(key.clone()));
    row.set("size", Value::from(req.size));
    row.set("mime", Value::String(req.content_type.clone()));
    row.set("tokenHash", Value::String(hash_token(&token)));
    row.set("status", Value::String("pending".into()));
    row.set("expiresAt", Value::String(expires_at.to_pb_string()));
    records::create(app.db(), &app.db().collections, &mut row)
        .await
        .map_err(|e| ApiError(crate::routes::records::read_error(e)))?;

    let storage = app.storage();
    let (upload_url, upload_method) = match storage
        .presign_put(&key, std::time::Duration::from_secs(TTL_SECONDS as u64))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
    {
        Some(url) => (url.to_string(), "PUT".to_string()),
        None => (
            format!("/api/files/presign-upload/{token}"),
            "PUT".to_string(),
        ),
    };

    Ok(PresignResponse {
        token,
        record_id,
        upload_url,
        upload_method,
        filename: stored_name,
        expires_at: expires_at.to_pb_string(),
    })
}

/// One pending-upload row, as looked up by token.
struct PendingUpload {
    id: String,
    collection_ref: String,
    field: String,
    record_ref: String,
    filename: String,
    key: String,
    size: i64,
    mime: String,
    status: String,
    expires_at: String,
}

async fn find_pending(app: &App, token: &str) -> Option<PendingUpload> {
    let hash = hash_token(token);
    let row = app
        .db()
        .query_one(
            r#"SELECT "id", "collectionRef", "field", "recordRef", "filename", "key", "size",
                "mime", "status", "expiresAt"
               FROM "_pendingUploads" WHERE "tokenHash" = $1"#,
            &[Sql::Text(hash)],
        )
        .await
        .ok()??;
    Some(PendingUpload {
        id: row.get_str("id")?.to_string(),
        collection_ref: row.get_str("collectionRef")?.to_string(),
        field: row.get_str("field")?.to_string(),
        record_ref: row.get_str("recordRef")?.to_string(),
        filename: row.get_str("filename")?.to_string(),
        key: row.get_str("key")?.to_string(),
        size: row.get_i64("size").unwrap_or(0),
        mime: row.get_str("mime").unwrap_or_default().to_string(),
        status: row.get_str("status")?.to_string(),
        expires_at: row.get_str("expiresAt")?.to_string(),
    })
}

async fn delete_pending(app: &App, id: &str) {
    if let Err(e) = app
        .db()
        .execute(
            r#"DELETE FROM "_pendingUploads" WHERE "id" = $1"#,
            &[Sql::Text(id.to_string())],
        )
        .await
    {
        tracing::warn!(error = %e, "failed to delete a consumed/expired pending upload");
    }
}

/// `PUT /api/files/presign-upload/{token}` — the local-storage fallback
/// upload route. The token is the bearer (same trust model as an S3
/// presigned URL's own signature), so no separate auth is checked here;
/// [`resolve_body_tokens`] is what actually attaches the file to a
/// record, and only after re-validating the ticket.
pub async fn store_local_upload(app: &App, token: &str, body: bytes::Bytes) -> ApiResult<()> {
    if !is_token(token) {
        return Err(ApiError::not_found(""));
    }
    let pending = find_pending(app, token)
        .await
        .ok_or_else(|| ApiError::not_found(""))?;
    if pending.status != "pending" {
        return Err(ApiError::bad_request(
            "This upload token has already been used.",
        ));
    }
    if pending.expires_at.as_str() < cratebase_core::DateTime::now().to_pb_string().as_str() {
        delete_pending(app, &pending.id).await;
        return Err(ApiError::bad_request("This upload token has expired."));
    }
    if body.len() as i64 != pending.size {
        return Err(ApiError::bad_request(format!(
            "Uploaded body is {} bytes, expected {} bytes.",
            body.len(),
            pending.size
        )));
    }
    app.storage()
        .put(&pending.key, body)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

/// Resolve every `CBUP_...` token in a file field of `body.data` for
/// `collection`/`record_id`, replacing it with the resolved file name and
/// appending an `already_stored` [`common::StagedUpload`] to
/// `body.uploads` so the ordinary `maxSize`/`mimeTypes` validation and
/// upload-rollback bookkeeping apply exactly as they do to a multipart
/// upload. A single-valued field's token is a bare string; a
/// multi-valued field's is any string element of its array that starts
/// with the token prefix.
///
/// Called from `crate::routes::records`'s create/update handlers right
/// after the body is parsed (before `input`/`Record` are built from it),
/// so the resolved file name — not the raw token — is what ends up
/// validated, stored and returned.
pub async fn resolve_body_tokens(
    app: &App,
    collection: &cratebase_core::Collection,
    record_id: &str,
    body: &mut common::ParsedBody,
) -> ApiResult<()> {
    for field in collection.fields_of_type(cratebase_core::FieldType::File) {
        let Some(value) = body.data.get(&field.name).cloned() else {
            continue;
        };
        match value {
            Value::String(s) if is_token(&s) => {
                let (resolved, upload) =
                    resolve_one(app, collection, record_id, &field.name, &s).await?;
                body.data
                    .insert(field.name.clone(), Value::String(resolved));
                body.uploads.push(upload);
            }
            Value::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    match item {
                        Value::String(s) if is_token(&s) => {
                            let (resolved, upload) =
                                resolve_one(app, collection, record_id, &field.name, &s).await?;
                            out.push(Value::String(resolved));
                            body.uploads.push(upload);
                        }
                        other => out.push(other),
                    }
                }
                body.data.insert(field.name.clone(), Value::Array(out));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Resolve one token into its final file name plus a
/// [`common::StagedUpload`] the caller appends to `body.uploads` — see
/// [`resolve_body_tokens`]'s doc for why that, rather than a second
/// write, is enough to fold into the ordinary validation/rollback path.
async fn resolve_one(
    app: &App,
    collection: &cratebase_core::Collection,
    record_id: &str,
    field_name: &str,
    token: &str,
) -> ApiResult<(String, common::StagedUpload)> {
    let pending = find_pending(app, token)
        .await
        .ok_or_else(|| ApiError::bad_request("Unknown or already-used upload token."))?;
    if pending.status != "pending" {
        return Err(ApiError::bad_request(
            "This upload token has already been used.",
        ));
    }
    if pending.expires_at.as_str() < cratebase_core::DateTime::now().to_pb_string().as_str() {
        delete_pending(app, &pending.id).await;
        return Err(ApiError::bad_request("This upload token has expired."));
    }
    if pending.collection_ref != collection.id
        || pending.field != field_name
        || pending.record_ref != record_id
    {
        return Err(ApiError::bad_request(
            "This upload token was issued for a different collection, field or record.",
        ));
    }

    let storage = app.storage();
    let actual_size = storage
        .size(&pending.key)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::bad_request("The presigned upload has not landed in storage yet.")
        })?;
    if actual_size != pending.size as u64 {
        return Err(ApiError::bad_request(format!(
            "Uploaded object size ({actual_size} bytes) does not match the presigned size ({} bytes).",
            pending.size
        )));
    }

    if let Err(e) = app
        .db()
        .execute(
            r#"UPDATE "_pendingUploads" SET "status" = 'consumed' WHERE "id" = $1"#,
            &[Sql::Text(pending.id.clone())],
        )
        .await
    {
        tracing::warn!(error = %e, "failed to mark a pending upload consumed");
    }

    let upload = common::StagedUpload::already_stored(
        field_name.to_string(),
        pending.filename.clone(),
        actual_size as i64,
        pending.mime.clone(),
    );
    Ok((pending.filename, upload))
}
