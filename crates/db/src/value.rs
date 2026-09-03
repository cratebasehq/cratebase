use cratebase_core::field::FieldType;
use serde_json::Value;
use sqlx::any::AnyArguments;
use sqlx::Arguments;

/// A field value narrowed to one of the three physical SQL storage shapes
/// Cratebase uses (see `Backend::{text,number,bool}_type`). Each variant
/// carries its own `Option` so a NULL is bound with the *matching* SQL
/// type — binding e.g. `None::<String>` (TEXT) into a `DOUBLE PRECISION`
/// column makes Postgres reject the statement (`column "x" is of type
/// double precision but expression is of type text`), even though the
/// value itself is NULL. Arrays (multi-select / multi-relation / multi-file)
/// and JSON fields are encoded as their JSON text representation.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnValue {
    Text(Option<String>),
    Number(Option<f64>),
    Bool(Option<bool>),
}

impl ColumnValue {
    /// Convert a JSON value coming from an API request body into the
    /// physical representation for `field_type`. Assumes `value` has
    /// already passed `cratebase-db::validate` (so a non-null value is
    /// guaranteed to have the right JSON shape for `field_type`).
    pub fn from_json(field_type: FieldType, multiple: bool, value: &Value) -> ColumnValue {
        if multiple {
            // Multi-valued select/relation/file fields are always stored as
            // a JSON array string regardless of the declared field type.
            return ColumnValue::Text(if value.is_null() {
                None
            } else {
                Some(value.to_string())
            });
        }
        match field_type {
            FieldType::Number => ColumnValue::Number(value.as_f64()),
            FieldType::Bool => ColumnValue::Bool(value.as_bool()),
            FieldType::Json => ColumnValue::Text(if value.is_null() {
                None
            } else {
                Some(value.to_string())
            }),
            _ => ColumnValue::Text(match value {
                Value::Null => None,
                Value::String(s) => Some(s.clone()),
                other => Some(other.to_string()),
            }),
        }
    }

    /// Convert a stored column value back into a JSON value for API
    /// responses.
    pub fn to_json(&self, field_type: FieldType, multiple: bool) -> Value {
        match self {
            ColumnValue::Number(n) => n
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            ColumnValue::Bool(b) => b.map(Value::Bool).unwrap_or(Value::Null),
            ColumnValue::Text(None) => Value::Null,
            ColumnValue::Text(Some(s)) => {
                if multiple || matches!(field_type, FieldType::Json) {
                    serde_json::from_str(s).unwrap_or(Value::String(s.clone()))
                } else {
                    Value::String(s.clone())
                }
            }
        }
    }

    pub fn bind<'q>(self, args: &mut AnyArguments<'q>) -> Result<(), sqlx::error::BoxDynError> {
        match self {
            ColumnValue::Text(v) => args.add(v),
            ColumnValue::Number(v) => args.add(v),
            // Stored as INTEGER (see `Backend::bool_type`): sqlx's `Any`
            // driver can't decode SQLite's native BOOLEAN column type, and
            // Postgres has no implicit bool<->integer cast, so 0/1 is the
            // one representation that round-trips on both backends.
            ColumnValue::Bool(v) => args.add(v.map(|b| b as i64)),
        }
    }
}

/// Bind an arbitrary JSON scalar produced by the filter compiler (context
/// values like `@request.auth.id`, or literals from the filter string) as a
/// query parameter. Unlike [`ColumnValue::from_json`] this has no target
/// field type to narrow against, so it infers the SQL shape directly from
/// the JSON value's own type. Comparisons against a literal `null` are
/// compiled to `IS [NOT] NULL` upstream (see `cratebase_filter::compile`)
/// and never reach here, so the only remaining nulls are context values
/// (e.g. an unauthenticated `@request.auth.id`), which are always compared
/// against TEXT id columns in practice.
pub fn bind_filter_value<'q>(
    args: &mut AnyArguments<'q>,
    value: Value,
) -> Result<(), sqlx::error::BoxDynError> {
    match value {
        Value::Null => args.add(None::<String>),
        // See `ColumnValue::bind`: bool fields are physically INTEGER.
        Value::Bool(b) => args.add(b as i64),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                args.add(i as f64)
            } else {
                args.add(n.as_f64().unwrap_or_default())
            }
        }
        Value::String(s) => args.add(s),
        other => args.add(other.to_string()),
    }
}
