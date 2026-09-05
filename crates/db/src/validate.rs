//! Per-field record validation, using PocketBase's `validation_*` codes
//! and wording so SDK clients that branch on `code` keep working.
//!
//! Each field type has its own function returning `Option<FieldError>`;
//! [`record`] runs them all and collects the failures into the map that
//! becomes the `data` object of a `400` response.
//!
//! Three PocketBase behaviors worth calling out, because they surprise
//! people (all three are pinned by `tests/conformance`):
//!
//! * "Blank" means the zero value, not just `null`: `""`, `[]`, `0` and
//!   `false` are all blank, so a required `bool` must be `true` and a
//!   required `number` must be non-zero (this is ozzo-validation's
//!   `Required`, which PocketBase uses).
//! * A blank *optional* value skips every other check — an empty string
//!   in an `email` field is fine, an empty string in a `required` one is
//!   not.
//! * **PocketBase coerces far more than it rejects.** `views: "abc"` on a
//!   number field stores `0`, an unparsable date stores `""`, and a
//!   single-valued select handed an array keeps the *last* element. Only
//!   `onlyInt`, the min/max bounds and the format checks are real errors.
//!   [`coerce`] runs before validation and does that shaping, which is
//!   why the per-type functions below can assume a well-typed value.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use cratebase_core::{
    codes, Collection, DateTime, Field, FieldError, FieldKind, FieldType, Record,
};
use serde_json::{Map, Value};

use crate::collections::CollectionStore;
use crate::engine::{quote_ident, Executor, Sql};
use crate::error::{DbError, DbResult};
use crate::query::in_placeholders;

/// Codes PocketBase uses that are not in [`cratebase_core::codes`], or
/// that differ from the obvious guess. All four are pinned by
/// `tests/conformance/records.test.ts`.
const JSON_SIZE_LIMIT: &str = "validation_json_size_limit";
/// Not `validation_is_url` — PocketBase's url field uses its own code.
const INVALID_URL: &str = "validation_invalid_url";
/// Date bounds reuse ozzo's comparison codes rather than a date-specific
/// one, and render the threshold in Go's default time format.
const MIN_DATE: &str = "validation_min_greater_equal_than_required";
const MAX_DATE: &str = "validation_max_less_equal_than_required";
/// Files have their own "too many" code, distinct from select's.
const TOO_MANY_FILES: &str = "validation_too_many_files";
/// Cratebase-specific: PocketBase has no `vector` field type, so there is
/// no upstream code to match.
const VECTOR_DIMENSION_MISMATCH: &str = "validation_vector_dimension_mismatch";

/// Metadata about one file the caller is about to store. `crates/db`
/// never touches storage, so the server layer supplies this after
/// parsing the multipart body.
#[derive(Debug, Clone)]
pub struct UploadMeta {
    /// Name of the file field the upload belongs to.
    pub field: String,
    /// Stored file name (what ends up in the record's value).
    pub name: String,
    pub size: i64,
    pub mime: String,
}

type Errors = BTreeMap<String, FieldError>;

fn err(code: &str, message: impl Into<String>) -> FieldError {
    FieldError::new(code, message)
}

/// PocketBase's notion of a blank value: the type's zero value.
pub fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Bool(b) => !*b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::Object(o) => o.is_empty(),
    }
}

/// `Cannot be blank.` when a required field holds its zero value.
pub fn required(field: &Field, value: &Value) -> Option<FieldError> {
    if field.required && is_blank(value) {
        return Some(err(codes::REQUIRED, "Cannot be blank."));
    }
    None
}

/// Values of a single- or multi-valued field as a list of strings.
/// A non-string element is rendered with `to_string` so the caller can
/// report it back verbatim.
fn string_list(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect(),
        Value::String(s) if s.is_empty() => vec![],
        Value::String(s) => vec![s.clone()],
        Value::Null => vec![],
        other => vec![other.to_string()],
    }
}

/// Compiled field patterns, keyed by the pattern source.
///
/// Validation runs on every write, and `regex::Regex::new` costs orders
/// of magnitude more than the match itself — compiling `^[a-z0-9]+$`
/// afresh for every record's `id` would dominate the write path. The set
/// of distinct patterns is bounded by the schema, so the cache never
/// needs eviction; a pattern that does not compile is cached as a
/// failure too, so a broken schema is not re-parsed on every request.
type PatternCache = Mutex<HashMap<String, Option<Arc<regex::Regex>>>>;

fn pattern_cache() -> &'static PatternCache {
    static CACHE: OnceLock<PatternCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn compile_pattern(pattern: &str) -> Result<Arc<regex::Regex>, FieldError> {
    let invalid = || {
        err(
            codes::INVALID_PATTERN,
            format!("Invalid pattern '{pattern}' in the field definition."),
        )
    };
    let mut cache = pattern_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = cache.get(pattern) {
        return hit.clone().ok_or_else(invalid);
    }
    let compiled = regex::Regex::new(pattern).ok().map(Arc::new);
    cache.insert(pattern.to_string(), compiled.clone());
    compiled.ok_or_else(invalid)
}

/// `text`, and (with its own options) `editor`. Length is counted in
/// characters, not bytes, like Go's `utf8.RuneCountInString`.
pub fn text(field: &Field, value: &Value) -> Option<FieldError> {
    let (min, max, pattern) = match &field.kind {
        FieldKind::Text {
            min, max, pattern, ..
        } => (*min, *max, pattern.as_str()),
        FieldKind::Editor { max_size, .. } => (0, *max_size, ""),
        _ => (0, 0, ""),
    };
    let Value::String(s) = value else {
        return Some(err(codes::INVALID_FORMAT, "Must be a valid string."));
    };
    let len = s.chars().count() as i64;
    if min > 0 && len < min {
        return Some(err(
            codes::MIN_TEXT,
            format!("Must be at least {min} character(s)."),
        ));
    }
    if max > 0 && len > max {
        return Some(err(
            codes::MAX_TEXT,
            format!("Must be no more than {max} character(s)."),
        ));
    }
    if !pattern.is_empty() {
        match compile_pattern(pattern) {
            Err(e) => return Some(e),
            Ok(re) if !re.is_match(s) => {
                return Some(err(codes::PATTERN_MISMATCH, "Invalid value format."))
            }
            Ok(_) => {}
        }
    }
    None
}

/// `number`. A non-numeric value never reaches here — [`coerce`] has
/// already turned it into `0`, which is what PocketBase stores — so the
/// only failures are `onlyInt` and the bounds.
pub fn number(field: &Field, value: &Value) -> Option<FieldError> {
    let FieldKind::Number { min, max, only_int } = &field.kind else {
        return None;
    };
    let Some(n) = value.as_f64() else {
        return Some(err(codes::INVALID_NUMBER, "Must be a valid number."));
    };
    if *only_int && n.fract() != 0.0 {
        return Some(err(codes::ONLY_INT, "Decimal numbers are not allowed."));
    }
    if let Some(min) = min {
        if n < *min {
            return Some(err(
                codes::MIN_NUMBER,
                format!("Must be no less than {}.", trim_float(*min)),
            ));
        }
    }
    if let Some(max) = max {
        if n > *max {
            return Some(err(
                codes::MAX_NUMBER,
                format!("Must be no greater than {}.", trim_float(*max)),
            ));
        }
    }
    None
}

/// `10` rather than `10.0` in a message, matching Go's `%v` on a float
/// that happens to be whole.
fn trim_float(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

pub fn boolean(value: &Value) -> Option<FieldError> {
    match value {
        Value::Bool(_) => None,
        _ => Some(err(codes::INVALID_BOOL, "Must be a valid boolean.")),
    }
}

/// A deliberately permissive check, the same shape PocketBase's
/// `is.EmailFormat` accepts: `local@domain.tld`.
fn looks_like_email(s: &str) -> bool {
    let Some((local, domain)) = s.rsplit_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains(' ')
        && !local.contains(' ')
}

/// `onlyDomains` wins when both lists are set, matching PocketBase.
fn domain_allowed(domain: &str, only: &[String], except: &[String]) -> bool {
    let eq = |d: &String| d.eq_ignore_ascii_case(domain);
    if !only.is_empty() {
        return only.iter().any(eq);
    }
    !except.iter().any(eq)
}

pub fn email(field: &Field, value: &Value) -> Option<FieldError> {
    let FieldKind::Email {
        only_domains,
        except_domains,
    } = &field.kind
    else {
        return None;
    };
    let Some(s) = value.as_str() else {
        return Some(err(codes::INVALID_EMAIL, "Must be a valid email address."));
    };
    if !looks_like_email(s) {
        return Some(err(codes::INVALID_EMAIL, "Must be a valid email address."));
    }
    let domain = s.rsplit_once('@').map(|(_, d)| d).unwrap_or_default();
    if !domain_allowed(domain, only_domains, except_domains) {
        return Some(err(
            codes::NOT_ALLOWED_DOMAIN,
            "Email domain is not allowed.",
        ));
    }
    None
}

pub fn url(field: &Field, value: &Value) -> Option<FieldError> {
    let FieldKind::Url {
        only_domains,
        except_domains,
    } = &field.kind
    else {
        return None;
    };
    let Some(s) = value.as_str() else {
        return Some(err(INVALID_URL, "Must be a valid url."));
    };
    let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    else {
        return Some(err(INVALID_URL, "Must be a valid url."));
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    if host.is_empty() || host.contains(' ') {
        return Some(err(INVALID_URL, "Must be a valid url."));
    }
    if !domain_allowed(host, only_domains, except_domains) {
        return Some(err(codes::NOT_ALLOWED_DOMAIN, "Url domain is not allowed."));
    }
    None
}

pub fn date(field: &Field, value: &Value) -> Option<FieldError> {
    let FieldKind::Date { min, max } = &field.kind else {
        return None;
    };
    let parsed = value.as_str().and_then(DateTime::parse);
    let Some(dt) = parsed else {
        return Some(err(codes::INVALID_DATE, "Must be a valid date."));
    };
    if let Some(min) = min {
        if dt < *min {
            return Some(
                err(MIN_DATE, format!("Must be no less than {}.", go_time(min)))
                    .with_param("threshold", Value::String(threshold(min))),
            );
        }
    }
    if let Some(max) = max {
        if dt > *max {
            return Some(
                err(
                    MAX_DATE,
                    format!("Must be no greater than {}.", go_time(max)),
                )
                .with_param("threshold", Value::String(threshold(max))),
            );
        }
    }
    None
}

/// The machine-readable form of a date bound in `params.threshold`. Go
/// marshals `time.Time` as RFC3339 with trailing zero fractions trimmed,
/// so a whole second renders as `2020-01-01T00:00:00Z` — not the
/// millisecond-padded form we use elsewhere.
fn threshold(dt: &DateTime) -> String {
    dt.inner()
        .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
}

/// Go's default `time.Time` rendering, which is what ends up in
/// PocketBase's date-bound messages: `2020-01-01 00:00:00 +0000 UTC`.
fn go_time(dt: &DateTime) -> String {
    dt.inner().format("%Y-%m-%d %H:%M:%S +0000 UTC").to_string()
}

pub fn select(field: &Field, value: &Value) -> Option<FieldError> {
    let FieldKind::Select { values, max_select } = &field.kind else {
        return None;
    };
    let selected = string_list(value);
    if selected.len() as i64 > (*max_select).max(1) {
        return Some(err(
            codes::MAX_SELECT,
            format!("Select no more than {max_select}."),
        ));
    }
    for v in &selected {
        if !values.contains(v) {
            return Some(err(codes::NOT_IN_LIST, format!("Invalid value '{v}'.")));
        }
    }
    None
}

/// `file` counts the values already on the record plus the pending
/// uploads for this field; size and mime are checked per upload, since
/// only the caller knows anything about bytes on the wire.
pub fn file(field: &Field, value: &Value, uploads: &[UploadMeta]) -> Option<FieldError> {
    let FieldKind::File {
        max_select,
        max_size,
        mime_types,
        ..
    } = &field.kind
    else {
        return None;
    };
    let mine: Vec<&UploadMeta> = uploads.iter().filter(|u| u.field == field.name).collect();
    let kept = string_list(value)
        .into_iter()
        .filter(|name| !mine.iter().any(|u| &u.name == name))
        .count();
    if (kept + mine.len()) as i64 > (*max_select).max(1) {
        return Some(err(
            TOO_MANY_FILES,
            format!("The maximum allowed files is {max_select}."),
        ));
    }
    for u in &mine {
        if *max_size > 0 && u.size > *max_size {
            return Some(err(
                codes::FILE_TOO_LARGE,
                format!(
                    "Failed to upload {} - the maximum allowed file size is {max_size} bytes.",
                    u.name
                ),
            ));
        }
        if !mime_types.is_empty() && !mime_types.iter().any(|m| m == &u.mime) {
            return Some(err(
                codes::FILE_MIME,
                format!(
                    "{} mime type must be one of: {}.",
                    u.name,
                    mime_types.join(", ")
                ),
            ));
        }
    }
    None
}

pub fn json(field: &Field, value: &Value) -> Option<FieldError> {
    let FieldKind::Json { max_size } = &field.kind else {
        return None;
    };
    // A json field submitted as a string is accepted only if the string
    // itself is valid JSON, matching PocketBase's `json.Valid` check.
    let encoded = match value {
        Value::String(s) => {
            if serde_json::from_str::<Value>(s).is_err() {
                return Some(err(codes::INVALID_JSON, "Must be a valid json value."));
            }
            s.clone()
        }
        other => other.to_string(),
    };
    if *max_size > 0 && encoded.len() as i64 > *max_size {
        return Some(err(
            JSON_SIZE_LIMIT,
            format!("The maximum allowed JSON size is {max_size} bytes."),
        ));
    }
    None
}

/// `vector`: must be a JSON array of exactly `dimensions` numbers. A
/// blank value (`null`/missing) already short-circuits in [`record`]
/// before this runs, same as every other optional field.
pub fn vector(field: &Field, value: &Value) -> Option<FieldError> {
    let FieldKind::Vector { dimensions, .. } = &field.kind else {
        return None;
    };
    let Value::Array(items) = value else {
        return Some(err(codes::INVALID_FORMAT, "Must be a valid vector array."));
    };
    if !items.iter().all(Value::is_number) {
        return Some(err(codes::INVALID_FORMAT, "Must be an array of numbers."));
    }
    if items.len() != *dimensions {
        return Some(err(
            VECTOR_DIMENSION_MISMATCH,
            format!(
                "Must be an array of exactly {dimensions} number(s), got {}.",
                items.len()
            ),
        ));
    }
    None
}

/// Password constraints apply to the *plaintext*; a value that is already
/// a stored hash is never re-validated (see [`crate::records::is_hash`]).
pub fn password(field: &Field, value: &Value) -> Option<FieldError> {
    let FieldKind::Password {
        min, max, pattern, ..
    } = &field.kind
    else {
        return None;
    };
    let Some(s) = value.as_str() else {
        return Some(err(codes::INVALID_FORMAT, "Must be a valid string."));
    };
    let len = s.chars().count() as i64;
    if *min > 0 && len < *min {
        return Some(err(
            codes::MIN_TEXT,
            format!("Must be at least {min} character(s)."),
        ));
    }
    if *max > 0 && len > *max {
        return Some(err(
            codes::MAX_TEXT,
            format!("Must be no more than {max} character(s)."),
        ));
    }
    if !pattern.is_empty() {
        match compile_pattern(pattern) {
            Err(e) => return Some(e),
            Ok(re) if !re.is_match(s) => {
                return Some(err(codes::PATTERN_MISMATCH, "Invalid value format."))
            }
            Ok(_) => {}
        }
    }
    None
}

pub fn geo_point(value: &Value) -> Option<FieldError> {
    let Value::Object(m) = value else {
        return Some(err(
            codes::INVALID_FORMAT,
            "Must be a valid geo point ({lon, lat}).",
        ));
    };
    let lon = m.get("lon").and_then(Value::as_f64);
    let lat = m.get("lat").and_then(Value::as_f64);
    match (lon, lat) {
        (Some(lon), Some(lat))
            if (-180.0..=180.0).contains(&lon) && (-90.0..=90.0).contains(&lat) =>
        {
            None
        }
        (Some(_), Some(_)) => Some(err(
            codes::INVALID_FORMAT,
            "Longitude must be between -180 and 180, latitude between -90 and 90.",
        )),
        _ => Some(err(
            codes::INVALID_FORMAT,
            "Must be a valid geo point ({lon, lat}).",
        )),
    }
}

/// `relation`: cardinality plus one batched existence check per field.
pub async fn relation(
    ex: &dyn Executor,
    store: &CollectionStore,
    field: &Field,
    value: &Value,
) -> DbResult<Option<FieldError>> {
    let FieldKind::Relation {
        collection_id,
        min_select,
        max_select,
        ..
    } = &field.kind
    else {
        return Ok(None);
    };
    let ids = string_list(value);
    if (ids.len() as i64) < *min_select {
        return Ok(Some(err(
            codes::MIN_SELECT,
            format!("Select at least {min_select}."),
        )));
    }
    if ids.len() as i64 > (*max_select).max(1) {
        return Ok(Some(err(
            codes::MAX_SELECT,
            format!("Select no more than {max_select}."),
        )));
    }
    if ids.is_empty() {
        return Ok(None);
    }
    let Some(target) = store.get_by_id(collection_id) else {
        return Ok(Some(err(
            codes::REL_COLLECTION,
            "The relation target collection no longer exists.",
        )));
    };
    let unique: Vec<String> = {
        let mut seen = HashSet::new();
        ids.iter()
            .filter(|id| seen.insert((*id).clone()))
            .cloned()
            .collect()
    };
    let sql = format!(
        "SELECT \"id\" FROM {} WHERE \"id\" IN ({})",
        quote_ident(target.table_name()),
        in_placeholders(1, unique.len())
    );
    let params: Vec<Sql> = unique.iter().map(|id| Sql::Text(id.clone())).collect();
    let rows = ex.query(&sql, &params).await?;
    let found: HashSet<String> = rows
        .iter()
        .filter_map(|r| r.get_str("id").map(str::to_string))
        .collect();
    if unique.iter().any(|id| !found.contains(id)) {
        return Ok(Some(err(
            codes::MISSING_REL,
            "Failed to find all relation records with the provided ids.",
        )));
    }
    Ok(None)
}

/// The `id` of a record being created: format from the field definition
/// (PocketBase's default is `^[a-z0-9]+$`, exactly 15 characters) plus a
/// uniqueness probe, so a duplicate is a `validation_not_unique` on `id`
/// rather than an opaque primary-key violation.
pub async fn id_on_create(
    ex: &dyn Executor,
    collection: &Collection,
    id: &str,
) -> DbResult<Option<FieldError>> {
    let Some(field) = collection.field("id") else {
        return Ok(None);
    };
    let value = Value::String(id.to_string());
    if let Some(e) = text(field, &value) {
        return Ok(Some(e));
    }
    let sql = format!(
        "SELECT \"id\" FROM {} WHERE \"id\" = $1",
        quote_ident(collection.table_name())
    );
    if ex
        .query_one(&sql, &[Sql::Text(id.to_string())])
        .await?
        .is_some()
    {
        return Ok(Some(err(codes::NOT_UNIQUE, "Value must be unique.")));
    }
    Ok(None)
}

/// Shape a submitted value into what PocketBase actually stores.
///
/// This is the counterpart of Go's `field.PrepareValue`, and it is the
/// reason so few of the checks above ever fire: PocketBase casts first
/// and validates second, so `views: "abc"` becomes `0`, `when: "not a
/// date"` becomes `""`, `published: "true"` becomes `true` and a
/// single-valued `category: ["news", "blog"]` keeps `"blog"`.
///
/// `autodate` is left alone (its value is server-computed) and a value
/// already in its stored shape is returned unchanged, which makes this
/// safe to run over a record loaded from the database.
pub fn coerce(field: &Field, value: &Value) -> Value {
    coerce_changed(field, value).unwrap_or_else(|| value.clone())
}

/// [`coerce`] that returns `None` when the value is already in its stored
/// shape — the overwhelmingly common case for a well-formed JSON body and
/// for every record read back from the database. Keeping it allocation
/// free there is what lets [`coerce_record`] run on every write.
fn coerce_changed(field: &Field, value: &Value) -> Option<Value> {
    if field.field_type() == FieldType::Autodate {
        return None;
    }
    if field.is_multiple() {
        if let Value::Array(items) = value {
            if items
                .iter()
                .all(|v| matches!(v, Value::String(s) if !s.is_empty()))
            {
                return None;
            }
        }
        return Some(Value::Array(
            string_list(value)
                .into_iter()
                .filter(|s| !s.is_empty())
                .map(Value::String)
                .collect(),
        ));
    }
    match field.field_type() {
        FieldType::Number => match value {
            Value::Number(_) => None,
            other => Some(Value::Number(
                serde_json::Number::from_f64(to_f64(other)).unwrap_or_else(|| 0.into()),
            )),
        },
        FieldType::Bool => match value {
            Value::Bool(_) => None,
            other => Some(Value::Bool(to_bool(other))),
        },
        FieldType::Date => {
            let raw = value.as_str().unwrap_or_default();
            match DateTime::parse(raw) {
                // Already normalized (which is how it comes back from a
                // column), so nothing to do.
                Some(dt) if dt.to_pb_string() == raw => None,
                Some(dt) => Some(Value::String(dt.to_pb_string())),
                // Garbage is the zero date, which serializes as "".
                None if raw.is_empty() && value.is_string() => None,
                None => Some(Value::String(String::new())),
            }
        }
        // A json/geoPoint/vector field submitted as text (multipart, or
        // a client that pre-encodes) is stored decoded.
        FieldType::Json | FieldType::GeoPoint | FieldType::Vector => match value {
            Value::String(s) => serde_json::from_str(s).ok(),
            _ => None,
        },
        // Single-valued select/file/relation handed a list keep the last
        // element, which is where PocketBase's cast lands.
        FieldType::Select | FieldType::File | FieldType::Relation => match value {
            Value::String(_) => None,
            other => Some(Value::String(string_list(other).pop().unwrap_or_default())),
        },
        _ => match value {
            Value::String(_) => None,
            other => Some(Value::String(to_string(other))),
        },
    }
}

/// Go's `cast.ToFloat64` with its error swallowed: anything unparsable
/// is `0`.
fn to_f64(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::String(s) => s.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Go's `cast.ToBool`: only the canonical truthy spellings count.
fn to_bool(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => matches!(s.as_str(), "1" | "t" | "T" | "true" | "TRUE" | "True"),
        _ => false,
    }
}

/// Go's `cast.ToString`: scalars stringify, containers become `""`.
fn to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// Run [`coerce`] over every field of a record, in place.
pub fn coerce_record(record: &mut Record) {
    let collection = record.collection().clone();
    for (i, field) in collection.fields.iter().enumerate() {
        // The primary key is handled by the write path, never cast.
        if field.is_primary_key() {
            continue;
        }
        let data = record.data_mut();
        // Fields and value slots share an order (see `Record::new`), so
        // this is an index rather than a hash lookup — verified, because
        // duplicate field names collapse into one slot.
        let slot = if crate::records::aligned(data, i, &field.name) {
            data.get_index_mut(i).map(|(_, v)| v)
        } else {
            data.get_mut(&field.name)
        };
        let Some(slot) = slot else { continue };
        let next = coerce_changed(field, slot);
        if let Some(next) = next {
            *slot = next;
        }
    }
}

/// Validate every field of `record`. `uploads` describes the files the
/// caller is about to store for this record (empty for a plain JSON
/// request).
///
/// `autodate` fields are skipped: their value is always server-computed,
/// never client-supplied.
pub async fn record(
    ex: &dyn Executor,
    store: &CollectionStore,
    record: &Record,
    uploads: &[UploadMeta],
) -> DbResult<Errors> {
    let collection = record.collection().clone();
    let mut errors = Errors::new();
    for field in &collection.fields {
        if field.field_type() == FieldType::Autodate {
            continue;
        }
        let value = record.get(&field.name).cloned().unwrap_or(Value::Null);

        // `id` is validated by `records::create` (which also checks
        // uniqueness); on update it is never writable.
        if field.is_primary_key() {
            continue;
        }
        if let Some(e) = required(field, &value) {
            errors.insert(field.name.clone(), e);
            continue;
        }
        // A blank optional value passes everything else, except files
        // whose pending uploads still need checking.
        if is_blank(&value) && field.field_type() != FieldType::File {
            continue;
        }
        let failure = match field.field_type() {
            FieldType::Text | FieldType::Editor => text(field, &value),
            FieldType::Number => number(field, &value),
            FieldType::Bool => boolean(&value),
            FieldType::Email => email(field, &value),
            FieldType::Url => url(field, &value),
            FieldType::Date => date(field, &value),
            FieldType::Select => select(field, &value),
            FieldType::File => file(field, &value, uploads),
            FieldType::Json => json(field, &value),
            FieldType::Password => {
                // Already-hashed values come from storage, not the client.
                if crate::records::is_hash(&value) {
                    None
                } else {
                    password(field, &value)
                }
            }
            FieldType::GeoPoint => geo_point(&value),
            FieldType::Relation => relation(ex, store, field, &value).await?,
            FieldType::Vector => vector(field, &value),
            FieldType::Autodate => None,
        };
        if let Some(e) = failure {
            errors.insert(field.name.clone(), e);
        }
    }
    Ok(errors)
}

/// [`record`], turned into the `Err` the write path returns.
pub async fn check(
    ex: &dyn Executor,
    store: &CollectionStore,
    rec: &Record,
    uploads: &[UploadMeta],
) -> DbResult<()> {
    let errors = record(ex, store, rec, uploads).await?;
    if errors.is_empty() {
        Ok(())
    } else {
        Err(DbError::Validation(errors))
    }
}

/// PocketBase's append/prepend/remove modifiers on multi-valued
/// `select` / `relation` / `file` fields:
///
/// * `"tags+": ["c"]` appends,
/// * `"+tags": ["a"]` prepends,
/// * `"tags-": ["b"]` removes.
///
/// The base is the plain key when the body also carries it, otherwise the
/// record's current value. The modifier keys are consumed, leaving a
/// plain `tags` entry the normal write path understands.
pub fn apply_modifiers(
    previous: Option<&Record>,
    input: &mut Map<String, Value>,
    collection: &Collection,
) {
    for field in &collection.fields {
        if !field.is_multiple() {
            continue;
        }
        let name = &field.name;
        let append_key = format!("{name}+");
        let prepend_key = format!("+{name}");
        let remove_key = format!("{name}-");
        let has_modifier = input.contains_key(&append_key)
            || input.contains_key(&prepend_key)
            || input.contains_key(&remove_key);
        if !has_modifier {
            continue;
        }

        let base = input
            .get(name)
            .cloned()
            .or_else(|| previous.and_then(|r| r.get(name).cloned()))
            .unwrap_or(Value::Null);
        let mut values = string_list(&base);

        if let Some(v) = input.remove(&prepend_key) {
            let mut head = string_list(&v);
            head.extend(values);
            values = head;
        }
        if let Some(v) = input.remove(&append_key) {
            values.extend(string_list(&v));
        }
        if let Some(v) = input.remove(&remove_key) {
            let drop: HashSet<String> = string_list(&v).into_iter().collect();
            values.retain(|x| !drop.contains(x));
        }

        input.insert(
            name.clone(),
            Value::Array(values.into_iter().map(Value::String).collect()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cratebase_core::{CollectionType, Field, FieldKind};
    use serde_json::json;
    use std::sync::Arc;

    fn f(name: &str, kind: FieldKind) -> Field {
        Field::new(name, kind)
    }

    #[test]
    fn text_constraints() {
        let mut field = f(
            "title",
            FieldKind::Text {
                min: 3,
                max: 5,
                pattern: "^[a-z]+$".into(),
                autogenerate_pattern: String::new(),
                primary_key: false,
            },
        );
        assert!(text(&field, &json!("abc")).is_none());
        assert_eq!(text(&field, &json!("ab")).unwrap().code, codes::MIN_TEXT);
        assert_eq!(
            text(&field, &json!("abcdef")).unwrap().code,
            codes::MAX_TEXT
        );
        assert_eq!(
            text(&field, &json!("ABC")).unwrap().code,
            codes::PATTERN_MISMATCH
        );
        assert_eq!(text(&field, &json!(1)).unwrap().code, codes::INVALID_FORMAT);
        field.required = true;
        assert_eq!(required(&field, &json!("")).unwrap().code, codes::REQUIRED);
        assert!(required(&field, &json!("x")).is_none());
    }

    #[test]
    fn number_bool_json_geo() {
        let field = f(
            "views",
            FieldKind::Number {
                min: Some(1.0),
                max: Some(10.0),
                only_int: true,
            },
        );
        assert!(number(&field, &json!(5)).is_none());
        let decimal = number(&field, &json!(0.5)).unwrap();
        assert_eq!(decimal.code, codes::ONLY_INT);
        assert_eq!(decimal.message, "Decimal numbers are not allowed.");
        assert_eq!(number(&field, &json!(0)).unwrap().code, codes::MIN_NUMBER);
        assert_eq!(number(&field, &json!(11)).unwrap().code, codes::MAX_NUMBER);
        assert_eq!(
            number(&field, &json!("x")).unwrap().code,
            codes::INVALID_NUMBER
        );

        assert!(boolean(&json!(true)).is_none());
        assert_eq!(boolean(&json!("y")).unwrap().code, codes::INVALID_BOOL);

        let field = f("data", FieldKind::Json { max_size: 12 });
        assert!(json(&field, &json!({"a": 1})).is_none());
        assert_eq!(
            json(&field, &json!("{oops")).unwrap().code,
            codes::INVALID_JSON
        );
        assert_eq!(
            json(&field, &json!({"long": "value"})).unwrap().code,
            JSON_SIZE_LIMIT
        );

        assert!(geo_point(&json!({"lon": 1.0, "lat": 2.0})).is_none());
        assert_eq!(
            geo_point(&json!({"lon": 200.0, "lat": 2.0})).unwrap().code,
            codes::INVALID_FORMAT
        );
        assert_eq!(geo_point(&json!("x")).unwrap().code, codes::INVALID_FORMAT);
    }

    #[test]
    fn email_url_date_select_file() {
        let field = f(
            "email",
            FieldKind::Email {
                except_domains: vec!["bad.com".into()],
                only_domains: vec![],
            },
        );
        assert!(email(&field, &json!("a@b.co")).is_none());
        assert_eq!(
            email(&field, &json!("nope")).unwrap().code,
            codes::INVALID_EMAIL
        );
        assert_eq!(
            email(&field, &json!("a@bad.com")).unwrap().code,
            codes::NOT_ALLOWED_DOMAIN
        );

        let field = f(
            "site",
            FieldKind::Url {
                except_domains: vec![],
                only_domains: vec!["ok.com".into()],
            },
        );
        assert!(url(&field, &json!("https://ok.com/x")).is_none());
        assert_eq!(
            url(&field, &json!("ftp://ok.com")).unwrap().code,
            INVALID_URL
        );
        assert_eq!(
            url(&field, &json!("https://other.com")).unwrap().code,
            codes::NOT_ALLOWED_DOMAIN
        );

        let field = f(
            "when",
            FieldKind::Date {
                min: DateTime::parse("2026-01-01 00:00:00.000Z"),
                max: DateTime::parse("2026-12-31 00:00:00.000Z"),
            },
        );
        assert!(date(&field, &json!("2026-06-01 00:00:00.000Z")).is_none());
        assert_eq!(
            date(&field, &json!("nope")).unwrap().code,
            codes::INVALID_DATE
        );
        let too_early = date(&field, &json!("2025-06-01 00:00:00.000Z")).unwrap();
        assert_eq!(too_early.code, MIN_DATE);
        assert_eq!(
            too_early.message,
            "Must be no less than 2026-01-01 00:00:00 +0000 UTC."
        );
        // PocketBase carries the bound machine-readably alongside the
        // prose, RFC3339 with trailing zero fractions trimmed (Go's
        // time.Time JSON form), asserted in
        // tests/conformance/records.test.ts.
        assert_eq!(
            too_early.params.as_ref().unwrap()["threshold"],
            json!("2026-01-01T00:00:00Z")
        );
        assert_eq!(
            date(&field, &json!("2027-06-01 00:00:00.000Z"))
                .unwrap()
                .code,
            MAX_DATE
        );

        let field = f(
            "tags",
            FieldKind::Select {
                values: vec!["a".into(), "b".into()],
                max_select: 2,
            },
        );
        assert!(select(&field, &json!(["a", "b"])).is_none());
        assert_eq!(
            select(&field, &json!(["a", "b", "a"])).unwrap().code,
            codes::MAX_SELECT
        );
        assert_eq!(
            select(&field, &json!(["z"])).unwrap().code,
            codes::NOT_IN_LIST
        );

        let field = f(
            "docs",
            FieldKind::File {
                max_select: 2,
                max_size: 10,
                mime_types: vec!["image/png".into()],
                thumbs: vec![],
                protected: false,
            },
        );
        let ok = UploadMeta {
            field: "docs".into(),
            name: "a.png".into(),
            size: 5,
            mime: "image/png".into(),
        };
        assert!(file(&field, &json!([]), std::slice::from_ref(&ok)).is_none());
        let big = UploadMeta {
            size: 50,
            ..ok.clone()
        };
        assert_eq!(
            file(&field, &json!([]), &[big]).unwrap().code,
            codes::FILE_TOO_LARGE
        );
        let wrong = UploadMeta {
            mime: "text/plain".into(),
            ..ok.clone()
        };
        assert_eq!(
            file(&field, &json!([]), &[wrong]).unwrap().code,
            codes::FILE_MIME
        );
        assert_eq!(
            file(&field, &json!(["x", "y"]), &[ok]).unwrap().code,
            TOO_MANY_FILES
        );
    }

    #[test]
    fn password_uses_text_codes() {
        let field = f(
            "password",
            FieldKind::Password {
                min: 8,
                max: 10,
                pattern: String::new(),
                cost: 0,
            },
        );
        assert!(password(&field, &json!("longenough")).is_none());
        assert_eq!(
            password(&field, &json!("short")).unwrap().code,
            codes::MIN_TEXT
        );
        assert_eq!(
            password(&field, &json!("waaaaaaaaytoolong")).unwrap().code,
            codes::MAX_TEXT
        );
    }

    #[test]
    fn modifiers_append_prepend_and_remove() {
        let mut c = cratebase_core::Collection::new("posts", CollectionType::Base);
        let pos = c.fields.len() - 2;
        c.fields.insert(
            pos,
            f(
                "tags",
                FieldKind::Select {
                    values: vec!["a".into(), "b".into(), "c".into()],
                    max_select: 5,
                },
            ),
        );
        let c = Arc::new(c);
        let mut previous = Record::new(c.clone());
        previous.set("tags", json!(["a", "b"]));

        let mut input = json!({"tags+": ["c"], "+tags": ["z"], "tags-": ["a"]})
            .as_object()
            .cloned()
            .unwrap();
        apply_modifiers(Some(&previous), &mut input, &c);
        assert_eq!(input["tags"], json!(["z", "b", "c"]));
        assert_eq!(input.len(), 1);

        // No modifier keys: the map is untouched.
        let mut input = json!({"tags": ["a"]}).as_object().cloned().unwrap();
        apply_modifiers(Some(&previous), &mut input, &c);
        assert_eq!(input["tags"], json!(["a"]));

        // Without a previous record the base is empty.
        let mut input = json!({"tags+": ["a"]}).as_object().cloned().unwrap();
        apply_modifiers(None, &mut input, &c);
        assert_eq!(input["tags"], json!(["a"]));
    }

    #[test]
    fn coercion_matches_pocketbase_casts() {
        let views = f(
            "views",
            FieldKind::Number {
                min: None,
                max: None,
                only_int: false,
            },
        );
        assert_eq!(coerce(&views, &json!("42")), json!(42.0));
        assert_eq!(coerce(&views, &json!("abc")), json!(0.0));

        let published = f("published", FieldKind::Bool {});
        assert_eq!(coerce(&published, &json!("true")), json!(true));
        assert_eq!(coerce(&published, &json!("yes")), json!(false));

        let when = f(
            "when",
            FieldKind::Date {
                min: None,
                max: None,
            },
        );
        assert_eq!(coerce(&when, &json!("not a date")), json!(""));
        assert_eq!(
            coerce(&when, &json!("2024-05-05")),
            json!("2024-05-05 00:00:00.000Z")
        );
        assert_eq!(
            coerce(&when, &json!("2024-05-05T12:30:00+02:00")),
            json!("2024-05-05 10:30:00.000Z")
        );

        let category = f(
            "category",
            FieldKind::Select {
                values: vec!["news".into(), "blog".into()],
                max_select: 1,
            },
        );
        assert_eq!(coerce(&category, &json!(["news", "blog"])), json!("blog"));

        let tags = f(
            "tags",
            FieldKind::Select {
                values: vec!["go".into()],
                max_select: 3,
            },
        );
        assert_eq!(coerce(&tags, &json!("go")), json!(["go"]));
        assert_eq!(coerce(&tags, &Value::Null), json!([]));

        let meta = f("meta", FieldKind::Json { max_size: 0 });
        assert_eq!(coerce(&meta, &json!("{\"x\":1}")), json!({"x": 1}));

        let title = f("title", FieldKind::default_for(FieldType::Text));
        assert_eq!(coerce(&title, &json!(5)), json!("5"));
        assert_eq!(coerce(&title, &Value::Null), json!(""));
    }

    #[test]
    fn blankness_matches_ozzo_required() {
        assert!(is_blank(&Value::Null));
        assert!(is_blank(&json!("")));
        assert!(is_blank(&json!([])));
        assert!(is_blank(&json!(0)));
        assert!(is_blank(&json!(false)));
        assert!(!is_blank(&json!("x")));
        assert!(!is_blank(&json!(1)));
        assert!(!is_blank(&json!(true)));
    }
}
