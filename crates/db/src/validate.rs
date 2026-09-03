use std::collections::HashMap;

use cratebase_core::field::{Field, FieldType};
use cratebase_core::{Collection, FieldError, RESERVED_FIELD_NAMES};
use serde_json::{Map, Value};
use sqlx::Row;

use crate::collections::get_collection_by_id;
use crate::error::{DbError, DbResult};
use crate::pool::Db;

fn is_multiple(field: &Field) -> bool {
    field.field_type.supports_multiple() && field.options.multiple.unwrap_or(false)
}

fn as_string_list(value: &Value, multiple: bool) -> Option<Vec<String>> {
    if multiple {
        value
            .as_array()?
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect()
    } else {
        value.as_str().map(|s| vec![s.to_string()])
    }
}

fn looks_like_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

fn looks_like_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn not_a_string() -> FieldError {
    FieldError::new("invalid_type", "expected a string")
}

/// Validate one field's value against its schema definition. Returns a
/// structured `{code, message}` error on failure so callers (an SDK, a
/// form library) can branch on `code` instead of string-matching prose.
fn validate_field(field: &Field, value: &Value) -> Result<(), FieldError> {
    if value.is_null() {
        return if field.required {
            Err(FieldError::new("value_required", "value is required"))
        } else {
            Ok(())
        };
    }

    let multiple = is_multiple(field);

    match field.field_type {
        FieldType::Number => {
            let n = value
                .as_f64()
                .ok_or_else(|| FieldError::new("invalid_number", "expected a number"))?;
            if field.options.only_int.unwrap_or(false) && n.fract() != 0.0 {
                return Err(FieldError::new("invalid_integer", "expected an integer"));
            }
            if let Some(min) = field.options.min {
                if n < min {
                    return Err(FieldError::new(
                        "value_too_small",
                        format!("must be >= {min}"),
                    ));
                }
            }
            if let Some(max) = field.options.max {
                if n > max {
                    return Err(FieldError::new(
                        "value_too_large",
                        format!("must be <= {max}"),
                    ));
                }
            }
        }
        FieldType::Bool => {
            value
                .as_bool()
                .ok_or_else(|| FieldError::new("invalid_bool", "expected a boolean"))?;
        }
        FieldType::Json => {}
        FieldType::Email => {
            let s = value.as_str().ok_or_else(not_a_string)?;
            if !looks_like_email(s) {
                return Err(FieldError::new(
                    "invalid_email",
                    "not a valid email address",
                ));
            }
        }
        FieldType::Url => {
            let s = value.as_str().ok_or_else(not_a_string)?;
            if !looks_like_url(s) {
                return Err(FieldError::new("invalid_url", "not a valid url"));
            }
        }
        FieldType::Date => {
            let s = value.as_str().ok_or_else(not_a_string)?;
            if chrono::DateTime::parse_from_rfc3339(s).is_err() {
                return Err(FieldError::new(
                    "invalid_date",
                    "expected an RFC3339 date-time string",
                ));
            }
        }
        // Never reached in practice: `validate_and_normalize` skips
        // `Autodate` fields entirely before calling this function, since
        // their value is always server-computed, never client-supplied.
        FieldType::Autodate => {}
        FieldType::Text | FieldType::Editor | FieldType::Password => {
            let s = value.as_str().ok_or_else(not_a_string)?;
            if let Some(min) = field.options.min {
                if (s.chars().count() as f64) < min {
                    return Err(FieldError::new(
                        "value_too_short",
                        format!("must be at least {min} characters"),
                    ));
                }
            }
            if let Some(max) = field.options.max {
                if (s.chars().count() as f64) > max {
                    return Err(FieldError::new(
                        "value_too_long",
                        format!("must be at most {max} characters"),
                    ));
                }
            }
            if let Some(pattern) = &field.options.pattern {
                let re = regex::Regex::new(pattern).map_err(|e| {
                    FieldError::new(
                        "invalid_pattern_definition",
                        format!("invalid pattern: {e}"),
                    )
                })?;
                if !re.is_match(s) {
                    return Err(FieldError::new(
                        "pattern_mismatch",
                        "does not match the required pattern",
                    ));
                }
            }
        }
        FieldType::Select => {
            let values = as_string_list(value, multiple).ok_or_else(|| {
                FieldError::new("invalid_type", "expected a string or array of strings")
            })?;
            let allowed = field.options.values.clone().unwrap_or_default();
            for v in &values {
                if !allowed.contains(v) {
                    return Err(FieldError::new(
                        "value_not_allowed",
                        format!("'{v}' is not one of the allowed values"),
                    ));
                }
            }
            if multiple {
                if let Some(max) = field.options.max_select {
                    if values.len() > max as usize {
                        return Err(FieldError::new(
                            "too_many_values",
                            format!("at most {max} values allowed"),
                        ));
                    }
                }
            }
        }
        FieldType::Relation | FieldType::File => {
            as_string_list(value, multiple).ok_or_else(|| {
                FieldError::new("invalid_type", "expected a string or array of strings")
            })?;
        }
    }
    Ok(())
}

/// Validate a create/update payload against `collection.schema`, verify
/// relation targets actually exist, and drop any keys that don't correspond
/// to a schema field or reserved system column. On update (`partial =
/// true`) only the supplied fields are checked; missing required fields are
/// not an error since the existing stored value is kept.
pub async fn validate_and_normalize(
    db: &Db,
    collection: &Collection,
    data: &Map<String, Value>,
    partial: bool,
) -> DbResult<Map<String, Value>> {
    let mut errors = HashMap::new();
    let mut normalized = Map::new();

    for field in &collection.schema {
        if field.field_type == FieldType::Autodate {
            // Server-computed on create/update (see `records::apply_autodate_fields`);
            // client-supplied values for this field are always ignored.
            continue;
        }
        let provided = data.get(&field.name);
        match provided {
            Some(value) => {
                if let Err(err) = validate_field(field, value) {
                    errors.insert(field.name.clone(), err);
                } else {
                    normalized.insert(field.name.clone(), value.clone());
                }
            }
            None if !partial && field.required => {
                errors.insert(
                    field.name.clone(),
                    FieldError::new("value_required", "value is required"),
                );
            }
            None => {}
        }
    }

    let _ = RESERVED_FIELD_NAMES; // reserved names are simply never schema fields

    if !errors.is_empty() {
        return Err(DbError::Validation(errors));
    }

    validate_relations(db, collection, &normalized).await?;

    Ok(normalized)
}

/// Transaction-scoped counterpart to [`validate_and_normalize`]. See
/// [`crate::collections::get_collection_by_id_tx`] for why this exists.
pub async fn validate_and_normalize_tx(
    tx: &mut crate::records::RecordTx,
    backend: crate::backend::Backend,
    collection: &Collection,
    data: &Map<String, Value>,
    partial: bool,
) -> DbResult<Map<String, Value>> {
    let mut errors = HashMap::new();
    let mut normalized = Map::new();

    for field in &collection.schema {
        if field.field_type == FieldType::Autodate {
            continue;
        }
        let provided = data.get(&field.name);
        match provided {
            Some(value) => {
                if let Err(err) = validate_field(field, value) {
                    errors.insert(field.name.clone(), err);
                } else {
                    normalized.insert(field.name.clone(), value.clone());
                }
            }
            None if !partial && field.required => {
                errors.insert(
                    field.name.clone(),
                    FieldError::new("value_required", "value is required"),
                );
            }
            None => {}
        }
    }

    if !errors.is_empty() {
        return Err(DbError::Validation(errors));
    }

    validate_relations_tx(tx, backend, collection, &normalized).await?;

    Ok(normalized)
}

async fn validate_relations_tx(
    tx: &mut crate::records::RecordTx,
    backend: crate::backend::Backend,
    collection: &Collection,
    data: &Map<String, Value>,
) -> DbResult<()> {
    let mut errors = HashMap::new();

    for field in &collection.schema {
        if field.field_type != FieldType::Relation {
            continue;
        }
        let Some(value) = data.get(&field.name) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let Some(target_id) = &field.options.collection_id else {
            continue;
        };
        let multiple = is_multiple(field);
        let ids = match as_string_list(value, multiple) {
            Some(ids) => ids,
            None => continue,
        };
        if ids.is_empty() {
            continue;
        }

        let target = match crate::collections::get_collection_by_id_tx(tx, target_id).await {
            Ok(c) => c,
            Err(_) => {
                errors.insert(
                    field.name.clone(),
                    FieldError::new(
                        "relation_target_missing",
                        "relation target collection no longer exists",
                    ),
                );
                continue;
            }
        };
        let table = backend.quote_ident(&target.table_name())?;
        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${i}")).collect();
        let sql = format!(
            "SELECT {} FROM {table} WHERE {} IN ({})",
            backend.quote_ident("id")?,
            backend.quote_ident("id")?,
            placeholders.join(", ")
        );
        let mut q = sqlx::query(&sql);
        for id in &ids {
            q = q.bind(id);
        }
        let rows = q.fetch_all(&mut **tx).await?;
        let found: std::collections::HashSet<String> = rows
            .iter()
            .filter_map(|r| r.try_get::<String, _>(0).ok())
            .collect();
        for id in &ids {
            if !found.contains(id) {
                errors.insert(
                    field.name.clone(),
                    FieldError::new(
                        "relation_not_found",
                        format!("related record '{id}' does not exist"),
                    ),
                );
                break;
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(DbError::Validation(errors))
    }
}

async fn validate_relations(
    db: &Db,
    collection: &Collection,
    data: &Map<String, Value>,
) -> DbResult<()> {
    let mut errors = HashMap::new();

    for field in &collection.schema {
        if field.field_type != FieldType::Relation {
            continue;
        }
        let Some(value) = data.get(&field.name) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let Some(target_id) = &field.options.collection_id else {
            continue;
        };
        let multiple = is_multiple(field);
        let ids = match as_string_list(value, multiple) {
            Some(ids) => ids,
            None => continue,
        };
        if ids.is_empty() {
            continue;
        }

        let target = match get_collection_by_id(db, target_id).await {
            Ok(c) => c,
            Err(_) => {
                errors.insert(
                    field.name.clone(),
                    FieldError::new(
                        "relation_target_missing",
                        "relation target collection no longer exists",
                    ),
                );
                continue;
            }
        };
        let table = db.backend.quote_ident(&target.table_name())?;
        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${i}")).collect();
        let sql = format!(
            "SELECT {} FROM {table} WHERE {} IN ({})",
            db.backend.quote_ident("id")?,
            db.backend.quote_ident("id")?,
            placeholders.join(", ")
        );
        let mut q = sqlx::query(&sql);
        for id in &ids {
            q = q.bind(id);
        }
        let rows = q.fetch_all(&db.pool).await?;
        let found: std::collections::HashSet<String> = rows
            .iter()
            .filter_map(|r| r.try_get::<String, _>(0).ok())
            .collect();
        for id in &ids {
            if !found.contains(id) {
                errors.insert(
                    field.name.clone(),
                    FieldError::new(
                        "relation_not_found",
                        format!("related record '{id}' does not exist"),
                    ),
                );
                break;
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(DbError::Validation(errors))
    }
}
