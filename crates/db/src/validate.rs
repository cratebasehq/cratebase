use std::collections::HashMap;

use cratebase_core::field::{Field, FieldType};
use cratebase_core::{Collection, RESERVED_FIELD_NAMES};
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

/// Validate one field's value against its schema definition. Returns a
/// human-readable error string on failure.
fn validate_field(field: &Field, value: &Value) -> Result<(), String> {
    if value.is_null() {
        return if field.required {
            Err("value is required".to_string())
        } else {
            Ok(())
        };
    }

    let multiple = is_multiple(field);

    match field.field_type {
        FieldType::Number => {
            let n = value.as_f64().ok_or("expected a number")?;
            if field.options.only_int.unwrap_or(false) && n.fract() != 0.0 {
                return Err("expected an integer".to_string());
            }
            if let Some(min) = field.options.min {
                if n < min {
                    return Err(format!("must be >= {min}"));
                }
            }
            if let Some(max) = field.options.max {
                if n > max {
                    return Err(format!("must be <= {max}"));
                }
            }
        }
        FieldType::Bool => {
            value.as_bool().ok_or("expected a boolean")?;
        }
        FieldType::Json => {}
        FieldType::Email => {
            let s = value.as_str().ok_or("expected a string")?;
            if !looks_like_email(s) {
                return Err("not a valid email address".to_string());
            }
        }
        FieldType::Url => {
            let s = value.as_str().ok_or("expected a string")?;
            if !looks_like_url(s) {
                return Err("not a valid url".to_string());
            }
        }
        FieldType::Date => {
            let s = value.as_str().ok_or("expected a string")?;
            if chrono::DateTime::parse_from_rfc3339(s).is_err() {
                return Err("expected an RFC3339 date-time string".to_string());
            }
        }
        FieldType::Text | FieldType::Editor | FieldType::Password => {
            let s = value.as_str().ok_or("expected a string")?;
            if let Some(min) = field.options.min {
                if (s.chars().count() as f64) < min {
                    return Err(format!("must be at least {min} characters"));
                }
            }
            if let Some(max) = field.options.max {
                if (s.chars().count() as f64) > max {
                    return Err(format!("must be at most {max} characters"));
                }
            }
            if let Some(pattern) = &field.options.pattern {
                let re = regex::Regex::new(pattern).map_err(|e| format!("invalid pattern: {e}"))?;
                if !re.is_match(s) {
                    return Err("does not match the required pattern".to_string());
                }
            }
        }
        FieldType::Select => {
            let values =
                as_string_list(value, multiple).ok_or("expected a string or array of strings")?;
            let allowed = field.options.values.clone().unwrap_or_default();
            for v in &values {
                if !allowed.contains(v) {
                    return Err(format!("'{v}' is not one of the allowed values"));
                }
            }
            if multiple {
                if let Some(max) = field.options.max_select {
                    if values.len() > max as usize {
                        return Err(format!("at most {max} values allowed"));
                    }
                }
            }
        }
        FieldType::Relation | FieldType::File => {
            as_string_list(value, multiple).ok_or("expected a string or array of strings")?;
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
        let provided = data.get(&field.name);
        match provided {
            Some(value) => {
                if let Err(msg) = validate_field(field, value) {
                    errors.insert(field.name.clone(), msg);
                } else {
                    normalized.insert(field.name.clone(), value.clone());
                }
            }
            None if !partial && field.required => {
                errors.insert(field.name.clone(), "value is required".to_string());
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
                    "relation target collection no longer exists".into(),
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
                    format!("related record '{id}' does not exist"),
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
