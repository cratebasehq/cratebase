//! Pieces shared by the record, file and auth routes: collection lookup,
//! rule probes, request-body parsing (JSON and multipart) and the
//! file-staging lifecycle.
//!
//! # Why body parsing lives here rather than in an extractor
//!
//! A records request needs the *collection* before it can decide what a
//! multipart part means — a part named `cover` is a file only if `cover`
//! is a file field, and `attachments+` has to be resolved through the
//! `+`/`-` modifier grammar first. An axum extractor runs before the
//! handler has looked anything up, so the parse is a function the handler
//! calls once it knows which collection it is talking to.
//!
//! # Staging
//!
//! Uploads are staged before the row is written, because PocketBase
//! validates `maxSize`/`mimeTypes`/`maxSelect` against the *pending*
//! files and the record's file columns must already hold their final
//! names when the insert runs. Staging keeps small files in memory and
//! spills anything larger than [`SPILL_THRESHOLD`] to a temp file, so a
//! 2 GB upload costs a 2 GB disk write and not a 2 GB allocation. The
//! bytes only reach the object store once the record id is known, and are
//! removed again if the write fails.

use std::sync::Arc;

use axum::extract::Multipart;
use bytes::{Bytes, BytesMut};
use cratebase_core::{AppError, Collection, FieldType, Record, SerializeOptions};
use cratebase_db::collections::CollectionStore;
use cratebase_db::context::{CollectionResolver, RequestContext};
use cratebase_db::engine::{quote_ident, Executor, Sql};
use cratebase_db::query::Query;
use cratebase_db::{rules, DbError, UploadMeta};
use cratebase_storage::Storage;
use serde_json::{Map, Value};

use crate::app::App;
use crate::extract::Auth;
use crate::http_error::{rule_errors, ApiError, ApiResult};

/// Multipart key carrying a whole JSON body alongside the file parts.
pub const JSON_PAYLOAD: &str = "@jsonPayload";

/// Files larger than this are written to a temp file instead of being
/// held in memory while the rest of the request is parsed.
pub const SPILL_THRESHOLD: usize = 2 * 1024 * 1024;

/// Random suffix length in a stored file name (`notes_<10>.txt`).
const SUFFIX_LEN: usize = 10;
/// PocketBase pads a very short base name so the result is not just the
/// random suffix.
const MIN_BASE_LEN: usize = 3;
/// Lowercase alphanumerics — the conformance suite pins the stored name
/// to `[a-z0-9]`.
const NAME_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

// --------------------------------------------------------------- lookups

/// The collection behind an `{idOrName}` path segment, from the in-memory
/// store. Never queries `_collections`.
pub fn collection_of(app: &App, name_or_id: &str) -> ApiResult<Arc<Collection>> {
    app.db()
        .collections
        .get(name_or_id)
        .ok_or_else(|| ApiError(rule_errors::missing_collection()))
}

/// Like [`collection_of`], but for the auth endpoints, which report a
/// base collection differently from a missing one (KNOWN_DIVERGENCES §19).
pub fn auth_collection_of(app: &App, name_or_id: &str) -> ApiResult<Arc<Collection>> {
    let collection = app
        .db()
        .collections
        .get(name_or_id)
        .ok_or_else(|| ApiError(rule_errors::missing_collection()))?;
    if !collection.is_auth() {
        return Err(ApiError::not_found(
            "Missing or invalid auth collection context.",
        ));
    }
    Ok(collection)
}

// ----------------------------------------------------------------- rules

/// Whether the stored record `id` satisfies `rule` for this caller.
///
/// Used for `manageRule` and for the protected-file `viewRule` gate, both
/// of which are "does this specific row pass?" questions that
/// [`cratebase_db::records`] does not expose (its readers apply the rule
/// as a filter and report absence).
pub async fn record_matches_rule(
    ex: &dyn Executor,
    store: &CollectionStore,
    ctx: &RequestContext,
    collection: &Arc<Collection>,
    rule: &Option<String>,
    id: &str,
) -> Result<bool, DbError> {
    if ctx.is_superuser() {
        return Ok(true);
    }
    let resolver = CollectionResolver::new(collection.clone(), store, ctx, ex.dialect());
    let outcome = rules::evaluate(rule, &resolver, 0)?;
    if outcome.is_deny_all() {
        return Ok(false);
    }
    let mut query = Query::new(collection);
    if let Some(filter) = outcome.into_filter() {
        query.push_filter(filter);
    }
    let placeholder = query.next_placeholder();
    query.push_condition(
        format!(
            "{}.\"id\" = ${placeholder}",
            quote_ident(collection.table_name())
        ),
        vec![Sql::Text(id.to_string())],
    );
    let sql = query.select_sql();
    query.bind_page(1, 0);
    Ok(ex.query_one(&sql, query.params()).await?.is_some())
}

/// The subset of `ids` that satisfies `rule`, in a single statement.
///
/// The per-row form would be one query per listed record; a page of 500
/// auth records under a `manageRule` would then cost 500 round trips just
/// to decide whether to show an email.
pub async fn records_matching_rule(
    ex: &dyn Executor,
    store: &CollectionStore,
    ctx: &RequestContext,
    collection: &Arc<Collection>,
    rule: &Option<String>,
    ids: &[String],
) -> Result<std::collections::HashSet<String>, DbError> {
    if ids.is_empty() {
        return Ok(Default::default());
    }
    if ctx.is_superuser() {
        return Ok(ids.iter().cloned().collect());
    }
    let resolver = CollectionResolver::new(collection.clone(), store, ctx, ex.dialect());
    let outcome = rules::evaluate(rule, &resolver, 0)?;
    if outcome.is_deny_all() {
        return Ok(Default::default());
    }
    let mut query = Query::new(collection);
    if let Some(filter) = outcome.into_filter() {
        query.push_filter(filter);
    }
    let start = query.next_placeholder();
    query.push_condition(
        format!(
            "{}.\"id\" IN ({})",
            quote_ident(collection.table_name()),
            cratebase_db::query::in_placeholders(start, ids.len())
        ),
        ids.iter().map(|id| Sql::Text(id.clone())).collect(),
    );
    let sql = query.select_sql();
    query.bind_page(ids.len() as i64, 0);
    let rows = ex.query(&sql, query.params()).await?;
    Ok(rows
        .iter()
        .filter_map(|r| r.get_str("id").map(str::to_string))
        .collect())
}

/// PocketBase's "auth manage access": a superuser, or a caller the
/// collection's `manageRule` lets through for this record. A `manageRule`
/// of `null` means superusers only.
pub async fn has_manage_access(
    ex: &dyn Executor,
    store: &CollectionStore,
    ctx: &RequestContext,
    collection: &Arc<Collection>,
    record_id: &str,
) -> Result<bool, DbError> {
    if ctx.is_superuser() {
        return Ok(true);
    }
    if !collection.is_auth() || collection.auth.manage_rule.is_none() || record_id.is_empty() {
        return Ok(false);
    }
    record_matches_rule(
        ex,
        store,
        ctx,
        collection,
        &collection.auth.manage_rule,
        record_id,
    )
    .await
}

// --------------------------------------------------------- serialization

/// Whether the viewer may see an auth record's `email` regardless of
/// `emailVisibility`: it is their own record, they are a superuser, or
/// they hold manage access.
pub fn show_email_for(auth: Option<&Auth>, record: &Record, manage_access: bool) -> bool {
    if manage_access {
        return true;
    }
    match auth {
        Some(a) => {
            a.is_superuser || (a.collection_id == record.collection.id && a.id == record.id())
        }
        None => false,
    }
}

/// Run `onRecordEnrich` and serialize. One serialization per record —
/// the projection in [`project`] mutates the same value in place.
pub async fn enrich_and_serialize(
    app: &App,
    collection: &Arc<Collection>,
    record: Record,
    auth: Option<Auth>,
    show_email: bool,
) -> Result<Value, ApiError> {
    let mut event = crate::events::RecordEnrichEvent::new(
        app.clone(),
        collection.clone(),
        record,
        auth,
        crate::events::collection_tags(collection),
    );
    app.hooks()
        .on_record_enrich
        .trigger_bare(&mut event)
        .await
        .map_err(ApiError)?;
    Ok(event.record.to_json(SerializeOptions {
        with_hidden: false,
        show_email,
        with_custom_data: true,
    }))
}

/// Apply `?fields=` if present.
pub fn project(value: &mut Value, fields: Option<&str>) {
    if let Some(spec) = fields.map(str::trim).filter(|s| !s.is_empty()) {
        cratebase_core::record::project_fields(value, spec);
    }
}

// ------------------------------------------------------------- staged files

enum StagedData {
    Memory(Bytes),
    Spilled(tempfile::TempPath),
}

/// One uploaded file, parsed but not yet in the object store.
pub struct StagedUpload {
    /// The collection field it belongs to (modifier suffix stripped).
    pub field: String,
    /// The name stored in the record and in the object key.
    pub name: String,
    /// The name the client uploaded it under. PocketBase quotes *this*
    /// one in its size/mime errors, not the generated one.
    pub original: String,
    pub size: i64,
    pub mime: String,
    data: StagedData,
}

impl StagedUpload {
    pub fn meta(&self) -> UploadMeta {
        UploadMeta {
            field: self.field.clone(),
            name: self.name.clone(),
            size: self.size,
            mime: self.mime.clone(),
        }
    }

    /// Write the staged bytes to `key`. Spilled files stream off disk, so
    /// a large upload is never fully resident.
    pub async fn store(&self, storage: &Storage, key: &str) -> Result<(), ApiError> {
        match &self.data {
            StagedData::Memory(bytes) => storage.put(key, bytes.clone()).await?,
            StagedData::Spilled(path) => {
                let file = tokio::fs::File::open(path).await?;
                let stream = tokio_util::io::ReaderStream::new(file);
                storage
                    .put_stream(key, stream, Some(self.size.max(0) as u64))
                    .await?;
            }
        }
        Ok(())
    }
}

/// The object key a record file lives under.
pub fn file_key(collection_id: &str, record_id: &str, filename: &str) -> String {
    format!("{collection_id}/{record_id}/{filename}")
}

/// The prefix holding every cached thumb of one file.
pub fn thumbs_prefix(collection_id: &str, record_id: &str, filename: &str) -> String {
    format!("{collection_id}/{record_id}/thumbs_{filename}")
}

/// Upload every staged file for `record_id`, removing whatever was
/// written if one of them fails. Returns the keys written so the caller
/// can roll them back when the row write fails.
pub async fn store_uploads(
    storage: &Storage,
    collection_id: &str,
    record_id: &str,
    uploads: &[StagedUpload],
) -> Result<Vec<String>, ApiError> {
    let mut written = Vec::with_capacity(uploads.len());
    for upload in uploads {
        let key = file_key(collection_id, record_id, &upload.name);
        if let Err(e) = upload.store(storage, &key).await {
            remove_keys(storage, &written).await;
            return Err(e);
        }
        written.push(key);
    }
    Ok(written)
}

/// Best-effort deletion; a failure here is logged, never surfaced (the
/// caller is already unwinding, or the record write already succeeded).
pub async fn remove_keys(storage: &Storage, keys: &[String]) {
    for key in keys {
        if let Err(e) = storage.delete(key).await {
            tracing::warn!(key = %key, error = %e, "failed to remove a staged upload");
        }
    }
}

/// Delete a record file and every thumb cached from it.
pub async fn remove_file(storage: &Storage, collection_id: &str, record_id: &str, name: &str) {
    if name.is_empty() {
        return;
    }
    if let Err(e) = storage
        .delete(&file_key(collection_id, record_id, name))
        .await
    {
        tracing::warn!(file = %name, error = %e, "failed to remove a record file");
    }
    if let Err(e) = storage
        .delete_prefix(&thumbs_prefix(collection_id, record_id, name))
        .await
    {
        tracing::warn!(file = %name, error = %e, "failed to remove cached thumbs");
    }
}

/// File names a record holds, per file field.
pub fn record_file_names(record: &Record) -> Vec<String> {
    let mut out = Vec::new();
    for field in record.collection.fields_of_type(FieldType::File) {
        out.extend(
            record
                .get_string_list(&field.name)
                .into_iter()
                .filter(|n| !n.is_empty()),
        );
    }
    out
}

// ------------------------------------------------------------ body parsing

/// A parsed request body plus whatever files came with it.
#[derive(Default)]
pub struct ParsedBody {
    pub data: Map<String, Value>,
    pub uploads: Vec<StagedUpload>,
}

impl ParsedBody {
    pub fn upload_meta(&self) -> Vec<UploadMeta> {
        self.uploads.iter().map(StagedUpload::meta).collect()
    }
}

/// Parse a JSON body. A malformed one is PocketBase's *generic* 400 on
/// the record endpoints (KNOWN_DIVERGENCES §17), which is why this does
/// not go through [`crate::http_error::ApiJson`].
pub fn parse_json_body(bytes: &[u8]) -> ApiResult<ParsedBody> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(ParsedBody::default());
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(data)) => Ok(ParsedBody {
            data,
            uploads: Vec::new(),
        }),
        Ok(Value::Null) => Ok(ParsedBody::default()),
        Ok(_) | Err(_) => Err(ApiError(AppError::bad_request(""))),
    }
}

/// PocketBase's `+`/`-` modifiers on **number** fields
/// (`{"views+": 5}` increments, `{"views-": 3}` decrements).
///
/// [`cratebase_db::validate::apply_modifiers`] only covers the
/// multi-valued `select`/`relation`/`file` forms, so this fills the gap
/// here rather than in the write path. It runs *before* the multi-value
/// pass and consumes its own keys, so the two never see each other's
/// input.
pub fn apply_number_modifiers(
    previous: Option<&Record>,
    input: &mut Map<String, Value>,
    collection: &Collection,
) {
    for field in collection.fields_of_type(cratebase_core::FieldType::Number) {
        let increment = format!("{}+", field.name);
        let decrement = format!("{}-", field.name);
        if !input.contains_key(&increment) && !input.contains_key(&decrement) {
            continue;
        }
        // The base is the plain key when the body also carries it,
        // otherwise the stored value — the same precedence the
        // multi-value modifiers use.
        let mut value = input
            .get(&field.name)
            .map(number)
            .or_else(|| previous.map(|r| r.get_f64(&field.name)))
            .unwrap_or(0.0);
        if let Some(delta) = input.remove(&increment) {
            value += number(&delta);
        }
        if let Some(delta) = input.remove(&decrement) {
            value -= number(&delta);
        }
        input.insert(field.name.clone(), whole_or_float(value));
    }
}

/// `15.0` serializes as `15`, matching what a number column reads back as
/// — the response to a write is built from the in-memory record, so
/// without this an incremented field would come back as a float while the
/// same record fetched a moment later came back as an integer.
fn whole_or_float(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

/// Go's `cast.ToFloat64`: anything unparsable is `0`, matching how
/// PocketBase coerces a number field elsewhere.
fn number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::String(s) => s.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Parse a `multipart/form-data` body against `collection`'s schema.
///
/// * a part with a filename whose (modifier-stripped) key is a file field
///   becomes a [`StagedUpload`], and its generated name is appended to the
///   body under the original key so the `+`/`-` modifiers still apply;
/// * `@jsonPayload` is parsed and merged;
/// * every other part is a text value, with repeated keys collected into
///   an array (PocketBase's behaviour, pinned by `records.test.ts`).
pub async fn parse_multipart(
    collection: &Arc<Collection>,
    mut multipart: Multipart,
) -> ApiResult<ParsedBody> {
    let mut out = ParsedBody::default();
    let mut json_payload: Option<Map<String, Value>> = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(_) => return Err(ApiError(AppError::bad_request(""))),
        };
        let key = field.name().unwrap_or_default().to_string();
        if key.is_empty() {
            continue;
        }
        let filename = field.file_name().map(str::to_string);
        let content_type = field.content_type().map(str::to_string);

        if key == JSON_PAYLOAD {
            let text = field
                .text()
                .await
                .map_err(|_| ApiError(AppError::bad_request("")))?;
            match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(map)) => json_payload = Some(map),
                _ => return Err(ApiError(AppError::bad_request(""))),
            }
            continue;
        }

        let base = strip_modifier(&key);
        let is_file_field = collection
            .field(base)
            .is_some_and(|f| f.field_type() == FieldType::File);

        match (filename, is_file_field) {
            (Some(original), true) => {
                let staged = stage_field(field, base, &original, content_type).await?;
                push_value(&mut out.data, &key, Value::String(staged.name.clone()));
                out.uploads.push(staged);
            }
            _ => {
                let text = field
                    .text()
                    .await
                    .map_err(|_| ApiError(AppError::bad_request("")))?;
                push_value(&mut out.data, &key, Value::String(text));
            }
        }
    }

    // The JSON payload is the base; explicit parts (files included) win,
    // which is what makes `@jsonPayload` + a `cover` part work.
    if let Some(payload) = json_payload {
        for (k, v) in payload {
            out.data.entry(k).or_insert(v);
        }
    }
    Ok(out)
}

/// Read one multipart file part, spilling to disk past
/// [`SPILL_THRESHOLD`].
pub(crate) async fn stage_field(
    mut field: axum::extract::multipart::Field<'_>,
    base_field: &str,
    original_name: &str,
    content_type: Option<String>,
) -> ApiResult<StagedUpload> {
    use tokio::io::AsyncWriteExt;

    let mut buffered = BytesMut::new();
    let mut spilled: Option<(tokio::fs::File, tempfile::TempPath)> = None;
    let mut size: i64 = 0;

    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|_| ApiError(AppError::bad_request("")))?
    {
        size += chunk.len() as i64;
        match &mut spilled {
            Some((file, _)) => file.write_all(&chunk).await?,
            None => {
                buffered.extend_from_slice(&chunk);
                if buffered.len() > SPILL_THRESHOLD {
                    let temp = tempfile::NamedTempFile::new()?.into_temp_path();
                    let mut file = tokio::fs::File::create(&temp).await?;
                    file.write_all(&buffered).await?;
                    buffered.clear();
                    spilled = Some((file, temp));
                }
            }
        }
    }

    let data = match spilled {
        Some((mut file, temp)) => {
            file.flush().await?;
            drop(file);
            StagedData::Spilled(temp)
        }
        None => StagedData::Memory(buffered.freeze()),
    };

    Ok(StagedUpload {
        field: base_field.to_string(),
        name: stored_file_name(original_name),
        original: original_name.to_string(),
        size,
        mime: mime_of(content_type.as_deref(), original_name),
        data,
    })
}

/// The mime type a `mimeTypes` constraint is checked against.
///
/// The declared part header wins, but only its *essence*: browsers and
/// `fetch` send `text/plain;charset=utf-8`, and comparing that against a
/// configured `text/plain` would reject every text upload. A missing or
/// deliberately vague declaration falls back to the extension, which is
/// what PocketBase lands on for files its content sniffer cannot place.
fn mime_of(declared: Option<&str>, filename: &str) -> String {
    let essence = declared
        .and_then(|v| v.split(';').next())
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty() && v != "application/octet-stream");
    essence.unwrap_or_else(|| {
        mime_guess::from_path(filename)
            .first_or_octet_stream()
            .essence_str()
            .to_string()
    })
}

/// Rewrite the generated file names in a validation error back into the
/// names the client used. `crates/db` only knows the stored name, but
/// PocketBase's message quotes the uploaded one.
pub fn relabel_upload_errors(error: ApiError, uploads: &[StagedUpload]) -> ApiError {
    let AppError::Validation { message, fields } = error.error else {
        return error;
    };
    let fields = fields
        .into_iter()
        .map(|(key, mut field_error)| {
            for upload in uploads {
                if field_error.message.contains(&upload.name) {
                    field_error.message =
                        field_error.message.replace(&upload.name, &upload.original);
                }
            }
            (key, field_error)
        })
        .collect::<Vec<_>>();
    ApiError {
        error: AppError::validation(message, fields),
        data: error.data,
    }
}

/// Collect repeated multipart keys into an array, as PocketBase does.
fn push_value(map: &mut Map<String, Value>, key: &str, value: Value) {
    match map.get_mut(key) {
        Some(Value::Array(items)) => items.push(value),
        Some(existing) => {
            let previous = std::mem::replace(existing, Value::Null);
            *existing = Value::Array(vec![previous, value]);
        }
        None => {
            map.insert(key.to_string(), value);
        }
    }
}

/// `tags+` / `+tags` / `tags-` → `tags`.
pub fn strip_modifier(key: &str) -> &str {
    if let Some(rest) = key.strip_prefix('+') {
        return rest;
    }
    if let Some(rest) = key.strip_suffix('+') {
        return rest;
    }
    key.strip_suffix('-').unwrap_or(key)
}

/// PocketBase's stored file name: the sanitized base name (padded when it
/// is very short), a 10-character random suffix and the original
/// extension — `notes.txt` → `notes_a1b2c3d4e5.txt`.
pub fn stored_file_name(original: &str) -> String {
    let original = original.rsplit(['/', '\\']).next().unwrap_or(original);
    let (base, ext) = match original.rfind('.') {
        Some(i) if i > 0 && original.len() - i <= 21 => (&original[..i], &original[i..]),
        _ => (original, ""),
    };
    let mut base: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    while base.chars().count() < MIN_BASE_LEN {
        base.push_str(&cratebase_core::ids::random_string(1, NAME_ALPHABET));
    }
    let ext: String = ext
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.')
        .collect();
    format!(
        "{base}_{}{ext}",
        cratebase_core::ids::random_string(SUFFIX_LEN, NAME_ALPHABET)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_names_follow_pocketbase() {
        let name = stored_file_name("notes.txt");
        assert!(regex_like(&name, "notes_", ".txt", SUFFIX_LEN), "{name}");
        // A one-character base name is padded, so the result never reads
        // as just the random suffix.
        let short = stored_file_name("a.txt");
        assert!(short.starts_with('a'), "{short}");
        assert!(short.ends_with(".txt"));
        let base = short.trim_end_matches(".txt");
        let (head, suffix) = base.rsplit_once('_').unwrap();
        assert!(head.len() >= MIN_BASE_LEN, "{short}");
        assert_eq!(suffix.len(), SUFFIX_LEN);
        assert!(suffix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
        // No extension at all.
        assert!(stored_file_name("README").contains('_'));
        // Path traversal cannot survive.
        let escaped = stored_file_name("../../etc/passwd");
        assert!(!escaped.contains('/'), "{escaped}");
    }

    fn regex_like(value: &str, prefix: &str, suffix: &str, random: usize) -> bool {
        value.starts_with(prefix)
            && value.ends_with(suffix)
            && value.len() == prefix.len() + random + suffix.len()
    }

    #[test]
    fn modifier_keys_resolve_to_their_field() {
        assert_eq!(strip_modifier("tags"), "tags");
        assert_eq!(strip_modifier("tags+"), "tags");
        assert_eq!(strip_modifier("+tags"), "tags");
        assert_eq!(strip_modifier("tags-"), "tags");
    }

    #[test]
    fn repeated_multipart_keys_become_arrays() {
        let mut map = Map::new();
        push_value(&mut map, "tags", Value::String("go".into()));
        assert_eq!(map["tags"], Value::String("go".into()));
        push_value(&mut map, "tags", Value::String("rust".into()));
        assert_eq!(map["tags"], serde_json::json!(["go", "rust"]));
        push_value(&mut map, "tags", Value::String("js".into()));
        assert_eq!(map["tags"], serde_json::json!(["go", "rust", "js"]));
    }

    #[test]
    fn malformed_json_is_the_generic_400() {
        let Err(err) = parse_json_body(b"{not json") else {
            panic!("malformed JSON must be rejected");
        };
        assert_eq!(err.error.status(), 400);
        assert_eq!(err.error.body().message, AppError::DEFAULT_BAD_REQUEST);
        assert!(parse_json_body(b"").unwrap().data.is_empty());
        assert!(parse_json_body(b"  ").unwrap().data.is_empty());
    }
}
