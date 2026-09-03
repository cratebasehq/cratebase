use serde::{Deserialize, Serialize};

/// The primitive type of a collection field. Each variant maps to a concrete
/// SQL column type in the storage backend (see `cratebase-db`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Text,
    Editor,
    Number,
    Bool,
    Email,
    Url,
    Date,
    Select,
    Json,
    Relation,
    File,
    Password,
}

impl FieldType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FieldType::Text => "text",
            FieldType::Editor => "editor",
            FieldType::Number => "number",
            FieldType::Bool => "bool",
            FieldType::Email => "email",
            FieldType::Url => "url",
            FieldType::Date => "date",
            FieldType::Select => "select",
            FieldType::Json => "json",
            FieldType::Relation => "relation",
            FieldType::File => "file",
            FieldType::Password => "password",
        }
    }

    /// Whether the field can natively hold multiple values (arrays).
    /// Controlled per-field via `FieldOptions::max_select` / `multiple`.
    pub fn supports_multiple(&self) -> bool {
        matches!(
            self,
            FieldType::Select | FieldType::Relation | FieldType::File
        )
    }
}

/// Extra, type-specific configuration for a [`Field`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FieldOptions {
    // text / editor
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub pattern: Option<String>,
    // select
    pub values: Option<Vec<String>>,
    // relation
    pub collection_id: Option<String>,
    // relation / select / file: allow storing more than one value
    pub multiple: Option<bool>,
    // file
    pub mime_types: Option<Vec<String>>,
    pub max_select: Option<u32>,
    pub max_size: Option<u64>,
    // number
    pub only_int: Option<bool>,
}

/// A single field (column) definition inside a [`crate::Collection`] schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub options: FieldOptions,
}

/// Field/collection names reserved by the system and unavailable to users.
pub const RESERVED_FIELD_NAMES: &[&str] = &[
    "id",
    "created",
    "updated",
    "collectionId",
    "collectionName",
    "expand",
];

pub fn is_valid_identifier(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
