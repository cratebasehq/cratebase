//! Collection field definitions, shaped exactly like PocketBase v0.23+'s
//! `fields[]` entries: every type-specific option lives flat on the
//! field object next to the common attributes, e.g.
//!
//! ```json
//! {"id":"text724990059","name":"title","type":"text","system":false,
//!  "hidden":false,"presentable":false,"required":true,"help":"",
//!  "min":0,"max":0,"pattern":"","autogeneratePattern":"","primaryKey":false}
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::datetime::DateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldType {
    Text,
    Editor,
    Number,
    Bool,
    Email,
    Url,
    Date,
    Autodate,
    Select,
    File,
    Relation,
    Json,
    Password,
    GeoPoint,
    Vector,
}

impl FieldType {
    pub fn as_str(self) -> &'static str {
        match self {
            FieldType::Text => "text",
            FieldType::Editor => "editor",
            FieldType::Number => "number",
            FieldType::Bool => "bool",
            FieldType::Email => "email",
            FieldType::Url => "url",
            FieldType::Date => "date",
            FieldType::Autodate => "autodate",
            FieldType::Select => "select",
            FieldType::File => "file",
            FieldType::Relation => "relation",
            FieldType::Json => "json",
            FieldType::Password => "password",
            FieldType::GeoPoint => "geoPoint",
            FieldType::Vector => "vector",
        }
    }

    pub fn all() -> &'static [FieldType] {
        &[
            FieldType::Text,
            FieldType::Editor,
            FieldType::Number,
            FieldType::Bool,
            FieldType::Email,
            FieldType::Url,
            FieldType::Date,
            FieldType::Autodate,
            FieldType::Select,
            FieldType::File,
            FieldType::Relation,
            FieldType::Json,
            FieldType::Password,
            FieldType::GeoPoint,
            FieldType::Vector,
        ]
    }
}

/// Type-specific options. Internally tagged on `type` and flattened into
/// [`Field`], which produces PocketBase's flat layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FieldKind {
    #[serde(rename_all = "camelCase")]
    Text {
        #[serde(default)]
        min: i64,
        #[serde(default)]
        max: i64,
        #[serde(default)]
        pattern: String,
        #[serde(default)]
        autogenerate_pattern: String,
        #[serde(default)]
        primary_key: bool,
    },
    #[serde(rename_all = "camelCase")]
    Editor {
        #[serde(default)]
        max_size: i64,
        #[serde(default, rename = "convertURLs")]
        convert_urls: bool,
    },
    #[serde(rename_all = "camelCase")]
    Number {
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        #[serde(default)]
        only_int: bool,
    },
    Bool {},
    #[serde(rename_all = "camelCase")]
    Email {
        #[serde(default)]
        except_domains: Vec<String>,
        #[serde(default)]
        only_domains: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    Url {
        #[serde(default)]
        except_domains: Vec<String>,
        #[serde(default)]
        only_domains: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    Date {
        #[serde(default, with = "empty_datetime")]
        min: Option<DateTime>,
        #[serde(default, with = "empty_datetime")]
        max: Option<DateTime>,
    },
    #[serde(rename_all = "camelCase")]
    Autodate {
        #[serde(default)]
        on_create: bool,
        #[serde(default)]
        on_update: bool,
    },
    #[serde(rename_all = "camelCase")]
    Select {
        #[serde(default)]
        values: Vec<String>,
        #[serde(default = "one")]
        max_select: i64,
    },
    #[serde(rename_all = "camelCase")]
    File {
        #[serde(default = "one")]
        max_select: i64,
        #[serde(default)]
        max_size: i64,
        #[serde(default)]
        mime_types: Vec<String>,
        #[serde(default)]
        thumbs: Vec<String>,
        #[serde(default)]
        protected: bool,
    },
    #[serde(rename_all = "camelCase")]
    Relation {
        #[serde(default)]
        collection_id: String,
        #[serde(default)]
        cascade_delete: bool,
        #[serde(default)]
        min_select: i64,
        #[serde(default = "one")]
        max_select: i64,
    },
    #[serde(rename_all = "camelCase")]
    Json {
        #[serde(default)]
        max_size: i64,
    },
    #[serde(rename_all = "camelCase")]
    Password {
        #[serde(default)]
        min: i64,
        #[serde(default)]
        max: i64,
        #[serde(default)]
        pattern: String,
        #[serde(default)]
        cost: i64,
    },
    GeoPoint {},
    /// A JSON array of exactly `dimensions` floats, application-side
    /// cosine similarity (no native ANN index in this pass — see
    /// `crates/server/src/embeddings.rs`). Either the caller supplies
    /// the array directly, or (when `embedding` is set) it is computed
    /// server-side from `embedding.source_field` on save.
    #[serde(rename_all = "camelCase")]
    Vector {
        #[serde(default)]
        dimensions: usize,
        #[serde(default)]
        embedding: Option<EmbeddingConfig>,
    },
}

/// Auto-embedding config for a `vector` field: instead of the caller
/// supplying the float array directly, it is computed server-side from
/// another field on the same record's current text every time that
/// source field's value changes (see
/// `crates/server/src/embeddings.rs::apply_embeddings`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EmbeddingConfig {
    /// Which embedding backend computes the vector: `"echo"` selects the
    /// deterministic, network-free test provider; anything else resolves
    /// to the configured HTTP provider (OpenAI-compatible `/embeddings`).
    pub provider: String,
    /// The model name passed to the HTTP provider (ignored by `"echo"`).
    pub model: String,
    /// The name of the text field on the same record whose value is
    /// embedded on save.
    pub source_field: String,
}

fn one() -> i64 {
    1
}

/// PocketBase serializes an unset date bound as `""`.
mod empty_datetime {
    use super::DateTime;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<DateTime>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(dt) => dt.serialize(s),
            None => s.serialize_str(""),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime>, D::Error> {
        let raw = Option::<String>::deserialize(d)?;
        Ok(raw.and_then(|s| DateTime::parse(&s)))
    }
}

impl FieldKind {
    pub fn field_type(&self) -> FieldType {
        match self {
            FieldKind::Text { .. } => FieldType::Text,
            FieldKind::Editor { .. } => FieldType::Editor,
            FieldKind::Number { .. } => FieldType::Number,
            FieldKind::Bool {} => FieldType::Bool,
            FieldKind::Email { .. } => FieldType::Email,
            FieldKind::Url { .. } => FieldType::Url,
            FieldKind::Date { .. } => FieldType::Date,
            FieldKind::Autodate { .. } => FieldType::Autodate,
            FieldKind::Select { .. } => FieldType::Select,
            FieldKind::File { .. } => FieldType::File,
            FieldKind::Relation { .. } => FieldType::Relation,
            FieldKind::Json { .. } => FieldType::Json,
            FieldKind::Password { .. } => FieldType::Password,
            FieldKind::GeoPoint {} => FieldType::GeoPoint,
            FieldKind::Vector { .. } => FieldType::Vector,
        }
    }

    /// The default options for a type, as the scaffold endpoint returns.
    pub fn default_for(t: FieldType) -> FieldKind {
        match t {
            FieldType::Text => FieldKind::Text {
                min: 0,
                max: 0,
                pattern: String::new(),
                autogenerate_pattern: String::new(),
                primary_key: false,
            },
            FieldType::Editor => FieldKind::Editor {
                max_size: 0,
                convert_urls: false,
            },
            FieldType::Number => FieldKind::Number {
                min: None,
                max: None,
                only_int: false,
            },
            FieldType::Bool => FieldKind::Bool {},
            FieldType::Email => FieldKind::Email {
                except_domains: vec![],
                only_domains: vec![],
            },
            FieldType::Url => FieldKind::Url {
                except_domains: vec![],
                only_domains: vec![],
            },
            FieldType::Date => FieldKind::Date {
                min: None,
                max: None,
            },
            FieldType::Autodate => FieldKind::Autodate {
                on_create: true,
                on_update: false,
            },
            FieldType::Select => FieldKind::Select {
                values: vec![],
                max_select: 1,
            },
            FieldType::File => FieldKind::File {
                max_select: 1,
                max_size: 0,
                mime_types: vec![],
                thumbs: vec![],
                protected: false,
            },
            FieldType::Relation => FieldKind::Relation {
                collection_id: String::new(),
                cascade_delete: false,
                min_select: 0,
                max_select: 1,
            },
            FieldType::Json => FieldKind::Json { max_size: 0 },
            FieldType::Password => FieldKind::Password {
                min: 0,
                max: 0,
                pattern: String::new(),
                cost: 0,
            },
            FieldType::GeoPoint => FieldKind::GeoPoint {},
            FieldType::Vector => FieldKind::Vector {
                dimensions: 0,
                embedding: None,
            },
        }
    }

    /// This field's shape as a JSON-Schema property — the piece
    /// [`crate::Collection::to_json_schema`] assembles into
    /// `parameters.properties`, and the same conversion both the MCP
    /// tool definitions (`crates/server/src/mcp.rs`) and the
    /// `/tool-schema` REST endpoint
    /// (`crates/server/src/routes/tool_schema.rs`) call, so the two
    /// surfaces can never drift apart. `help`, when non-empty, becomes
    /// the property's `description`; a handful of kinds fall back to a
    /// fixed note explaining a constraint the schema can't otherwise
    /// express (e.g. files can't be uploaded through a tool call).
    pub fn to_json_schema(&self, help: &str) -> Value {
        let (mut schema, default_note): (Value, &str) = match self {
            FieldKind::Text {
                min, max, pattern, ..
            } => {
                let mut s = json!({ "type": "string" });
                if *min > 0 {
                    s["minLength"] = json!(min);
                }
                if *max > 0 {
                    s["maxLength"] = json!(max);
                }
                if !pattern.is_empty() {
                    s["pattern"] = json!(pattern);
                }
                (s, "")
            }
            FieldKind::Editor { .. } => (json!({ "type": "string" }), "Rich text/HTML content."),
            FieldKind::Number { min, max, only_int } => {
                let mut s = json!({ "type": if *only_int { "integer" } else { "number" } });
                if let Some(min) = min {
                    s["minimum"] = json!(min);
                }
                if let Some(max) = max {
                    s["maximum"] = json!(max);
                }
                (s, "")
            }
            FieldKind::Bool {} => (json!({ "type": "boolean" }), ""),
            FieldKind::Email { .. } => (json!({ "type": "string", "format": "email" }), ""),
            FieldKind::Url { .. } => (json!({ "type": "string", "format": "uri" }), ""),
            FieldKind::Date { .. } | FieldKind::Autodate { .. } => {
                (json!({ "type": "string", "format": "date-time" }), "")
            }
            FieldKind::Select { values, max_select } => {
                let schema = if *max_select > 1 {
                    json!({ "type": "array", "items": { "type": "string", "enum": values } })
                } else {
                    json!({ "type": "string", "enum": values })
                };
                (schema, "")
            }
            FieldKind::File { max_select, .. } => {
                let item = json!({ "type": "string" });
                let schema = if *max_select != 1 {
                    json!({ "type": "array", "items": item })
                } else {
                    item
                };
                (
                    schema,
                    "Stored file name; MCP/REST tool calls cannot upload new files.",
                )
            }
            FieldKind::Relation { max_select, .. } => {
                let item = json!({ "type": "string" });
                let schema = if *max_select != 1 {
                    json!({ "type": "array", "items": item })
                } else {
                    item
                };
                (schema, "Related record id.")
            }
            FieldKind::Json { .. } => (json!({}), "Arbitrary JSON value."),
            FieldKind::Password { .. } => (
                json!({ "type": "string" }),
                "Write-only; never returned by reads.",
            ),
            FieldKind::GeoPoint {} => (
                json!({
                    "type": "object",
                    "properties": { "lon": {"type": "number"}, "lat": {"type": "number"} },
                    "required": ["lon", "lat"],
                }),
                "",
            ),
            FieldKind::Vector { .. } => (
                json!({ "type": "array", "items": { "type": "number" } }),
                "Embedding vector; usually computed server-side, not supplied directly.",
            ),
        };
        let description = if !help.is_empty() { help } else { default_note };
        if !description.is_empty() {
            if let Value::Object(map) = &mut schema {
                map.insert("description".into(), json!(description));
            }
        }
        schema
    }
}

/// A single field (column) definition inside a [`crate::Collection`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub system: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub presentable: bool,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub help: String,
    #[serde(flatten)]
    pub kind: FieldKind,
}

impl Field {
    pub fn new(name: impl Into<String>, kind: FieldKind) -> Self {
        let name = name.into();
        Field {
            id: crate::ids::field_id(kind.field_type().as_str(), &name),
            name,
            system: false,
            hidden: false,
            presentable: false,
            required: false,
            help: String::new(),
            kind,
        }
    }

    pub fn system(mut self) -> Self {
        self.system = true;
        self
    }

    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn field_type(&self) -> FieldType {
        self.kind.field_type()
    }

    /// Whether the field stores a JSON array of values (select/file/
    /// relation with `maxSelect > 1`).
    pub fn is_multiple(&self) -> bool {
        match &self.kind {
            FieldKind::Select { max_select, .. }
            | FieldKind::File { max_select, .. }
            | FieldKind::Relation { max_select, .. } => *max_select > 1,
            _ => false,
        }
    }

    pub fn max_select(&self) -> Option<i64> {
        match &self.kind {
            FieldKind::Select { max_select, .. }
            | FieldKind::File { max_select, .. }
            | FieldKind::Relation { max_select, .. } => Some(*max_select),
            _ => None,
        }
    }

    pub fn is_primary_key(&self) -> bool {
        matches!(
            &self.kind,
            FieldKind::Text {
                primary_key: true,
                ..
            }
        )
    }

    /// For relation fields, the target collection id.
    pub fn relation_collection_id(&self) -> Option<&str> {
        match &self.kind {
            FieldKind::Relation { collection_id, .. } => Some(collection_id.as_str()),
            _ => None,
        }
    }

    pub fn cascade_delete(&self) -> bool {
        matches!(
            &self.kind,
            FieldKind::Relation {
                cascade_delete: true,
                ..
            }
        )
    }

    // --- PocketBase's standard system fields -------------------------------

    /// The `id` primary key field every collection has.
    pub fn id_field() -> Field {
        let mut f = Field::new(
            "id",
            FieldKind::Text {
                min: 15,
                max: 15,
                pattern: "^[a-z0-9]+$".into(),
                autogenerate_pattern: "[a-z0-9]{15}".into(),
                primary_key: true,
            },
        );
        f.system = true;
        f.required = true;
        f
    }

    pub fn created_field() -> Field {
        Field::new(
            "created",
            FieldKind::Autodate {
                on_create: true,
                on_update: false,
            },
        )
    }

    pub fn updated_field() -> Field {
        Field::new(
            "updated",
            FieldKind::Autodate {
                on_create: true,
                on_update: true,
            },
        )
    }

    pub fn password_field() -> Field {
        let mut f = Field::new(
            "password",
            FieldKind::Password {
                min: 8,
                max: 0,
                pattern: String::new(),
                cost: 0,
            },
        );
        f.system = true;
        f.hidden = true;
        f.required = true;
        f
    }

    pub fn token_key_field() -> Field {
        let mut f = Field::new(
            "tokenKey",
            FieldKind::Text {
                min: 30,
                max: 60,
                pattern: String::new(),
                autogenerate_pattern: "[a-zA-Z0-9]{50}".into(),
                primary_key: false,
            },
        );
        f.system = true;
        f.hidden = true;
        f.required = true;
        f
    }

    pub fn email_field() -> Field {
        let mut f = Field::new(
            "email",
            FieldKind::Email {
                except_domains: vec![],
                only_domains: vec![],
            },
        );
        f.system = true;
        f.required = true;
        f
    }

    pub fn email_visibility_field() -> Field {
        let mut f = Field::new("emailVisibility", FieldKind::Bool {});
        f.system = true;
        f
    }

    pub fn verified_field() -> Field {
        let mut f = Field::new("verified", FieldKind::Bool {});
        f.system = true;
        f
    }

    /// The `role` field on the built-in `_superusers` collection —
    /// `"owner"` or `"admin"` (see `crate::SUPERUSER_ROLE_OWNER`/
    /// `SUPERUSER_ROLE_ADMIN`). Required with no default value on
    /// purpose: a caller creating a new superuser account must say which
    /// role it gets rather than silently inheriting one, and every
    /// existing writer that predates this field (`App::create_superuser`,
    /// the `8_add_superuser_role.rs` migration) sets it explicitly.
    pub fn role_field() -> Field {
        let mut f = Field::new(
            "role",
            FieldKind::Select {
                values: vec![
                    crate::SUPERUSER_ROLE_OWNER.to_string(),
                    crate::SUPERUSER_ROLE_ADMIN.to_string(),
                ],
                max_select: 1,
            },
        );
        f.system = true;
        f.required = true;
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_flat_like_pocketbase() {
        let f = Field::id_field();
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["type"], "text");
        assert_eq!(v["id"], "text3208210256");
        assert_eq!(v["primaryKey"], true);
        assert_eq!(v["autogeneratePattern"], "[a-z0-9]{15}");
        assert_eq!(v["max"], 15);
        assert_eq!(v["system"], true);
        assert!(v.get("options").is_none());
    }

    #[test]
    fn deserializes_pocketbase_fixture_field() {
        let raw = r#"{
            "help": "", "hidden": false, "id": "file376926767", "maxSelect": 1,
            "maxSize": 0, "mimeTypes": ["image/jpeg"], "name": "avatar",
            "presentable": false, "protected": false, "required": false,
            "system": false, "thumbs": [], "type": "file"
        }"#;
        let f: Field = serde_json::from_str(raw).unwrap();
        assert_eq!(f.field_type(), FieldType::File);
        assert!(!f.is_multiple());
        match f.kind {
            FieldKind::File { mime_types, .. } => assert_eq!(mime_types, vec!["image/jpeg"]),
            _ => panic!(),
        }
    }

    #[test]
    fn geo_point_tag_is_camel_case() {
        let f = Field::new("loc", FieldKind::GeoPoint {});
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["type"], "geoPoint");
        let back: Field = serde_json::from_value(v).unwrap();
        assert_eq!(back.field_type(), FieldType::GeoPoint);
    }

    #[test]
    fn lenient_input_fills_defaults() {
        let f: Field = serde_json::from_str(r#"{"name":"title","type":"text"}"#).unwrap();
        assert_eq!(f.field_type(), FieldType::Text);
        assert!(!f.required);
        let f: Field =
            serde_json::from_str(r#"{"name":"tags","type":"select","values":["a"]}"#).unwrap();
        assert_eq!(f.max_select(), Some(1));
        let f: Field =
            serde_json::from_str(r#"{"name":"when","type":"date","min":"","max":"2030-01-01"}"#)
                .unwrap();
        match f.kind {
            FieldKind::Date { min, max } => {
                assert!(min.is_none());
                assert!(max.is_some());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn vector_field_round_trips_with_and_without_embedding() {
        let f = Field::new(
            "embedding",
            FieldKind::Vector {
                dimensions: 3,
                embedding: None,
            },
        );
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["type"], "vector");
        assert_eq!(v["dimensions"], 3);
        assert!(v["embedding"].is_null());
        let back: Field = serde_json::from_value(v).unwrap();
        assert_eq!(back.field_type(), FieldType::Vector);
        match back.kind {
            FieldKind::Vector {
                dimensions,
                embedding,
            } => {
                assert_eq!(dimensions, 3);
                assert!(embedding.is_none());
            }
            _ => panic!(),
        }

        let f = Field::new(
            "embedding",
            FieldKind::Vector {
                dimensions: 1536,
                embedding: Some(EmbeddingConfig {
                    provider: "echo".into(),
                    model: "text-embedding-3-small".into(),
                    source_field: "body".into(),
                }),
            },
        );
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["embedding"]["provider"], "echo");
        assert_eq!(v["embedding"]["sourceField"], "body");
        let back: Field = serde_json::from_value(v).unwrap();
        match back.kind {
            FieldKind::Vector {
                dimensions,
                embedding: Some(cfg),
            } => {
                assert_eq!(dimensions, 1536);
                assert_eq!(cfg.provider, "echo");
                assert_eq!(cfg.source_field, "body");
            }
            _ => panic!(),
        }
    }
}
