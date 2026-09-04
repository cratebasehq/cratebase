//! The record read/write API: everything above this line is HTTP, hooks
//! and events (the server layer); everything below it is SQL.
//!
//! Deliberate boundaries, so the server layer stays in charge of the
//! things it must own:
//!
//! * **No events, no hooks, no storage.** `create`/`update`/`delete`
//!   touch the database and nothing else. `delete` *reports* the file
//!   keys it orphaned; removing them from storage is the caller's job.
//! * **Rules in, HTTP status out.** `list`/`find_by_id` apply the
//!   collection's `listRule`/`viewRule`; a record the rule hides is
//!   absent (a denied `view` is [`DbError::NotFound`], never a 403, so
//!   the API cannot be used to probe for hidden ids). A rule of `None`
//!   ("superusers only") yields an empty page here — PocketBase answers
//!   that with `403` before querying, which is the server layer's call
//!   (see [`crate::rules::evaluate`]).
//! * **Serialization is the caller's.** Records come back as
//!   [`Record`]s; `?fields=` projection happens at the JSON boundary via
//!   [`cratebase_core::record::project_fields`].

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use cratebase_core::{
    ids, now, Collection, Field, FieldKind, FieldType, Record, RESERVED_FIELD_NAMES,
};
use serde_json::{Map, Value};

use crate::collections::CollectionStore;
use crate::context::{CollectionResolver, RequestContext};
use crate::engine::{quote_ident, Executor, Row, Sql};
use crate::error::{DbError, DbResult};
use crate::query::{self, in_placeholders, Query};
use crate::rules;
use crate::validate::{self, UploadMeta};
use crate::{expand, schema};

/// PocketBase's default and maximum page sizes. The cap is 1000, as
/// measured against v0.40.2 (`tests/conformance/records.test.ts`).
pub const DEFAULT_PER_PAGE: i64 = 30;
pub const MAX_PER_PAGE: i64 = 1000;

/// A file that belonged to a deleted record. The caller removes it from
/// storage; `crates/db` has no storage handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    pub collection_id: String,
    pub record_id: String,
    pub filename: String,
}

#[derive(Debug, Clone, Default)]
pub struct ListParams<'a> {
    /// 1-based; clamped to at least 1.
    pub page: i64,
    /// Clamped to `1..=1000`. The caller supplies PocketBase's default
    /// of 30 when the request omits `perPage`.
    pub per_page: i64,
    pub sort: Option<&'a str>,
    pub filter: Option<&'a str>,
    pub expand: Option<&'a str>,
    /// Skip the `COUNT(*)`; `totalItems`/`totalPages` come back as `-1`,
    /// exactly as PocketBase's `?skipTotal=1`.
    pub skip_total: bool,
}

#[derive(Debug)]
pub struct ListResult {
    pub items: Vec<Record>,
    pub page: i64,
    pub per_page: i64,
    pub total_items: i64,
    pub total_pages: i64,
}

// --- column <-> JSON ------------------------------------------------------

/// Whether a value is already a password hash rather than a plaintext to
/// be hashed: Argon2 (ours) or bcrypt (imported from PocketBase).
pub fn is_hash(value: &Value) -> bool {
    matches!(value.as_str(), Some(s) if s.starts_with("$argon2") || s.starts_with("$2"))
}

/// Whether a field's stored column holds JSON text.
fn stores_json(field: &Field) -> bool {
    schema::is_multiple(field)
        || matches!(
            field.field_type(),
            FieldType::Json | FieldType::GeoPoint | FieldType::Vector
        )
}

/// Encode a JSON value for its physical column. Multi-valued fields and
/// `json`/`geoPoint` become JSON text; numbers are `REAL`; bools are
/// `0`/`1`; everything else is text, with `null` normalized to `''` so a
/// column never mixes `NULL` and `''` for the same "empty".
pub fn column_value(field: &Field, value: &Value) -> Sql {
    if schema::is_multiple(field) {
        let items: Vec<Value> = match value {
            Value::Array(a) => a.clone(),
            Value::Null => vec![],
            Value::String(s) if s.is_empty() => vec![],
            Value::String(s) => match serde_json::from_str::<Value>(s) {
                Ok(Value::Array(a)) => a,
                _ => vec![Value::String(s.clone())],
            },
            other => vec![other.clone()],
        };
        return Sql::Text(Value::Array(items).to_string());
    }
    match field.field_type() {
        FieldType::Number => Sql::Real(value.as_f64().unwrap_or(0.0)),
        FieldType::Bool => Sql::Int(i64::from(value.as_bool().unwrap_or(false))),
        FieldType::Json | FieldType::GeoPoint | FieldType::Vector => match value {
            Value::Null => Sql::Null,
            // A json/vector field submitted as text keeps its own encoding.
            Value::String(s) => Sql::Text(s.clone()),
            other => Sql::Text(other.to_string()),
        },
        _ => match value {
            Value::Null => Sql::Text(String::new()),
            Value::String(s) => Sql::Text(s.clone()),
            other => Sql::Text(other.to_string()),
        },
    }
}

/// Decode a column back into the JSON shape the API emits.
pub fn decode_column(field: &Field, value: Option<&Sql>) -> Value {
    let Some(value) = value else {
        return cratebase_core::record::zero_value(field.field_type(), field.is_multiple());
    };
    if stores_json(field) {
        let raw = match value {
            Sql::Null => None,
            other => Some(other.clone().into_string()),
        };
        let parsed = raw
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .and_then(|s| serde_json::from_str::<Value>(s).ok());
        return match parsed {
            Some(v) if schema::is_multiple(field) && !v.is_array() => Value::Array(vec![v]),
            Some(v) => v,
            // Not valid JSON: hand back the raw text rather than losing
            // it (a hand-written view query can produce anything).
            None => match raw {
                Some(s) if !s.is_empty() => {
                    if schema::is_multiple(field) {
                        Value::Array(vec![Value::String(s)])
                    } else {
                        Value::String(s)
                    }
                }
                _ => cratebase_core::record::zero_value(field.field_type(), field.is_multiple()),
            },
        };
    }
    match field.field_type() {
        FieldType::Number => match value {
            Sql::Null => Value::Number(0.into()),
            other => number_value(other.as_f64().unwrap_or(0.0)),
        },
        FieldType::Bool => Value::Bool(value.as_i64().unwrap_or(0) != 0),
        _ => match value {
            Sql::Null => Value::String(String::new()),
            other => Value::String(other.clone().into_string()),
        },
    }
}

/// `2.0` serializes as `2`, matching PocketBase's JSON output for whole
/// numbers stored in a float column.
fn number_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        Value::Number((n as i64).into())
    } else {
        serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Number(0.into()))
    }
}

/// Maps each field of a collection onto its position in a result set.
///
/// Built **once per query**, not once per row: `Row::get` is a linear
/// scan of the column names, so decoding a 500-row page field-by-field
/// through it would be quadratic in the field count. Decoding then walks
/// the record's value slots positionally — no hashing, no key clones and
/// no intermediate `serde_json::Map`.
pub struct RowDecoder<'a> {
    collection: &'a Arc<Collection>,
    positions: Vec<Option<usize>>,
}

impl<'a> RowDecoder<'a> {
    pub fn new(collection: &'a Arc<Collection>, columns: &[String]) -> Self {
        let positions = collection
            .fields
            .iter()
            .map(|f| columns.iter().position(|c| *c == f.name))
            .collect();
        RowDecoder {
            collection,
            positions,
        }
    }

    pub fn decode(&self, row: &Row) -> Record {
        // `Record::new` lays the fields out in schema order, which is the
        // order `positions` is in, so the slots line up by index.
        let mut record = Record::new(self.collection.clone());
        {
            let data = record.data_mut();
            for (i, field) in self.collection.fields.iter().enumerate() {
                let column = self.positions[i].and_then(|p| row.values.get(p));
                let value = decode_column(field, column);
                // A schema with duplicate field names collapses to fewer
                // slots than fields (PocketBase allows that: last one
                // wins), so the alignment is verified rather than assumed.
                if aligned(data, i, &field.name) {
                    if let Some((_, slot)) = data.get_index_mut(i) {
                        *slot = value;
                    }
                } else {
                    data.insert(field.name.clone(), value);
                }
            }
        }
        // Marks the record as loaded and snapshots `original` for the
        // update diff, which is what `Record::from_loaded` would do.
        record.mark_saved();
        record
    }
}

/// Whether slot `index` of a record's value map belongs to `field`.
pub(crate) fn aligned(data: &indexmap::IndexMap<String, Value>, index: usize, field: &str) -> bool {
    data.get_index(index).is_some_and(|(k, _)| k == field)
}

/// Decode a single row. Prefer [`RowDecoder`] for a result set.
pub fn row_to_record(collection: &Arc<Collection>, row: &Row) -> Record {
    RowDecoder::new(collection, &row.columns).decode(row)
}

fn rows_to_records(collection: &Arc<Collection>, rows: &[Row]) -> Vec<Record> {
    let Some(first) = rows.first() else {
        return vec![];
    };
    let decoder = RowDecoder::new(collection, &first.columns);
    rows.iter().map(|r| decoder.decode(r)).collect()
}

// --- reads ----------------------------------------------------------------

/// List with the collection's `listRule` applied and the user's `filter`
/// AND-ed onto it. Parameter numbering is shared: the rule binds first,
/// the filter continues from where the rule stopped, and `LIMIT`/`OFFSET`
/// take the last two slots.
pub async fn list(
    ex: &dyn Executor,
    store: &CollectionStore,
    ctx: &RequestContext,
    collection: &Arc<Collection>,
    params: ListParams<'_>,
) -> DbResult<ListResult> {
    let per_page = params.per_page.clamp(1, MAX_PER_PAGE);
    let page = params.page.max(1);
    let resolver = CollectionResolver::new(collection.clone(), store, ctx, ex.dialect());

    let rule = rules::evaluate(&collection.list_rule, &resolver, 0)?;
    if rule.is_deny_all() {
        return Ok(empty_result(page, per_page, params.skip_total));
    }

    let mut query = Query::new(collection);
    if let Some(filter) = rule.into_filter() {
        query.push_filter(filter);
    }
    if let Some(expr) = params.filter.map(str::trim).filter(|s| !s.is_empty()) {
        let compiled = cratebase_filter::parse_and_compile(expr, &resolver, query.params().len())?;
        query.push_filter(compiled);
    }
    query.set_order_by(query::order_by(&resolver, params.sort)?);

    let total_items = if params.skip_total {
        -1
    } else {
        ex.query_scalar(&query.count_sql(), query.params())
            .await?
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
    };

    // The SQL is rendered before the page parameters are bound so the
    // `LIMIT $n OFFSET $n+1` placeholders land on the right slots.
    let sql = query.select_sql();
    query.bind_page(per_page, (page - 1) * per_page);
    let rows = ex.query(&sql, query.params()).await?;
    let mut items = rows_to_records(collection, &rows);

    if let Some(spec) = params.expand.map(str::trim).filter(|s| !s.is_empty()) {
        expand::resolve(ex, store, ctx, &mut items, spec, 0).await?;
    }

    let total_pages = match total_items {
        -1 => -1,
        0 => 0,
        n => (n + per_page - 1) / per_page,
    };
    Ok(ListResult {
        items,
        page,
        per_page,
        total_items,
        total_pages,
    })
}

fn empty_result(page: i64, per_page: i64, skip_total: bool) -> ListResult {
    ListResult {
        items: vec![],
        page,
        per_page,
        total_items: if skip_total { -1 } else { 0 },
        total_pages: if skip_total { -1 } else { 0 },
    }
}

/// One record with `viewRule` applied. A record the rule denies is
/// [`DbError::NotFound`], the same answer a missing id gets.
pub async fn find_by_id(
    ex: &dyn Executor,
    store: &CollectionStore,
    ctx: &RequestContext,
    collection: &Arc<Collection>,
    id: &str,
    expand_spec: Option<&str>,
) -> DbResult<Record> {
    let resolver = CollectionResolver::new(collection.clone(), store, ctx, ex.dialect());
    let rule = rules::evaluate(&collection.view_rule, &resolver, 0)?;
    if rule.is_deny_all() {
        return Err(DbError::NotFound);
    }

    let mut query = Query::new(collection);
    if let Some(filter) = rule.into_filter() {
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
    let row = ex
        .query_one(&sql, query.params())
        .await?
        .ok_or(DbError::NotFound)?;
    let mut records = vec![row_to_record(collection, &row)];

    if let Some(spec) = expand_spec.map(str::trim).filter(|s| !s.is_empty()) {
        expand::resolve(ex, store, ctx, &mut records, spec, 0).await?;
    }
    Ok(records.remove(0))
}

/// Rule-free lookup by id, for internal callers (auth, hooks, plugins).
pub async fn find_by_id_raw(
    ex: &dyn Executor,
    collection: &Arc<Collection>,
    id: &str,
) -> DbResult<Record> {
    let sql = format!(
        "SELECT * FROM {} WHERE \"id\" = $1 LIMIT 1",
        quote_ident(collection.table_name())
    );
    let row = ex
        .query_one(&sql, &[Sql::Text(id.to_string())])
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_record(collection, &row))
}

/// Rule-free lookup of several records by id, in one statement. The order
/// of the result follows the database, not `ids`.
pub async fn find_by_ids(
    ex: &dyn Executor,
    collection: &Arc<Collection>,
    ids: &[String],
) -> DbResult<Vec<Record>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let sql = format!(
        "SELECT * FROM {} WHERE \"id\" IN ({})",
        quote_ident(collection.table_name()),
        in_placeholders(1, ids.len())
    );
    let params: Vec<Sql> = ids.iter().map(|id| Sql::Text(id.clone())).collect();
    let rows = ex.query(&sql, &params).await?;
    Ok(rows_to_records(collection, &rows))
}

/// Rule-free "first record matching this filter", PocketBase's
/// `FindFirstRecordByFilter`. `params` fills `{:name}` placeholders in
/// the filter, so a caller never has to build the expression by string
/// concatenation.
pub async fn find_first_by_filter(
    ex: &dyn Executor,
    store: &CollectionStore,
    collection: &Arc<Collection>,
    filter: &str,
    params: &Map<String, Value>,
) -> DbResult<Option<Record>> {
    let ctx = RequestContext::superuser();
    let resolver = CollectionResolver::new(collection.clone(), store, &ctx, ex.dialect());
    let expr = substitute_params(filter, params);
    let compiled = cratebase_filter::parse_and_compile(&expr, &resolver, 0)?;

    let mut query = Query::new(collection);
    query.push_filter(compiled);
    query.set_order_by(query::order_by(&resolver, None)?);
    let sql = query.select_sql();
    query.bind_page(1, 0);
    let row = ex.query_one(&sql, query.params()).await?;
    Ok(row.map(|r| row_to_record(collection, &r)))
}

/// Replace `{:name}` placeholders with filter-language literals. Strings
/// are quoted and escaped, so a value can never break out into the
/// expression; the compiler then binds them as real SQL parameters.
fn substitute_params(filter: &str, params: &Map<String, Value>) -> String {
    if params.is_empty() || !filter.contains("{:") {
        return filter.to_string();
    }
    let mut out = filter.to_string();
    for (key, value) in params {
        let literal = match value {
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            other => {
                let s = match other {
                    Value::String(s) => s.clone(),
                    v => v.to_string(),
                };
                format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
            }
        };
        out = out.replace(&format!("{{:{key}}}"), &literal);
    }
    out
}

/// Total rows in a collection, no rule and no filter.
pub async fn count(ex: &dyn Executor, collection: &Arc<Collection>) -> DbResult<i64> {
    let sql = format!(
        "SELECT COUNT(*) FROM {}",
        quote_ident(collection.table_name())
    );
    Ok(ex
        .query_scalar(&sql, &[])
        .await?
        .and_then(|v| v.as_i64())
        .unwrap_or(0))
}

// --- writes ---------------------------------------------------------------

fn reject_view(collection: &Collection) -> DbResult<()> {
    if collection.is_view() {
        return Err(DbError::Unsupported(format!(
            "collection '{}' is a view and is read-only",
            collection.name
        )));
    }
    Ok(())
}

/// Fill in the values the server owns: autodate stamps, autogenerated
/// text (`id`, `tokenKey`, any field with an `autogeneratePattern`) and
/// the `emailVisibility` default.
fn normalize(record: &mut Record, is_create: bool) {
    let collection = record.collection().clone();
    let stamp = now().to_pb_string();

    // Cast before anything else: PocketBase stores the cast value, so
    // autogeneration and validation must both see it (a number field sent
    // as `"abc"` is `0`, and `0` is blank, so a required one still fails).
    validate::coerce_record(record);

    if is_create && record.id().is_empty() {
        let generated = collection
            .field("id")
            .and_then(|f| match &f.kind {
                FieldKind::Text {
                    autogenerate_pattern,
                    ..
                } if !autogenerate_pattern.is_empty() => {
                    Some(ids::autogenerate(autogenerate_pattern))
                }
                _ => None,
            })
            .unwrap_or_else(ids::record_id);
        record.set_id(generated);
    }

    for field in &collection.fields {
        match &field.kind {
            FieldKind::Autodate {
                on_create,
                on_update,
            } => {
                if (is_create && *on_create) || (!is_create && *on_update) {
                    record.set(&field.name, Value::String(stamp.clone()));
                }
            }
            FieldKind::Text {
                autogenerate_pattern,
                primary_key,
                ..
            } if !autogenerate_pattern.is_empty() && !primary_key => {
                let blank = record
                    .get(&field.name)
                    .map(validate::is_blank)
                    .unwrap_or(true);
                if blank {
                    record.set(
                        &field.name,
                        Value::String(ids::autogenerate(autogenerate_pattern)),
                    );
                }
            }
            _ => {}
        }
    }

    // PocketBase keeps `emailVisibility` a real bool and defaults it to
    // false, so an auth record never leaks an address by accident.
    if collection.is_auth() && !matches!(record.get("emailVisibility"), Some(Value::Bool(_))) {
        record.set("emailVisibility", Value::Bool(false));
    }
}

/// Hash any `password` field holding a fresh plaintext. A value that is
/// already a hash (loaded from storage, or unchanged) is left alone, so
/// an update never re-hashes and invalidates a password.
fn hash_passwords(record: &mut Record) -> DbResult<()> {
    let collection = record.collection().clone();
    for field in collection.fields_of_type(FieldType::Password) {
        let Some(value) = record.get(&field.name) else {
            continue;
        };
        if validate::is_blank(value) || is_hash(value) {
            continue;
        }
        let Some(plain) = value.as_str() else {
            continue;
        };
        let hash = cratebase_auth::hash_password(plain)
            .map_err(|e| DbError::Other(format!("password hashing failed: {e}")))?;
        record.set(&field.name, Value::String(hash));
    }
    Ok(())
}

/// Validate and insert. An empty `record.id()` is generated first (so the
/// caller can rely on the id afterwards), autodate/autogenerate/password
/// normalization runs, then the row is written.
pub async fn create(
    ex: &dyn Executor,
    store: &CollectionStore,
    record: &mut Record,
) -> DbResult<()> {
    create_with_uploads(ex, store, record, &[]).await
}

/// [`create`] with the pending file uploads the caller is about to store,
/// so `maxSelect`/`maxSize`/`mimeTypes` can be enforced.
pub async fn create_with_uploads(
    ex: &dyn Executor,
    store: &CollectionStore,
    record: &mut Record,
    uploads: &[UploadMeta],
) -> DbResult<()> {
    let collection = record.collection().clone();
    reject_view(&collection)?;
    let supplied_id = !record.id().is_empty();

    normalize(record, true);
    if supplied_id {
        if let Some(e) = validate::id_on_create(ex, &collection, record.id()).await? {
            let mut errors = BTreeMap::new();
            errors.insert("id".to_string(), e);
            return Err(DbError::Validation(errors));
        }
    }
    validate::check(ex, store, record, uploads).await?;
    hash_passwords(record)?;

    let mut columns = Vec::with_capacity(collection.fields.len());
    let mut params = Vec::with_capacity(collection.fields.len());
    for field in &collection.fields {
        let value = record.get(&field.name).cloned().unwrap_or(Value::Null);
        columns.push(quote_ident(&field.name));
        params.push(column_value(field, &value));
    }
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        quote_ident(collection.table_name()),
        columns.join(", "),
        in_placeholders(1, params.len())
    );
    ex.execute(&sql, &params)
        .await
        .map_err(|e| map_unique(&collection, e))?;
    record.mark_saved();
    Ok(())
}

/// Validate and update only the fields whose value differs from what was
/// loaded ([`Record`] keeps the original row for exactly this).
pub async fn update(
    ex: &dyn Executor,
    store: &CollectionStore,
    record: &mut Record,
) -> DbResult<()> {
    update_with_uploads(ex, store, record, &[]).await
}

/// [`update`] with pending file uploads; see [`create_with_uploads`].
pub async fn update_with_uploads(
    ex: &dyn Executor,
    store: &CollectionStore,
    record: &mut Record,
    uploads: &[UploadMeta],
) -> DbResult<()> {
    let collection = record.collection().clone();
    reject_view(&collection)?;
    if record.is_new() {
        return Err(DbError::Unsupported(
            "update called on a record that was never loaded".into(),
        ));
    }
    let id = record.id().to_string();
    if id.is_empty() {
        return Err(DbError::NotFound);
    }

    normalize(record, false);
    validate::check(ex, store, record, uploads).await?;
    hash_passwords(record)?;

    let mut assignments = Vec::new();
    let mut params = Vec::new();
    for field in &collection.fields {
        // The primary key is never rewritten; PocketBase does not allow
        // changing a record's id after creation.
        if field.is_primary_key() {
            continue;
        }
        let value = record.get(&field.name).cloned().unwrap_or(Value::Null);
        if record.original(&field.name) == Some(&value) {
            continue;
        }
        params.push(column_value(field, &value));
        assignments.push(format!("{} = ${}", quote_ident(&field.name), params.len()));
    }
    if assignments.is_empty() {
        return Ok(());
    }
    params.push(Sql::Text(id));
    let sql = format!(
        "UPDATE {} SET {} WHERE \"id\" = ${}",
        quote_ident(collection.table_name()),
        assignments.join(", "),
        params.len()
    );
    let affected = ex
        .execute(&sql, &params)
        .await
        .map_err(|e| map_unique(&collection, e))?;
    if affected == 0 {
        return Err(DbError::NotFound);
    }
    record.mark_saved();
    Ok(())
}

/// Map a driver unique violation onto the field its index covers.
fn map_unique(collection: &Collection, e: DbError) -> DbError {
    match e {
        DbError::UniqueViolation(detail) => query::unique_violation(collection, &detail),
        other => other,
    }
}

/// Delete a record and everything that must follow it.
///
/// For every relation field on any *other* collection that points at this
/// one, PocketBase either deletes the referencing records
/// (`cascadeDelete: true`, applied recursively) or strips this id out of
/// their value (`''` for a single relation, removal from the array for a
/// multi one). A visited set stops a relation cycle from looping.
///
/// The returned [`FileRef`]s cover every record actually deleted,
/// including cascaded ones.
pub async fn delete(
    ex: &dyn Executor,
    store: &CollectionStore,
    record: &Record,
) -> DbResult<Vec<FileRef>> {
    let collection = record.collection().clone();
    reject_view(&collection)?;
    if record.id().is_empty() {
        return Err(DbError::NotFound);
    }

    let mut files = Vec::new();
    let mut visited: HashSet<(String, String)> = HashSet::new();
    let mut queue: Vec<(Arc<Collection>, String)> = vec![(collection, record.id().to_string())];
    let mut first = true;

    while let Some((collection, id)) = queue.pop() {
        if !visited.insert((collection.id.clone(), id.clone())) {
            continue;
        }
        // The caller already loaded the root record; everything cascaded
        // has to be fetched to know which files it owned.
        let loaded = if first {
            first = false;
            Some(record.clone())
        } else {
            match find_by_id_raw(ex, &collection, &id).await {
                Ok(r) => Some(r),
                Err(DbError::NotFound) => None,
                Err(e) => return Err(e),
            }
        };
        let Some(loaded) = loaded else { continue };
        files.extend(file_refs(&loaded));

        for (referencing, field) in referencing_fields(store, &collection) {
            if field.cascade_delete() {
                for target in referencing_ids(ex, &referencing, &field, &id).await? {
                    queue.push((referencing.clone(), target));
                }
            } else {
                strip_reference(ex, &referencing, &field, &id).await?;
            }
        }

        let sql = format!(
            "DELETE FROM {} WHERE \"id\" = $1",
            quote_ident(collection.table_name())
        );
        let affected = ex.execute(&sql, &[Sql::Text(id.clone())]).await?;
        if affected == 0 && visited.len() == 1 {
            return Err(DbError::NotFound);
        }
    }
    Ok(files)
}

/// Whether [`delete`] can touch anything beyond the record's own row.
///
/// A relation field pointing at `target` is either cascaded into (more
/// `DELETE`s, plus a `SELECT` per cascaded row) or has its reference
/// stripped (an `UPDATE`), so a referenced collection always means a
/// multi-statement delete that has to be atomic. With no referencing
/// field the whole operation is one `DELETE ... WHERE id = ?`, which
/// SQLite already runs atomically on its own — the caller can then skip
/// `BEGIN IMMEDIATE`/`COMMIT` and the two extra writer round trips they
/// cost.
///
/// Cheaper than [`referencing_fields`] on the hot path: it stops at the
/// first hit and clones nothing.
pub fn delete_touches_other_collections(store: &CollectionStore, target: &Collection) -> bool {
    store.all().all.iter().any(|collection| {
        !collection.is_view()
            && collection
                .fields_of_type(FieldType::Relation)
                .any(|field| field.relation_collection_id() == Some(target.id.as_str()))
    })
}

/// Every `(collection, relation field)` pair pointing at `target`,
/// excluding `target`'s own self-relations' owner (a self-relation still
/// counts, but the visited set stops the recursion).
fn referencing_fields(
    store: &CollectionStore,
    target: &Collection,
) -> Vec<(Arc<Collection>, Field)> {
    let mut out = Vec::new();
    for collection in &store.all().all {
        if collection.is_view() {
            continue;
        }
        for field in collection.fields_of_type(FieldType::Relation) {
            if field.relation_collection_id() == Some(target.id.as_str()) {
                out.push((collection.clone(), field.clone()));
            }
        }
    }
    out
}

/// A multi-valued column as a JSON array, tolerating the two non-array
/// states a column can be left in: `NULL` (the schema default) and `''`
/// (a field that used to be single-valued). `json_each` on either would
/// abort the statement with "malformed JSON".
pub(crate) fn json_array(column: &str) -> String {
    format!("CASE WHEN {column} IS NULL OR {column} = '' THEN '[]' ELSE {column} END")
}

/// SQL testing whether `field` on `table` references `$1`.
fn references_condition(field: &Field, dialect: cratebase_filter::Dialect) -> String {
    let column = quote_ident(&field.name);
    if !field.is_multiple() {
        return format!("{column} = $1");
    }
    let array = json_array(&column);
    match dialect {
        cratebase_filter::Dialect::Sqlite => {
            format!("EXISTS (SELECT 1 FROM json_each({array}) WHERE \"value\" = $1)")
        }
        cratebase_filter::Dialect::Postgres => {
            format!("jsonb_exists(({array})::jsonb, $1)")
        }
    }
}

async fn referencing_ids(
    ex: &dyn Executor,
    collection: &Arc<Collection>,
    field: &Field,
    id: &str,
) -> DbResult<Vec<String>> {
    let sql = format!(
        "SELECT \"id\" FROM {} WHERE {}",
        quote_ident(collection.table_name()),
        references_condition(field, ex.dialect())
    );
    let rows = ex.query(&sql, &[Sql::Text(id.to_string())]).await?;
    Ok(rows
        .iter()
        .filter_map(|r| r.get_str("id").map(str::to_string))
        .collect())
}

/// Remove `id` from a non-cascading relation field: `''` for a single
/// relation, filtered out of the array for a multi one.
async fn strip_reference(
    ex: &dyn Executor,
    collection: &Arc<Collection>,
    field: &Field,
    id: &str,
) -> DbResult<()> {
    let table = quote_ident(collection.table_name());
    let column = quote_ident(&field.name);
    if !field.is_multiple() {
        let sql = format!("UPDATE {table} SET {column} = '' WHERE {column} = $1");
        ex.execute(&sql, &[Sql::Text(id.to_string())]).await?;
        return Ok(());
    }
    // Rewriting the JSON array in Rust keeps one code path for both
    // backends instead of two dialects of JSON surgery in SQL.
    let sql = format!(
        "SELECT \"id\", {column} FROM {table} WHERE {}",
        references_condition(field, ex.dialect())
    );
    let rows = ex.query(&sql, &[Sql::Text(id.to_string())]).await?;
    for row in rows {
        let Some(row_id) = row.get_str("id").map(str::to_string) else {
            continue;
        };
        let current = decode_column(field, row.get(&field.name));
        let kept: Vec<Value> = match current {
            Value::Array(items) => items
                .into_iter()
                .filter(|v| v.as_str() != Some(id))
                .collect(),
            other => vec![other],
        };
        let update = format!("UPDATE {table} SET {column} = $1 WHERE \"id\" = $2");
        ex.execute(
            &update,
            &[Sql::Text(Value::Array(kept).to_string()), Sql::Text(row_id)],
        )
        .await?;
    }
    Ok(())
}

/// Every file name a record owns, as storage keys.
fn file_refs(record: &Record) -> Vec<FileRef> {
    let collection = record.collection();
    let mut out = Vec::new();
    for field in collection.fields_of_type(FieldType::File) {
        for name in record.get_string_list(&field.name) {
            if name.is_empty() {
                continue;
            }
            out.push(FileRef {
                collection_id: collection.id.clone(),
                record_id: record.id().to_string(),
                filename: name,
            });
        }
    }
    out
}

/// Build a [`Record`] from a request body, ignoring keys that are not
/// fields (PocketBase silently drops unknown keys) and the reserved
/// names it never accepts.
pub fn from_body(collection: Arc<Collection>, body: &Map<String, Value>) -> Record {
    let mut record = Record::new(collection.clone());
    for (key, value) in body {
        if RESERVED_FIELD_NAMES.contains(&key.as_str()) {
            continue;
        }
        if collection.has_field(key) {
            record.set(key, value.clone());
        }
    }
    record
}

/// Apply a request body onto a loaded record, leaving untouched fields at
/// their stored value (PocketBase's `PATCH` semantics).
pub fn apply_body(record: &mut Record, body: &Map<String, Value>) {
    let collection = record.collection().clone();
    for (key, value) in body {
        if RESERVED_FIELD_NAMES.contains(&key.as_str()) {
            continue;
        }
        if collection.has_field(key) {
            record.set(key, value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cratebase_core::{CollectionType, FieldKind};
    use serde_json::json;

    /// The route layer skips `BEGIN IMMEDIATE`/`COMMIT` for a delete this
    /// says is confined to one row, so a wrong answer here is a lost
    /// cascade, not a slow one.
    #[test]
    fn spots_the_collections_a_delete_would_reach_beyond_its_own_row() {
        let posts = Collection::new("posts", CollectionType::Base);
        let mut comments = Collection::new("comments", CollectionType::Base);
        let mut unrelated = Collection::new("tags", CollectionType::Base);
        unrelated.fields.push(Field::new(
            "topic",
            FieldKind::Relation {
                collection_id: "some_other_collection".into(),
                cascade_delete: true,
                min_select: 0,
                max_select: 1,
            },
        ));

        let store = CollectionStore::new();
        store.replace(vec![posts.clone(), unrelated.clone()]);
        assert!(!delete_touches_other_collections(&store, &posts));

        // A relation that does *not* cascade still turns the delete into
        // several statements: the reference has to be stripped.
        for cascade_delete in [false, true] {
            comments.fields.retain(|f| f.name != "post");
            comments.fields.push(Field::new(
                "post",
                FieldKind::Relation {
                    collection_id: posts.id.clone(),
                    cascade_delete,
                    min_select: 0,
                    max_select: 1,
                },
            ));
            store.replace(vec![posts.clone(), comments.clone(), unrelated.clone()]);
            assert!(
                delete_touches_other_collections(&store, &posts),
                "cascade_delete = {cascade_delete}"
            );
            // The relation points one way only.
            assert!(!delete_touches_other_collections(&store, &comments));
        }
    }

    #[test]
    fn substitutes_named_filter_params_safely() {
        let mut params = Map::new();
        params.insert("name".into(), json!("it's"));
        params.insert("n".into(), json!(3));
        assert_eq!(
            substitute_params("a = {:name} && b > {:n}", &params),
            "a = 'it\\'s' && b > 3"
        );
        assert_eq!(substitute_params("a = 1", &params), "a = 1");
    }

    #[test]
    fn encodes_and_decodes_every_field_shape() {
        let multi = Field::new(
            "tags",
            FieldKind::Select {
                values: vec!["a".into()],
                max_select: 3,
            },
        );
        assert_eq!(
            column_value(&multi, &json!(["a"])),
            Sql::Text("[\"a\"]".into())
        );
        assert_eq!(column_value(&multi, &Value::Null), Sql::Text("[]".into()));
        assert_eq!(
            decode_column(&multi, Some(&Sql::Text("[\"a\"]".into()))),
            json!(["a"])
        );
        assert_eq!(decode_column(&multi, Some(&Sql::Null)), json!([]));

        let number = Field::new(
            "views",
            FieldKind::Number {
                min: None,
                max: None,
                only_int: false,
            },
        );
        assert_eq!(column_value(&number, &json!(2)), Sql::Real(2.0));
        assert_eq!(decode_column(&number, Some(&Sql::Real(2.0))), json!(2));
        assert_eq!(decode_column(&number, Some(&Sql::Real(2.5))), json!(2.5));

        let flag = Field::new("published", FieldKind::Bool {});
        assert_eq!(column_value(&flag, &json!(true)), Sql::Int(1));
        assert_eq!(decode_column(&flag, Some(&Sql::Int(1))), json!(true));

        let data = Field::new("data", FieldKind::Json { max_size: 0 });
        assert_eq!(
            column_value(&data, &json!({"a": 1})),
            Sql::Text("{\"a\":1}".into())
        );
        assert_eq!(column_value(&data, &Value::Null), Sql::Null);
        assert_eq!(
            decode_column(&data, Some(&Sql::Text("{\"a\":1}".into()))),
            json!({"a": 1})
        );
        assert_eq!(decode_column(&data, Some(&Sql::Null)), Value::Null);

        let text = Field::new("title", FieldKind::default_for(FieldType::Text));
        assert_eq!(column_value(&text, &Value::Null), Sql::Text(String::new()));
        assert_eq!(decode_column(&text, Some(&Sql::Null)), json!(""));
        assert_eq!(decode_column(&text, None), json!(""));
    }

    #[test]
    fn body_is_filtered_to_real_fields() {
        let mut c = Collection::new("posts", CollectionType::Base);
        let pos = c.fields.len() - 2;
        c.fields.insert(
            pos,
            Field::new("title", FieldKind::default_for(FieldType::Text)),
        );
        let c = Arc::new(c);
        let body = json!({"title": "hi", "collectionId": "x", "nope": 1})
            .as_object()
            .cloned()
            .unwrap();
        let record = from_body(c, &body);
        assert_eq!(record.get("title"), Some(&json!("hi")));
        assert!(record.get("nope").is_none());
    }

    #[test]
    fn hash_detection() {
        assert!(is_hash(&json!("$argon2id$v=19$...")));
        assert!(is_hash(&json!("$2a$05$abc")));
        assert!(!is_hash(&json!("plaintext")));
        assert!(!is_hash(&Value::Null));
    }
}
