//! The per-request context rules and filters are evaluated against, and
//! the bridge that turns it (plus the collection store) into a
//! [`cratebase_filter::Resolver`].
//!
//! PocketBase exposes the request to API rules through the `@request.*`
//! macros: `@request.auth.id`, `@request.body.title`,
//! `@request.query.foo`, `@request.headers.x_token`, `@request.method`
//! and `@request.context`. [`RequestContext`] is exactly that surface,
//! nothing more — the HTTP layer fills it in, `crates/db` only reads it.

use std::sync::Arc;

use cratebase_core::{Collection, Record};
use cratebase_filter::{Dialect, RequestPath, Resolver};
use serde_json::{Map, Value};

use crate::collections::CollectionStore;

/// The authenticated caller: the auth record itself plus the collection
/// it belongs to. `is_superuser` is `true` only for records of the
/// built-in `_superusers` collection (PocketBase has no separate admin
/// table since v0.23 — superusers are ordinary auth records).
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub record: Record,
    pub collection: Arc<Collection>,
    pub is_superuser: bool,
}

impl AuthContext {
    /// Build an auth context from a record, deriving `is_superuser` from
    /// its collection name.
    pub fn new(record: Record) -> Self {
        let collection = record.collection().clone();
        AuthContext {
            is_superuser: collection.is_superusers(),
            record,
            collection,
        }
    }

    pub fn id(&self) -> &str {
        self.record.id()
    }

    pub fn collection_id(&self) -> &str {
        &self.collection.id
    }
}

/// Everything a rule or filter may reference beyond the record columns.
///
/// `Default` is an unauthenticated `GET` in the `"default"` context,
/// which is what an anonymous request looks like.
#[derive(Debug, Clone)]
pub struct RequestContext {
    pub auth: Option<AuthContext>,
    pub body: Map<String, Value>,
    pub query: Map<String, Value>,
    /// Header names lowercased with `-` replaced by `_`, matching how
    /// PocketBase spells them in rules (`@request.headers.x_token`).
    pub headers: Map<String, Value>,
    pub method: String,
    /// One of `default`, `oauth2`, `otp`, `password`, `realtime`,
    /// `protectedFile` — PocketBase's `@request.context` values.
    pub context: String,
    /// Bypass every API rule. Set by [`RequestContext::superuser`] for
    /// internal callers (migrations, hooks, the dashboard) that have no
    /// auth record to attach but must not be filtered.
    pub superuser: bool,
}

impl Default for RequestContext {
    fn default() -> Self {
        RequestContext {
            auth: None,
            body: Map::new(),
            query: Map::new(),
            headers: Map::new(),
            method: "GET".into(),
            context: "default".into(),
            superuser: false,
        }
    }
}

impl RequestContext {
    /// A context that bypasses every API rule, for internal callers.
    pub fn superuser() -> Self {
        RequestContext {
            superuser: true,
            ..Default::default()
        }
    }

    /// Whether rules should be skipped: an explicit superuser context, or
    /// an authenticated `_superusers` record.
    pub fn is_superuser(&self) -> bool {
        self.superuser || self.auth.as_ref().is_some_and(|a| a.is_superuser)
    }

    pub fn auth_id(&self) -> Option<&str> {
        self.auth.as_ref().map(AuthContext::id)
    }

    /// Normalize a header name the way PocketBase does before looking it
    /// up in a rule (`X-Token` → `x_token`).
    pub fn header_key(name: &str) -> String {
        name.to_ascii_lowercase().replace('-', "_")
    }

    pub fn with_body(mut self, body: Map<String, Value>) -> Self {
        self.body = body;
        self
    }

    pub fn with_auth(mut self, auth: AuthContext) -> Self {
        self.auth = Some(auth);
        self
    }
}

/// Implements [`Resolver`] over a root collection, the process-wide
/// [`CollectionStore`] and a [`RequestContext`].
///
/// The store is what makes relation traversal (`author.name`),
/// back-relations (`comments_via_post.title`) and `@collection.X` work:
/// the filter compiler asks for other collections by name or id and the
/// snapshot answers without touching the database.
pub struct CollectionResolver<'a> {
    root: Arc<Collection>,
    store: &'a CollectionStore,
    ctx: &'a RequestContext,
    dialect: Dialect,
}

impl<'a> CollectionResolver<'a> {
    pub fn new(
        root: Arc<Collection>,
        store: &'a CollectionStore,
        ctx: &'a RequestContext,
        dialect: Dialect,
    ) -> Self {
        CollectionResolver {
            root,
            store,
            ctx,
            dialect,
        }
    }

    pub fn ctx(&self) -> &RequestContext {
        self.ctx
    }

    pub fn store(&self) -> &'a CollectionStore {
        self.store
    }

    /// The collection filters are compiled against. Named `root_collection`
    /// rather than `collection` so it cannot shadow
    /// [`Resolver::collection`], which looks *another* collection up.
    pub fn root_collection(&self) -> &Arc<Collection> {
        &self.root
    }
}

/// Navigate a JSON value by dotted path segments, `null` when missing.
/// Array indices are accepted so `@request.body.tags.0` resolves.
fn dig(value: &Value, path: &str) -> Value {
    let mut cur = value;
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

fn dig_map(map: &Map<String, Value>, path: &str) -> Value {
    match path.split_once('.') {
        None => map.get(path).cloned().unwrap_or(Value::Null),
        Some((head, rest)) => match map.get(head) {
            Some(v) => dig(v, rest),
            None => Value::Null,
        },
    }
}

/// `@request.auth.<path>`. `id`, `collectionId` and `collectionName` are
/// synthesized; everything else comes from the auth record's fields —
/// including hidden ones (`tokenKey`, `password`), because rules are
/// server-side and PocketBase lets them reference those.
fn auth_value(auth: &AuthContext, path: &str) -> Value {
    let (head, rest) = match path.split_once('.') {
        Some((h, r)) => (h, Some(r)),
        None => (path, None),
    };
    let head_value = match head {
        "id" => Value::String(auth.id().to_string()),
        "collectionId" => Value::String(auth.collection.id.clone()),
        "collectionName" => Value::String(auth.collection.name.clone()),
        other => auth.record.get(other).cloned().unwrap_or(Value::Null),
    };
    match rest {
        None => head_value,
        Some(rest) => dig(&head_value, rest),
    }
}

impl Resolver for CollectionResolver<'_> {
    fn root(&self) -> &Collection {
        &self.root
    }

    fn collection(&self, name_or_id: &str) -> Option<Arc<Collection>> {
        self.store.get(name_or_id)
    }

    fn request_value(&self, path: &RequestPath) -> Value {
        match path {
            // Bare `@request.auth` is truthy when someone is signed in;
            // PocketBase compiles `@request.auth.id != ""` the same way.
            RequestPath::Auth(None) => Value::Bool(self.ctx.auth.is_some()),
            RequestPath::Auth(Some(p)) => match &self.ctx.auth {
                Some(auth) => auth_value(auth, p),
                None => Value::Null,
            },
            RequestPath::Body(p) => dig_map(&self.ctx.body, p),
            RequestPath::Query(p) => dig_map(&self.ctx.query, p),
            RequestPath::Headers(p) => dig_map(&self.ctx.headers, p),
            RequestPath::Method => Value::String(self.ctx.method.clone()),
            RequestPath::Context => Value::String(self.ctx.context.clone()),
        }
    }

    fn body_has(&self, key: &str) -> bool {
        let head = key.split('.').next().unwrap_or(key);
        self.ctx.body.contains_key(head)
    }

    fn dialect(&self) -> Dialect {
        self.dialect
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn users_record() -> Record {
        let users = Arc::new(Collection::default_users());
        let mut r = Record::new(users);
        r.set_id("u1");
        r.set("email", json!("a@b.co"));
        r.set("tokenKey", json!("tk"));
        r
    }

    #[test]
    fn auth_paths_resolve_including_hidden_fields() {
        let auth = AuthContext::new(users_record());
        assert_eq!(auth_value(&auth, "id"), json!("u1"));
        assert_eq!(auth_value(&auth, "collectionId"), json!("_pb_users_auth_"));
        assert_eq!(auth_value(&auth, "collectionName"), json!("users"));
        assert_eq!(auth_value(&auth, "email"), json!("a@b.co"));
        assert_eq!(auth_value(&auth, "tokenKey"), json!("tk"));
        assert_eq!(auth_value(&auth, "nope"), Value::Null);
        assert!(!auth.is_superuser);
    }

    #[test]
    fn request_values_and_superuser_flag() {
        let store = CollectionStore::new();
        store.replace(vec![Collection::default_users()]);
        let users = store.get("users").unwrap();

        let mut ctx = RequestContext {
            body: json!({"title": "hi", "meta": {"n": 2}})
                .as_object()
                .cloned()
                .unwrap(),
            query: json!({"page": 1}).as_object().cloned().unwrap(),
            headers: json!({"x_token": "t"}).as_object().cloned().unwrap(),
            method: "POST".into(),
            context: "oauth2".into(),
            ..Default::default()
        };
        let r = CollectionResolver::new(users.clone(), &store, &ctx, Dialect::Sqlite);
        assert_eq!(r.request_value(&RequestPath::Auth(None)), json!(false));
        assert_eq!(
            r.request_value(&RequestPath::Body("meta.n".into())),
            json!(2)
        );
        assert_eq!(
            r.request_value(&RequestPath::Query("page".into())),
            json!(1)
        );
        assert_eq!(
            r.request_value(&RequestPath::Headers("x_token".into())),
            json!("t")
        );
        assert_eq!(r.request_value(&RequestPath::Method), json!("POST"));
        assert_eq!(r.request_value(&RequestPath::Context), json!("oauth2"));
        assert!(r.body_has("title"));
        assert!(r.body_has("meta.n"));
        assert!(!r.body_has("other"));
        assert_eq!(r.root().name, "users");
        assert!(Resolver::collection(&r, "users").is_some());
        drop(r);

        ctx.auth = Some(AuthContext::new(users_record()));
        let r = CollectionResolver::new(users, &store, &ctx, Dialect::Sqlite);
        assert_eq!(r.request_value(&RequestPath::Auth(None)), json!(true));
        assert_eq!(
            r.request_value(&RequestPath::Auth(Some("id".into()))),
            json!("u1")
        );
        assert!(!r.ctx().is_superuser());

        assert!(RequestContext::superuser().is_superuser());
        assert_eq!(
            RequestContext::header_key("X-Custom-Token"),
            "x_custom_token"
        );
    }
}
