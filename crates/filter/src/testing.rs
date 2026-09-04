//! Test helpers: a schema fixture (posts/users/companies/tags/comments/
//! memberships) and an in-memory [`Resolver`] backed by JSON maps. Used
//! by this crate's tests and available to downstream crates' tests.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use cratebase_core::{Collection, CollectionType, Field, FieldKind};
use serde_json::{Map, Value};

use crate::resolver::{Dialect, RequestPath, Resolver};

fn text(name: &str) -> Field {
    Field::new(
        name,
        FieldKind::Text {
            min: 0,
            max: 0,
            pattern: String::new(),
            autogenerate_pattern: String::new(),
            primary_key: false,
        },
    )
}

fn relation(name: &str, target: &Collection, max_select: i64) -> Field {
    Field::new(
        name,
        FieldKind::Relation {
            collection_id: target.id.clone(),
            cascade_delete: false,
            min_select: 0,
            max_select,
        },
    )
}

fn select(name: &str, values: &[&str], max_select: i64) -> Field {
    Field::new(
        name,
        FieldKind::Select {
            values: values.iter().map(|v| v.to_string()).collect(),
            max_select,
        },
    )
}

fn insert_fields(c: &mut Collection, fields: Vec<Field>) {
    // Keep `created`/`updated` last like PocketBase's scaffold does.
    let pos = c.fields.len() - 2;
    c.fields.splice(pos..pos, fields);
}

/// The fixture schema:
///
/// * `companies`: `name`
/// * `users` (auth): `name`, `company` → companies
/// * `tags`: `name`, `aliases` (multi select), `owner` → users,
///   `related` → tags (multi)
/// * `posts`: `title`, `status` (select), `author` → users, `tags` →
///   tags (multi), `categories` (multi select), `attachments` (multi
///   file), `views` (number), `published` (bool), `data` (json), `loc`
///   (geoPoint), `published_at` (date)
/// * `comments`: `post` → posts, `title`, `author` → users, `tags` →
///   tags (multi)
/// * `memberships`: `user` → users, `team` (text), `roles` (multi select),
///   `projects` → posts (multi)
pub fn fixture_collections() -> Vec<Arc<Collection>> {
    let mut companies = Collection::new("companies", CollectionType::Base);
    insert_fields(&mut companies, vec![text("name")]);

    let mut users = Collection::default_users();
    insert_fields(&mut users, vec![relation("company", &companies, 1)]);

    let mut tags = Collection::new("tags", CollectionType::Base);
    let tags_id = tags.id.clone();
    insert_fields(
        &mut tags,
        vec![
            text("name"),
            select("aliases", &["a", "b", "c"], 5),
            relation("owner", &users, 1),
            Field::new(
                "related",
                FieldKind::Relation {
                    collection_id: tags_id,
                    cascade_delete: false,
                    min_select: 0,
                    max_select: 10,
                },
            ),
        ],
    );

    let mut posts = Collection::new("posts", CollectionType::Base);
    insert_fields(
        &mut posts,
        vec![
            text("title"),
            select("status", &["draft", "active", "archived"], 1),
            relation("author", &users, 1),
            relation("tags", &tags, 10),
            select("categories", &["news", "tech", "life"], 3),
            Field::new(
                "attachments",
                FieldKind::File {
                    max_select: 5,
                    max_size: 0,
                    mime_types: vec![],
                    thumbs: vec![],
                    protected: false,
                },
            ),
            Field::new(
                "views",
                FieldKind::Number {
                    min: None,
                    max: None,
                    only_int: true,
                },
            ),
            Field::new("published", FieldKind::Bool {}),
            Field::new("data", FieldKind::Json { max_size: 0 }),
            Field::new("loc", FieldKind::GeoPoint {}),
            Field::new(
                "published_at",
                FieldKind::Date {
                    min: None,
                    max: None,
                },
            ),
        ],
    );

    let mut comments = Collection::new("comments", CollectionType::Base);
    insert_fields(
        &mut comments,
        vec![
            relation("post", &posts, 1),
            text("title"),
            relation("author", &users, 1),
            relation("tags", &tags, 10),
        ],
    );

    let mut memberships = Collection::new("memberships", CollectionType::Base);
    insert_fields(
        &mut memberships,
        vec![
            relation("user", &users, 1),
            text("team"),
            select("roles", &["admin", "member"], 2),
            relation("projects", &posts, 20),
        ],
    );

    vec![companies, users, tags, posts, comments, memberships]
        .into_iter()
        .map(Arc::new)
        .collect()
}

/// An in-memory resolver over [`fixture_collections`].
pub struct TestResolver {
    pub root: Arc<Collection>,
    pub collections: Vec<Arc<Collection>>,
    pub dialect: Dialect,
    /// The authenticated record as JSON, or `None` when unauthenticated.
    pub auth: Option<Value>,
    pub body: Map<String, Value>,
    pub query: Map<String, Value>,
    pub headers: Map<String, Value>,
    pub method: String,
    pub context: String,
    pub now: DateTime<Utc>,
}

impl TestResolver {
    /// A resolver rooted at `root` (a fixture collection name) with an
    /// unauthenticated, empty request and a fixed clock
    /// (`2026-09-03 12:44:06.146Z`, a Thursday).
    pub fn new(root: &str, dialect: Dialect) -> Self {
        let collections = fixture_collections();
        let root = collections
            .iter()
            .find(|c| c.name == root)
            .unwrap_or_else(|| panic!("no fixture collection {root}"))
            .clone();
        TestResolver {
            root,
            collections,
            dialect,
            auth: None,
            body: Map::new(),
            query: Map::new(),
            headers: Map::new(),
            method: "GET".into(),
            context: "default".into(),
            now: Utc.with_ymd_and_hms(2026, 9, 3, 12, 44, 6).unwrap()
                + chrono::Duration::milliseconds(146),
        }
    }

    pub fn sqlite(root: &str) -> Self {
        Self::new(root, Dialect::Sqlite)
    }

    pub fn postgres(root: &str) -> Self {
        Self::new(root, Dialect::Postgres)
    }

    pub fn with_auth(mut self, auth: Value) -> Self {
        self.auth = Some(auth);
        self
    }

    pub fn with_body(mut self, body: Value) -> Self {
        self.body = body.as_object().cloned().unwrap_or_default();
        self
    }

    pub fn with_query(mut self, query: Value) -> Self {
        self.query = query.as_object().cloned().unwrap_or_default();
        self
    }

    pub fn with_headers(mut self, headers: Value) -> Self {
        self.headers = headers.as_object().cloned().unwrap_or_default();
        self
    }

    pub fn with_method(mut self, method: &str) -> Self {
        self.method = method.into();
        self
    }

    pub fn with_context(mut self, context: &str) -> Self {
        self.context = context.into();
        self
    }
}

fn dig(v: &Value, path: &str) -> Value {
    let mut cur = v;
    for seg in path.split('.') {
        cur = match cur {
            Value::Object(m) => m.get(seg).unwrap_or(&Value::Null),
            Value::Array(items) => seg
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i))
                .unwrap_or(&Value::Null),
            _ => &Value::Null,
        };
    }
    cur.clone()
}

impl Resolver for TestResolver {
    fn root(&self) -> &Collection {
        &self.root
    }

    fn collection(&self, name_or_id: &str) -> Option<Arc<Collection>> {
        self.collections
            .iter()
            .find(|c| c.name == name_or_id || c.id == name_or_id)
            .cloned()
    }

    fn request_value(&self, path: &RequestPath) -> Value {
        match path {
            RequestPath::Auth(None) => Value::Bool(self.auth.is_some()),
            RequestPath::Auth(Some(p)) => {
                self.auth.as_ref().map(|a| dig(a, p)).unwrap_or(Value::Null)
            }
            RequestPath::Body(p) => dig(&Value::Object(self.body.clone()), p),
            RequestPath::Query(p) => dig(&Value::Object(self.query.clone()), p),
            RequestPath::Headers(p) => dig(&Value::Object(self.headers.clone()), p),
            RequestPath::Method => Value::String(self.method.clone()),
            RequestPath::Context => Value::String(self.context.clone()),
        }
    }

    fn body_has(&self, key: &str) -> bool {
        let head = key.split('.').next().unwrap_or(key);
        self.body.contains_key(head)
    }

    fn dialect(&self) -> Dialect {
        self.dialect
    }

    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}
