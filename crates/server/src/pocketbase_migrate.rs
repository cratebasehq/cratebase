//! `cratebase migrate-from-pocketbase <pb_data_dir>` — a one-shot import of
//! an existing PocketBase installation's schema, records and files into
//! this Cratebase instance.
//!
//! # Why this works with almost no translation
//!
//! Collection/field JSON is already byte-for-byte the same shape PocketBase
//! v0.23+ uses (see `cratebase_core::collection`/`field` module docs), and
//! [`cratebase_core::ids::collection_id`]/[`cratebase_core::ids::field_id`]
//! reimplement PocketBase's own `crc32(type + name)` id derivation. So a
//! Cratebase collection created with the same `name`/`type` as a PocketBase
//! one, and no explicit `id`, gets the *exact same id* PocketBase assigned —
//! which means a `relation` field's `collectionId`, copied verbatim from the
//! PocketBase dump, already points at the right place without any id
//! remapping table. The only genuine translation needed is:
//!
//! * dropping PocketBase's `system` fields (`id`, `password`, `tokenKey`,
//!   `email`, `emailVisibility`, `verified`) from the payload — Cratebase's
//!   own collection-creation path (`ensure_system_fields`) regenerates them
//!   identically, so submitting them is redundant, not required;
//! * flattening the auth-specific config PocketBase stores in
//!   `_collections.options` onto the collection JSON's top level, PB's own
//!   v0.23+ shape (`authRule`, `passwordAuth`, `mfa`, `otp`, ...), while
//!   deliberately *not* copying the per-instance token-signing secrets
//!   (a fresh instance must never reuse another instance's JWT secret) or
//!   OAuth2 provider client secrets (PocketBase never round-trips these
//!   over its own JSON API either, so they are not in `options` to begin
//!   with in a dashboard-driven setup, and even if present they would be
//!   invalid for a different app anyway);
//! * decoding each column's raw SQLite value against the field's `kind`
//!   (SQLite has no schema enforcement on `TEXT`, so a `select`/`file`/
//!   `relation` column is a bare string when `maxSelect<=1` and a JSON
//!   array otherwise — verified against a real `pocketbase serve` database
//!   dump, not assumed).
//!
//! # What is intentionally NOT migrated
//!
//! * PocketBase's `_mfas`/`_otps`/`_externalAuths`/`_authOrigins` rows —
//!   ephemeral second-factor/session state that is meaningless without the
//!   exact signing secrets behind it (which this tool does not copy; see
//!   above). Users re-establish MFA/OAuth2 links after migrating.
//! * Session tokens already issued by the PocketBase instance — they are
//!   signed with a secret this tool never reads out of `pb_data`, so they
//!   are simply invalid against Cratebase. Existing users must log in again
//!   (their *password* still works — see `docs/migrating-from-pocketbase.md`
//!   for the verified bcrypt compatibility).
//! * PocketBase JS migration history / hooks (`pb_hooks`, `pb_migrations`) —
//!   out of scope; this tool moves data, not server-side application code.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cratebase_core::{Collection, Field, FieldKind};
use cratebase_db::{Executor, Sql, UploadMeta};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::extract::RequestInfo;
use crate::routes::common::file_key;
use crate::routes::schema::plan_and_apply;

/// Field types this tool knows how to carry across. Kept as an explicit
/// allow-list (rather than "anything PocketBase sends") so a PocketBase
/// version newer than the one this was built against fails loudly on an
/// unrecognised field type instead of silently dropping the column.
const KNOWN_FIELD_TYPES: &[&str] = &[
    "text", "editor", "number", "bool", "email", "url", "date", "autodate", "select", "file",
    "relation", "json", "password", "geoPoint",
];

/// One field this tool refused to carry across, and why.
#[derive(Debug, Clone)]
pub struct UnsupportedField {
    pub collection: String,
    pub field: String,
    pub field_type: String,
}

/// One file that failed to copy.
#[derive(Debug, Clone)]
pub struct FailedFile {
    pub collection: String,
    pub record_id: String,
    pub filename: String,
    pub error: String,
}

#[derive(Debug, Default)]
pub struct MigrationReport {
    pub collections_created: Vec<String>,
    pub collections_updated: Vec<String>,
    pub collections_skipped_system: Vec<String>,
    pub unsupported_fields: Vec<UnsupportedField>,
    pub oauth2_needs_reconfiguration: Vec<String>,
    /// Collection name -> number of records copied.
    pub records_migrated: BTreeMap<String, usize>,
    pub files_copied: usize,
    pub failed_files: Vec<FailedFile>,
}

/// A collection row read straight out of PocketBase's `_collections` table.
struct PbCollection {
    id: String,
    name: String,
    collection_type: String,
    system: bool,
    fields: Vec<Value>,
    list_rule: Option<String>,
    view_rule: Option<String>,
    create_rule: Option<String>,
    update_rule: Option<String>,
    delete_rule: Option<String>,
    options: Value,
}

/// Run the whole migration: schema, then records and files.
///
/// `pb_dir` is PocketBase's data directory (what `pocketbase serve --dir`
/// points at): it must contain `data.db` and, if any collection has file
/// fields, a `storage/` subdirectory.
pub async fn run(app: &App, pb_dir: &str) -> anyhow::Result<MigrationReport> {
    let pb_dir = PathBuf::from(pb_dir);
    let db_path = pb_dir.join("data.db");
    if !db_path.exists() {
        anyhow::bail!(
            "{} not found — expected a PocketBase data directory (the one \
             `pocketbase serve --dir` was pointed at), containing data.db",
            db_path.display()
        );
    }
    let storage_dir = pb_dir.join("storage");

    let db_path_for_read = db_path.clone();
    let pb_collections: Vec<PbCollection> =
        tokio::task::spawn_blocking(move || read_pb_collections(&db_path_for_read)).await??;

    let mut report = MigrationReport::default();

    // ---------------------------------------------------------------
    // Phase 1: schema. PocketBase's `_mfas`/`_otps`/`_externalAuths`/
    // `_authOrigins` are skipped outright (see module docs); `_superusers`
    // and `users` already exist with an identical schema (same
    // deterministic id, same default fields) so only records need to move
    // for them — no schema payload is built.
    // ---------------------------------------------------------------
    let mut payload_items = Vec::new();
    for pbc in &pb_collections {
        if pbc.system {
            if pbc.name != "_superusers" {
                report.collections_skipped_system.push(pbc.name.clone());
            }
            continue;
        }
        let (translated, oauth2_flag) = translate_collection(pbc, &mut report.unsupported_fields);
        if oauth2_flag {
            report.oauth2_needs_reconfiguration.push(pbc.name.clone());
        }
        payload_items.push(translated);
    }

    if !payload_items.is_empty() {
        let body = json!({ "collections": payload_items });
        let info = RequestInfo::default();
        let diff = plan_and_apply(app, &body, false, false, &info, None)
            .await
            .map_err(|e| anyhow::anyhow!("schema migration failed: {}", e.error))?;
        for c in &diff.collections {
            match c.action {
                "create" => report.collections_created.push(c.name.clone()),
                "update" => report.collections_updated.push(c.name.clone()),
                _ => {}
            }
        }
    }

    // ---------------------------------------------------------------
    // Phase 2: records + files, collection by collection.
    // ---------------------------------------------------------------
    for pbc in &pb_collections {
        if pbc.system && pbc.name != "_superusers" {
            continue;
        }
        let target = app.db().collections.get_by_name(&pbc.name).ok_or_else(|| {
            anyhow::anyhow!("collection {} missing after schema migration", pbc.name)
        })?;
        let db_path = db_path.clone();
        let pbc_name = pbc.name.clone();
        let rows = tokio::task::spawn_blocking(move || read_pb_rows(&db_path, &pbc_name)).await??;

        let mut count = 0usize;
        for row in rows {
            migrate_record(app, &target, &pbc.id, &storage_dir, row, &mut report).await?;
            count += 1;
        }
        report.records_migrated.insert(pbc.name.clone(), count);
    }

    Ok(report)
}

// --------------------------------------------------------------------
// Reading PocketBase's SQLite file directly (never through a running
// PocketBase server — this tool only needs `pb_data`, not a live pb
// process).
// --------------------------------------------------------------------

fn open_pb_db(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

fn read_pb_collections(db_path: &Path) -> anyhow::Result<Vec<PbCollection>> {
    let conn = open_pb_db(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT id, name, type, system, fields, listRule, viewRule, createRule, updateRule, \
         deleteRule, options FROM _collections ORDER BY rowid ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        let fields_raw: String = r.get(4)?;
        let options_raw: String = r.get(10)?;
        Ok(PbCollection {
            id: r.get(0)?,
            name: r.get(1)?,
            collection_type: r.get(2)?,
            system: r.get::<_, i64>(3)? != 0,
            fields: serde_json::from_str::<Vec<Value>>(&fields_raw).unwrap_or_default(),
            list_rule: r.get(5)?,
            view_rule: r.get(6)?,
            create_rule: r.get(7)?,
            update_rule: r.get(8)?,
            delete_rule: r.get(9)?,
            options: serde_json::from_str::<Value>(&options_raw).unwrap_or(Value::Null),
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// One PocketBase row, as raw SQLite values keyed by column name.
type PbRow = Vec<(String, SqlValue)>;

fn read_pb_rows(db_path: &Path, table: &str) -> anyhow::Result<Vec<PbRow>> {
    let conn = open_pb_db(db_path)?;
    // `table` comes from `_collections.name`, validated on the way in by
    // PocketBase's own collection-name rules (ASCII identifier); not
    // user-supplied at this call site, but quoted defensively all the same.
    let quoted = format!("\"{}\"", table.replace('"', "\"\""));
    let sql = format!("SELECT * FROM {quoted}");
    let mut stmt = conn.prepare(&sql)?;
    let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let n = col_names.len();
    let rows = stmt.query_map([], |r| {
        let mut row = Vec::with_capacity(n);
        for (i, name) in col_names.iter().enumerate() {
            row.push((name.clone(), r.get::<_, SqlValue>(i)?));
        }
        Ok(row)
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

// --------------------------------------------------------------------
// Schema translation.
// --------------------------------------------------------------------

/// Build the Cratebase collection JSON `plan_and_apply` expects, and
/// whether this collection has OAuth2 provider config an operator needs to
/// re-enter (this tool never carries provider client secrets across).
fn translate_collection(
    pbc: &PbCollection,
    unsupported: &mut Vec<UnsupportedField>,
) -> (Value, bool) {
    let mut fields = Vec::new();
    for f in &pbc.fields {
        let is_system = f.get("system").and_then(Value::as_bool).unwrap_or(false);
        if is_system {
            // Cratebase's `ensure_system_fields` regenerates these
            // identically from the collection type; resubmitting them is
            // both unnecessary and (for id/password/tokenKey) would be
            // rejected as an attempt to redefine a reserved field.
            continue;
        }
        let field_type = f.get("type").and_then(Value::as_str).unwrap_or("");
        if !KNOWN_FIELD_TYPES.contains(&field_type) {
            unsupported.push(UnsupportedField {
                collection: pbc.name.clone(),
                field: f
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                field_type: field_type.to_string(),
            });
            continue;
        }
        fields.push(f.clone());
    }

    let mut doc = Map::new();
    doc.insert("name".into(), json!(pbc.name));
    doc.insert("type".into(), json!(pbc.collection_type));
    doc.insert("listRule".into(), rule_json(&pbc.list_rule));
    doc.insert("viewRule".into(), rule_json(&pbc.view_rule));
    doc.insert("createRule".into(), rule_json(&pbc.create_rule));
    doc.insert("updateRule".into(), rule_json(&pbc.update_rule));
    doc.insert("deleteRule".into(), rule_json(&pbc.delete_rule));
    doc.insert("fields".into(), Value::Array(fields));

    let mut oauth2_flag = false;
    if pbc.collection_type == "auth" {
        if let Value::Object(opts) = &pbc.options {
            // Non-secret auth config: carried over verbatim, same
            // camelCase shape PocketBase and Cratebase both use.
            for key in [
                "authRule",
                "manageRule",
                "passwordAuth",
                "mfa",
                "otp",
                "authAlert",
            ] {
                if let Some(v) = opts.get(key) {
                    doc.insert(key.to_string(), v.clone());
                }
            }
            // OAuth2: never copy provider client secrets across install
            // boundaries. If the source had any provider configured,
            // ship `oauth2` with providers cleared and flag it — the
            // operator re-adds providers with fresh credentials.
            if let Some(oauth2) = opts.get("oauth2") {
                let had_providers = oauth2
                    .get("providers")
                    .and_then(Value::as_array)
                    .is_some_and(|p| !p.is_empty());
                if had_providers {
                    oauth2_flag = true;
                }
                doc.insert(
                    "oauth2".into(),
                    json!({
                        "enabled": false,
                        "mappedFields": oauth2.get("mappedFields").cloned().unwrap_or(json!({})),
                        "providers": [],
                    }),
                );
            }
            // Token secrets/durations and email templates that embed
            // per-instance secrets are deliberately not copied; Cratebase
            // keeps the secrets it generated at bootstrap.
        }
    } else if pbc.collection_type == "view" {
        if let Some(query) = pbc.options.get("query").and_then(Value::as_str) {
            doc.insert("viewQuery".into(), json!(query));
        }
    }

    (Value::Object(doc), oauth2_flag)
}

fn rule_json(rule: &Option<String>) -> Value {
    match rule {
        Some(s) => json!(s),
        None => Value::Null,
    }
}

// --------------------------------------------------------------------
// Record + file migration.
// --------------------------------------------------------------------

async fn migrate_record(
    app: &App,
    target: &std::sync::Arc<Collection>,
    pb_collection_id: &str,
    storage_dir: &Path,
    row: PbRow,
    report: &mut MigrationReport,
) -> anyhow::Result<()> {
    let mut record = cratebase_core::Record::new(target.clone());
    let mut created_raw: Option<String> = None;
    let mut updated_raw: Option<String> = None;

    for (col, raw) in &row {
        if col == "created" {
            if let SqlValue::Text(s) = raw {
                created_raw = Some(s.clone());
            }
        }
        if col == "updated" {
            if let SqlValue::Text(s) = raw {
                updated_raw = Some(s.clone());
            }
        }
        let Some(field) = target.field(col) else {
            continue;
        };
        record.set(col, decode_value(field, raw.clone()));
    }

    let record_id = record.id().to_string();
    if record_id.is_empty() {
        anyhow::bail!("row in {} has no id", target.name);
    }

    // `_superusers.role` (owner/admin) is a Cratebase-only addition with
    // no PocketBase equivalent — PocketBase has exactly one superuser
    // tier. A migrated superuser gets the same "owner" (full access)
    // role Cratebase's own upgrade migration backfills onto pre-existing
    // rows, so migrating in does not downgrade anyone's access.
    if target.name == cratebase_core::SUPERUSERS_COLLECTION
        && target.has_field("role")
        && record
            .get("role")
            .map(cratebase_db::validate::is_blank)
            .unwrap_or(true)
    {
        record.set(
            "role",
            Value::String(cratebase_core::SUPERUSER_ROLE_OWNER.into()),
        );
    }

    // Gather file fields: filenames to copy from PocketBase's storage
    // layout (`<collectionId>/<recordId>/<filename>`, identical convention
    // Cratebase uses) plus the `UploadMeta` the record write validates
    // maxSize/mimeTypes against.
    let mut uploads = Vec::new();
    let mut files_to_copy: Vec<(String, String)> = Vec::new(); // (field, filename)
    for field in target.fields_of_type(cratebase_core::FieldType::File) {
        let Some(value) = record.get(&field.name) else {
            continue;
        };
        let filenames: Vec<String> = if field.is_multiple() {
            value
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            value
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| vec![s.to_string()])
                .unwrap_or_default()
        };
        for filename in filenames {
            let src = storage_dir
                .join(pb_collection_id)
                .join(&record_id)
                .join(&filename);
            match std::fs::metadata(&src) {
                Ok(meta) => {
                    let mime = mime_guess::from_path(&filename)
                        .first_or_octet_stream()
                        .essence_str()
                        .to_string();
                    uploads.push(UploadMeta {
                        field: field.name.clone(),
                        name: filename.clone(),
                        size: meta.len() as i64,
                        mime,
                    });
                    files_to_copy.push((field.name.clone(), filename));
                }
                Err(e) => {
                    report.failed_files.push(FailedFile {
                        collection: target.name.clone(),
                        record_id: record_id.clone(),
                        filename: filename.clone(),
                        error: format!("source file missing on disk: {e}"),
                    });
                }
            }
        }
    }

    cratebase_db::records::create_with_uploads(
        app.db(),
        &app.db().collections,
        &mut record,
        &uploads,
    )
    .await
    .map_err(|e| {
        anyhow::anyhow!(
            "failed to create record {}/{}: {:?}",
            target.name,
            record_id,
            e
        )
    })?;

    // `records::create` stamps `created`/`updated` autodate fields with
    // "now"; overwrite them back to PocketBase's original values so
    // migrated history (and any `sort=created` query) survives intact.
    if created_raw.is_some() || updated_raw.is_some() {
        let mut sets = Vec::new();
        let mut params = Vec::new();
        if let Some(c) = &created_raw {
            sets.push("\"created\" = ?");
            params.push(Sql::Text(c.clone()));
        }
        if let Some(u) = &updated_raw {
            sets.push("\"updated\" = ?");
            params.push(Sql::Text(u.clone()));
        }
        if !sets.is_empty() {
            params.push(Sql::Text(record_id.clone()));
            let sql = format!(
                "UPDATE \"{}\" SET {} WHERE \"id\" = ?",
                target.table_name(),
                sets.join(", ")
            );
            app.db().execute(&sql, &params).await?;
        }
    }

    for (field, filename) in files_to_copy {
        let src = storage_dir
            .join(pb_collection_id)
            .join(&record_id)
            .join(&filename);
        match std::fs::read(&src) {
            Ok(bytes) => {
                let key = file_key(&target.id, &record_id, &filename);
                if let Err(e) = app.storage().put(&key, bytes.into()).await {
                    report.failed_files.push(FailedFile {
                        collection: target.name.clone(),
                        record_id: record_id.clone(),
                        filename: filename.clone(),
                        error: format!("storage write failed: {e}"),
                    });
                } else {
                    report.files_copied += 1;
                }
            }
            Err(e) => {
                report.failed_files.push(FailedFile {
                    collection: target.name.clone(),
                    record_id: record_id.clone(),
                    filename,
                    error: format!("read failed: {e} (field {field})"),
                });
            }
        }
    }

    Ok(())
}

/// Decode one raw SQLite column against the field's declared kind.
/// PocketBase's SQLite columns are untyped `TEXT`/`NUMERIC`/`BOOLEAN`
/// affinities: a `select`/`file`/`relation` field is a bare string when
/// `maxSelect<=1` and a JSON array TEXT otherwise (verified against a real
/// `pocketbase serve` database, not assumed); `json`/`geoPoint`/`vector`
/// are always JSON TEXT; `bool` is stored as SQLite's 0/1 integer.
fn decode_value(field: &Field, raw: SqlValue) -> Value {
    match &field.kind {
        FieldKind::Bool {} => match raw {
            SqlValue::Integer(n) => json!(n != 0),
            SqlValue::Text(s) => json!(s == "1" || s.eq_ignore_ascii_case("true")),
            _ => json!(false),
        },
        FieldKind::Number { .. } => match raw {
            SqlValue::Integer(n) => json!(n),
            SqlValue::Real(f) => json!(f),
            SqlValue::Text(s) => serde_json::from_str(&s).unwrap_or(json!(0)),
            _ => json!(0),
        },
        FieldKind::Json { .. } | FieldKind::GeoPoint {} | FieldKind::Vector { .. } => match raw {
            SqlValue::Text(s) if !s.is_empty() => serde_json::from_str(&s).unwrap_or(Value::Null),
            _ => Value::Null,
        },
        FieldKind::Select { .. } | FieldKind::File { .. } | FieldKind::Relation { .. } => {
            if field.is_multiple() {
                match raw {
                    SqlValue::Text(s) if !s.is_empty() => {
                        serde_json::from_str(&s).unwrap_or(json!([]))
                    }
                    _ => json!([]),
                }
            } else {
                match raw {
                    SqlValue::Text(s) => json!(s),
                    _ => json!(""),
                }
            }
        }
        // text, editor, email, url, date, autodate, password: all plain
        // TEXT columns.
        _ => match raw {
            SqlValue::Text(s) => json!(s),
            SqlValue::Integer(n) => json!(n.to_string()),
            SqlValue::Real(f) => json!(f.to_string()),
            _ => json!(""),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pbc(collection_type: &str, fields: Value, options: Value) -> PbCollection {
        PbCollection {
            id: "pbc_test".into(),
            name: "widgets".into(),
            collection_type: collection_type.into(),
            system: false,
            fields: fields.as_array().cloned().unwrap_or_default(),
            list_rule: Some(String::new()),
            view_rule: Some(String::new()),
            create_rule: Some(String::new()),
            update_rule: Some(String::new()),
            delete_rule: Some(String::new()),
            options,
        }
    }

    #[test]
    fn strips_system_fields_and_keeps_custom_ones() {
        let c = pbc(
            "base",
            json!([
                { "name": "id", "type": "text", "system": true },
                { "name": "title", "type": "text", "system": false },
            ]),
            Value::Null,
        );
        let mut unsupported = Vec::new();
        let (doc, oauth2_flag) = translate_collection(&c, &mut unsupported);
        assert!(!oauth2_flag);
        assert!(unsupported.is_empty());
        let names: Vec<&str> = doc["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["title"]);
    }

    #[test]
    fn flags_unknown_field_types_as_unsupported_instead_of_dropping_silently() {
        let c = pbc(
            "base",
            json!([
                { "name": "title", "type": "text", "system": false },
                { "name": "mystery", "type": "some_future_type", "system": false },
            ]),
            Value::Null,
        );
        let mut unsupported = Vec::new();
        let (doc, _) = translate_collection(&c, &mut unsupported);
        assert_eq!(unsupported.len(), 1);
        assert_eq!(unsupported[0].field, "mystery");
        assert_eq!(unsupported[0].field_type, "some_future_type");
        let names: Vec<&str> = doc["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["title"]);
    }

    #[test]
    fn auth_collection_carries_non_secret_config_and_flags_oauth2_providers() {
        let c = pbc(
            "auth",
            json!([]),
            json!({
                "authRule": "",
                "passwordAuth": { "enabled": true, "identityFields": ["email"] },
                "oauth2": { "enabled": true, "providers": [{ "name": "google", "clientSecret": "sekrit" }] },
                "authToken": { "secret": "must-not-be-copied", "duration": 432000 },
            }),
        );
        let mut unsupported = Vec::new();
        let (doc, oauth2_flag) = translate_collection(&c, &mut unsupported);
        assert!(oauth2_flag, "provider present -> operator must reconfigure");
        assert_eq!(doc["passwordAuth"]["enabled"], json!(true));
        assert_eq!(doc["oauth2"]["enabled"], json!(false));
        assert_eq!(doc["oauth2"]["providers"], json!([]));
        assert!(
            doc.get("authToken").is_none(),
            "token secrets must never be copied across instances"
        );
    }

    #[test]
    fn relation_and_select_decode_by_max_select() {
        use cratebase_core::{Field, FieldKind};

        let single = Field::new(
            "category",
            FieldKind::Relation {
                collection_id: "pbc_x".into(),
                cascade_delete: false,
                min_select: 0,
                max_select: 1,
            },
        );
        assert_eq!(
            decode_value(&single, SqlValue::Text("abc123".into())),
            json!("abc123")
        );

        let multi = Field::new(
            "tags",
            FieldKind::Select {
                values: vec!["a".into(), "b".into()],
                max_select: 3,
            },
        );
        assert_eq!(
            decode_value(&multi, SqlValue::Text(r#"["a","b"]"#.into())),
            json!(["a", "b"])
        );
    }
}
