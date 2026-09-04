//! A record: one row of a collection, plus its expansions, as the API
//! serializes it.

use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::collection::Collection;
use crate::datetime::DateTime;
use crate::field::FieldType;

/// Controls which parts of a record the JSON output includes. The caller
/// (the service layer) decides based on who is asking.
#[derive(Debug, Clone, Copy, Default)]
pub struct SerializeOptions {
    /// Include `hidden` fields (`password`, `tokenKey`, custom hidden
    /// fields). PocketBase never exposes these over HTTP; only internal
    /// callers (hooks, the JS runtime) ask for them.
    pub with_hidden: bool,
    /// Show `email` on an auth record regardless of `emailVisibility`
    /// (the viewer is the record itself, a superuser, or passes
    /// `manageRule`).
    pub show_email: bool,
    /// Include custom (non-schema) data set via `set_custom`.
    pub with_custom_data: bool,
}

#[derive(Debug, Clone)]
pub struct Record {
    pub collection: Arc<Collection>,
    data: IndexMap<String, Value>,
    expand: IndexMap<String, Value>,
    custom: IndexMap<String, Value>,
    is_new: bool,
    /// Values as they were loaded, kept so an update can write only the
    /// columns that actually changed. `None` means "nothing has been
    /// mutated since load", in which case `data` *is* the original — a
    /// list of 200 rows would otherwise clone 200 whole value maps for
    /// change tracking that read paths never consult. The snapshot is
    /// taken lazily on the first mutation of a loaded record.
    original: Option<IndexMap<String, Value>>,
}

impl Record {
    /// A new, unsaved record with every field at its zero value.
    pub fn new(collection: Arc<Collection>) -> Self {
        let mut data = IndexMap::with_capacity(collection.fields.len());
        for f in &collection.fields {
            data.insert(f.name.clone(), zero_value(f.field_type(), f.is_multiple()));
        }
        Record {
            collection,
            data,
            expand: IndexMap::new(),
            custom: IndexMap::new(),
            is_new: true,
            original: None,
        }
    }

    /// Snapshot `data` before the first mutation of a loaded record, so
    /// [`Record::original`] keeps reporting the loaded values.
    fn snapshot_original(&mut self) {
        if !self.is_new && self.original.is_none() {
            self.original = Some(self.data.clone());
        }
    }

    /// A record loaded from storage (`is_new == false`). `data` keys that
    /// are not fields are ignored.
    pub fn from_loaded(collection: Arc<Collection>, mut data: Map<String, Value>) -> Self {
        let mut record = Record::new(collection);
        // Walk the schema and pull each field out of `data`, rather than
        // walking `data` and asking `has_field` per key — the latter is a
        // linear scan of the schema for every key.
        let names: Vec<String> = record
            .collection
            .fields
            .iter()
            .map(|f| f.name.clone())
            .collect();
        for name in names {
            if let Some(v) = data.remove(&name) {
                record.data.insert(name, v);
            }
        }
        record.is_new = false;
        record.original = None;
        record
    }

    pub fn id(&self) -> &str {
        self.data.get("id").and_then(Value::as_str).unwrap_or("")
    }

    pub fn set_id(&mut self, id: impl Into<String>) {
        self.snapshot_original();
        self.data.insert("id".into(), Value::String(id.into()));
    }

    pub fn is_new(&self) -> bool {
        self.is_new
    }

    /// Mark the record as persisted: it is no longer new, and its current
    /// values become the baseline for the next change diff.
    pub fn mark_saved(&mut self) {
        self.is_new = false;
        self.original = None;
    }

    pub fn collection(&self) -> &Arc<Collection> {
        &self.collection
    }

    pub fn get(&self, field: &str) -> Option<&Value> {
        self.data.get(field)
    }

    pub fn get_string(&self, field: &str) -> String {
        match self.data.get(field) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        }
    }

    pub fn get_bool(&self, field: &str) -> bool {
        matches!(self.data.get(field), Some(Value::Bool(true)))
    }

    pub fn get_f64(&self, field: &str) -> f64 {
        self.data.get(field).and_then(Value::as_f64).unwrap_or(0.0)
    }

    pub fn get_datetime(&self, field: &str) -> Option<DateTime> {
        self.data
            .get(field)
            .and_then(Value::as_str)
            .and_then(DateTime::parse)
    }

    /// Multi-valued fields as a list; a scalar becomes a one-element list.
    pub fn get_string_list(&self, field: &str) -> Vec<String> {
        match self.data.get(field) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
            _ => vec![],
        }
    }

    /// Set a field. Unknown fields are stored as custom data only when
    /// `with_custom_data` is later requested; they never reach storage.
    pub fn set(&mut self, field: &str, value: Value) {
        if self.collection.has_field(field) {
            self.snapshot_original();
            self.data.insert(field.to_string(), value);
        } else {
            self.custom.insert(field.to_string(), value);
        }
    }

    pub fn set_custom(&mut self, key: &str, value: Value) {
        self.custom.insert(key.to_string(), value);
    }

    /// The value this field had when the record was loaded. For a record
    /// that has not been mutated this is simply its current value.
    pub fn original(&self, field: &str) -> Option<&Value> {
        self.original.as_ref().unwrap_or(&self.data).get(field)
    }

    /// Field names whose value differs from the loaded one. Empty for a
    /// record that has not been mutated; every field for a new record.
    pub fn changed_fields(&self) -> Vec<&str> {
        match &self.original {
            None if self.is_new => self.data.keys().map(String::as_str).collect(),
            None => Vec::new(),
            Some(original) => self
                .data
                .iter()
                .filter(|(k, v)| original.get(*k) != Some(*v))
                .map(|(k, _)| k.as_str())
                .collect(),
        }
    }

    pub fn expand(&self) -> &IndexMap<String, Value> {
        &self.expand
    }

    pub fn set_expand(&mut self, key: &str, value: Value) {
        self.expand.insert(key.to_string(), value);
    }

    pub fn merge_expand(&mut self, other: IndexMap<String, Value>) {
        self.expand.extend(other);
    }

    /// Raw field data, including hidden fields. Internal use.
    pub fn data(&self) -> &IndexMap<String, Value> {
        &self.data
    }

    /// Direct access to the field map. This bypasses change tracking's
    /// normal trigger, so it snapshots eagerly for an already-loaded
    /// record; filling a fresh `Record::new` (the decode path) costs
    /// nothing.
    pub fn data_mut(&mut self) -> &mut IndexMap<String, Value> {
        self.snapshot_original();
        &mut self.data
    }

    // --- auth record helpers ------------------------------------------------

    pub fn email(&self) -> String {
        self.get_string("email")
    }

    pub fn verified(&self) -> bool {
        self.get_bool("verified")
    }

    pub fn email_visibility(&self) -> bool {
        self.get_bool("emailVisibility")
    }

    pub fn token_key(&self) -> String {
        self.get_string("tokenKey")
    }

    /// The stored password hash (never the plaintext).
    pub fn password_hash(&self) -> String {
        self.get_string("password")
    }

    /// JSON as the HTTP API emits it: schema fields in order (minus hidden
    /// ones and a masked `email`), then `collectionId`, `collectionName`,
    /// then `expand` when non-empty. Keys are sorted at the end to match
    /// PocketBase's map serialization.
    pub fn to_json(&self, opts: SerializeOptions) -> Value {
        let mut m = Map::new();
        for f in &self.collection.fields {
            if f.hidden && !opts.with_hidden {
                continue;
            }
            if f.field_type() == FieldType::Password && !opts.with_hidden {
                continue;
            }
            // An email the viewer may not see is omitted entirely rather
            // than blanked — PocketBase drops the key, and a client that
            // checks `"email" in record` would otherwise see a field that
            // looks present but empty. Measured in
            // tests/conformance/KNOWN_DIVERGENCES.md #20.
            if self.collection.is_auth()
                && f.name == "email"
                && !opts.show_email
                && !self.email_visibility()
            {
                continue;
            }
            let value = self.data.get(&f.name).cloned().unwrap_or(Value::Null);
            m.insert(f.name.clone(), value);
        }
        m.insert(
            "collectionId".into(),
            Value::String(self.collection.id.clone()),
        );
        m.insert(
            "collectionName".into(),
            Value::String(self.collection.name.clone()),
        );
        if !self.expand.is_empty() {
            m.insert(
                "expand".into(),
                Value::Object(
                    self.expand
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                ),
            );
        }
        if opts.with_custom_data {
            for (k, v) in &self.custom {
                m.insert(k.clone(), v.clone());
            }
        }
        Value::Object(m)
    }
}

/// The value a field holds when nothing has been set: PocketBase returns
/// `""` for text-like fields, `0` for numbers, `false` for bools, `[]`
/// for multi-valued fields, `null` for json/geoPoint-less fields.
pub fn zero_value(t: FieldType, multiple: bool) -> Value {
    if multiple {
        return Value::Array(vec![]);
    }
    match t {
        FieldType::Number => Value::Number(0.into()),
        FieldType::Bool => Value::Bool(false),
        FieldType::Json | FieldType::Vector => Value::Null,
        FieldType::GeoPoint => serde_json::json!({ "lon": 0.0, "lat": 0.0 }),
        _ => Value::String(String::new()),
    }
}

/// Apply a PocketBase `?fields=` projection to a serialized record or
/// list. Supports `*`, comma-separated paths, `expand.rel.field`, and the
/// `:excerpt(maxLength, withEllipsis)` modifier on string values.
pub fn project_fields(value: &mut Value, spec: &str) {
    let spec = spec.trim();
    if spec.is_empty() {
        return;
    }
    let entries: Vec<FieldSpec> = split_top_level(spec)
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| FieldSpec::parse(&s))
        .collect();
    if entries.is_empty() {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                project_object(item, &entries);
            }
        }
        obj => project_object(obj, &entries),
    }
}

/// Split on commas that are not inside parentheses, so
/// `title:excerpt(200,true),body` yields two entries.
fn split_top_level(spec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for c in spec.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    out.push(current);
    out
}

#[derive(Debug, Clone)]
struct FieldSpec {
    path: Vec<String>,
    excerpt: Option<(usize, bool)>,
}

impl FieldSpec {
    fn parse(raw: &str) -> Self {
        let (path, modifier) = match raw.split_once(':') {
            Some((p, m)) => (p, Some(m)),
            None => (raw, None),
        };
        let excerpt = modifier.and_then(|m| {
            let inner = m.strip_prefix("excerpt(")?.strip_suffix(')')?;
            let mut parts = inner.split(',').map(str::trim);
            let len: usize = parts.next()?.parse().ok()?;
            let ellipsis = parts.next().map(|s| s == "true").unwrap_or(false);
            Some((len, ellipsis))
        });
        FieldSpec {
            path: path.split('.').map(str::to_string).collect(),
            excerpt,
        }
    }
}

fn project_object(value: &mut Value, specs: &[FieldSpec]) {
    let Value::Object(map) = value else {
        return;
    };
    let mut out = Map::new();
    // Keys that another spec addresses with a deeper path. `*` must not
    // copy those wholesale: in `?fields=*,expand.author.name` the star
    // would otherwise emit the entire `expand` object, and the narrower
    // spec — which merges into what is already there — would have nothing
    // left to trim.
    let deep: std::collections::HashSet<&str> = specs
        .iter()
        .filter(|s| s.path.len() > 1)
        .map(|s| s.path[0].as_str())
        .collect();
    for spec in specs {
        project_into(map, &spec.path, &mut out, spec.excerpt, &deep);
    }
    *value = Value::Object(out);
}

fn project_into(
    src: &Map<String, Value>,
    path: &[String],
    out: &mut Map<String, Value>,
    excerpt: Option<(usize, bool)>,
    deep: &std::collections::HashSet<&str>,
) {
    let Some((head, rest)) = path.split_first() else {
        return;
    };
    if head == "*" {
        for (k, v) in src {
            if deep.contains(k.as_str()) {
                continue;
            }
            if rest.is_empty() {
                out.insert(k.clone(), apply_excerpt(v.clone(), excerpt));
            } else {
                project_nested(k, v, rest, out, excerpt);
            }
        }
        return;
    }
    let Some(v) = src.get(head) else {
        return;
    };
    if rest.is_empty() {
        out.insert(head.clone(), apply_excerpt(v.clone(), excerpt));
    } else {
        project_nested(head, v, rest, out, excerpt);
    }
}

fn project_nested(
    key: &str,
    v: &Value,
    rest: &[String],
    out: &mut Map<String, Value>,
    excerpt: Option<(usize, bool)>,
) {
    let slot = out.entry(key.to_string()).or_insert_with(|| match v {
        Value::Array(_) => Value::Array(vec![]),
        _ => Value::Object(Map::new()),
    });
    match (v, slot) {
        (Value::Object(inner), Value::Object(target)) => {
            project_into(inner, rest, target, excerpt, &Default::default());
        }
        (Value::Array(items), Value::Array(targets)) => {
            if targets.len() < items.len() {
                targets.resize(items.len(), Value::Object(Map::new()));
            }
            for (item, target) in items.iter().zip(targets.iter_mut()) {
                if let (Value::Object(inner), Value::Object(t)) = (item, target) {
                    project_into(inner, rest, t, excerpt, &Default::default());
                }
            }
        }
        _ => {}
    }
}

fn apply_excerpt(v: Value, excerpt: Option<(usize, bool)>) -> Value {
    let Some((max, ellipsis)) = excerpt else {
        return v;
    };
    match v {
        Value::String(s) => {
            // PocketBase strips HTML tags and collapses whitespace first.
            let plain = strip_tags(&s);
            let chars: Vec<char> = plain.chars().collect();
            if chars.len() <= max {
                Value::String(plain)
            } else {
                let mut cut: String = chars[..max].iter().collect();
                if ellipsis {
                    cut.push_str("...");
                }
                Value::String(cut)
            }
        }
        other => other,
    }
}

/// Drop HTML tags and collapse runs of whitespace, the way PocketBase
/// prepares an `:excerpt` operand.
///
/// A tag is removed, not replaced with a space: `Hello <b>world</b>, ...`
/// must read `Hello world, ...`, and inserting a space would put one in
/// front of the comma.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::CollectionType;
    use crate::field::{Field, FieldKind};

    fn posts() -> Arc<Collection> {
        let mut c = Collection::new("posts", CollectionType::Base);
        c.fields.insert(
            1,
            Field::new(
                "title",
                FieldKind::Text {
                    min: 0,
                    max: 0,
                    pattern: String::new(),
                    autogenerate_pattern: String::new(),
                    primary_key: false,
                },
            ),
        );
        c.fields.insert(
            2,
            Field::new(
                "tags",
                FieldKind::Select {
                    values: vec!["a".into(), "b".into()],
                    max_select: 3,
                },
            ),
        );
        Arc::new(c)
    }

    #[test]
    fn new_record_has_zero_values_and_no_hidden_leak() {
        let users = Arc::new(Collection::default_users());
        let r = Record::new(users);
        let v = r.to_json(SerializeOptions::default());
        assert!(v.get("email").is_none());
        assert_eq!(v["verified"], false);
        assert!(v.get("password").is_none());
        assert!(v.get("tokenKey").is_none());
        assert_eq!(v["collectionName"], "users");
        let v = r.to_json(SerializeOptions {
            with_hidden: true,
            ..Default::default()
        });
        assert!(v.get("tokenKey").is_some());
    }

    #[test]
    fn star_yields_to_a_deeper_spec_for_the_same_key() {
        // `*` must not emit the whole `expand`, or the narrower spec has
        // nothing left to trim. Asserted against PocketBase in
        // tests/conformance/records.test.ts.
        let mut v = serde_json::json!({
            "id": "1",
            "title": "t",
            "expand": {"author": {"id": "a1", "name": "Ann", "email": "e@x.co"}}
        });
        project_fields(&mut v, "*,expand.author.name");
        assert_eq!(
            v,
            serde_json::json!({
                "id": "1",
                "title": "t",
                "expand": {"author": {"name": "Ann"}}
            })
        );

        // With no deeper spec, `*` still emits everything.
        let mut v = serde_json::json!({"id": "1", "expand": {"a": {"b": 2}}});
        project_fields(&mut v, "*");
        assert_eq!(v, serde_json::json!({"id": "1", "expand": {"a": {"b": 2}}}));
    }

    #[test]
    fn excerpt_removes_tags_without_leaving_a_space_before_punctuation() {
        let mut v = serde_json::json!({"body": "Hello <b>world</b>, this is a post."});
        project_fields(&mut v, "body:excerpt(17,true)");
        assert_eq!(v["body"], "Hello world, this...");

        let mut v = serde_json::json!({"body": "<p>Short</p>"});
        project_fields(&mut v, "body:excerpt(50)");
        assert_eq!(v["body"], "Short");
    }

    #[test]
    fn change_tracking_is_copy_on_write() {
        let c = posts();
        let mut r = Record::from_loaded(
            c,
            serde_json::json!({"id": "a", "title": "old"})
                .as_object()
                .unwrap()
                .clone(),
        );
        // Nothing mutated: the loaded values are the current values, and
        // no snapshot has been taken.
        assert!(!r.is_new());
        assert_eq!(r.original("title"), Some(&Value::String("old".into())));
        assert!(r.changed_fields().is_empty());

        r.set("title", Value::String("new".into()));
        assert_eq!(r.original("title"), Some(&Value::String("old".into())));
        assert_eq!(r.get("title"), Some(&Value::String("new".into())));
        assert_eq!(r.changed_fields(), vec!["title"]);

        // Re-setting keeps the ORIGINAL baseline, not the intermediate.
        r.set("title", Value::String("newer".into()));
        assert_eq!(r.original("title"), Some(&Value::String("old".into())));

        // Saving rebaselines.
        r.mark_saved();
        assert_eq!(r.original("title"), Some(&Value::String("newer".into())));
        assert!(r.changed_fields().is_empty());
    }

    #[test]
    fn from_loaded_ignores_unknown_keys() {
        let r = Record::from_loaded(
            posts(),
            serde_json::json!({"id": "a", "title": "t", "nope": 1})
                .as_object()
                .unwrap()
                .clone(),
        );
        let v = r.to_json(SerializeOptions::default());
        assert_eq!(v["title"], "t");
        assert!(v.get("nope").is_none());
    }

    #[test]
    fn hidden_email_is_omitted_not_blanked() {
        let users = Arc::new(Collection::default_users());
        let mut r = Record::new(users);
        r.set("email", Value::String("a@b.co".into()));
        // Not visible: the key is absent entirely, as PocketBase does.
        let v = r.to_json(SerializeOptions::default());
        assert!(v.get("email").is_none());
        assert!(v.get("emailVisibility").is_some());

        r.set("emailVisibility", Value::Bool(true));
        assert_eq!(r.to_json(SerializeOptions::default())["email"], "a@b.co");

        // Self / superuser / manageRule sees it even when hidden.
        r.set("emailVisibility", Value::Bool(false));
        let opts = SerializeOptions {
            show_email: true,
            ..Default::default()
        };
        assert_eq!(r.to_json(opts)["email"], "a@b.co");
    }

    #[test]
    fn multi_field_zero_is_empty_array() {
        let r = Record::new(posts());
        assert_eq!(
            r.to_json(SerializeOptions::default())["tags"],
            serde_json::json!([])
        );
    }

    #[test]
    fn fields_projection_with_excerpt_and_expand() {
        let mut v = serde_json::json!({
            "id": "1", "title": "<p>Hello   big</p> world", "body": "x",
            "expand": {"author": {"id": "a1", "name": "Ann", "email": "e"}}
        });
        project_fields(&mut v, "id,title:excerpt(7,true),expand.author.name");
        assert_eq!(
            v,
            serde_json::json!({"id": "1", "title": "Hello b...", "expand": {"author": {"name": "Ann"}}})
        );

        let mut v = serde_json::json!([{"a": 1, "b": 2}, {"a": 3, "b": 4}]);
        project_fields(&mut v, "a");
        assert_eq!(v, serde_json::json!([{"a": 1}, {"a": 3}]));

        let mut v = serde_json::json!({"a": 1, "b": "long text"});
        project_fields(&mut v, "*:excerpt(4)");
        assert_eq!(v, serde_json::json!({"a": 1, "b": "long"}));
    }
}
