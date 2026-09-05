//! `_collections` persistence and the in-memory [`CollectionStore`].
//!
//! Every request addresses a collection and needs its fields and rules,
//! so the store is the only read path: a fully loaded [`Snapshot`]
//! behind an `ArcSwap`, replaced atomically after any change. Readers
//! take an `Arc<Snapshot>` (one atomic load, no lock) and keep a
//! consistent view for the duration of a request even if a collection
//! is modified concurrently.
//!
//! Two server processes sharing one database will not see each other's
//! schema changes until restart, the same limitation PocketBase has.

use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use cratebase_core::{Collection, DateTime};
use serde_json::{json, Map, Value};

use crate::backend::Backend;
use crate::engine::{Engine, Executor, Row, Sql};
use crate::error::{DbError, DbResult};
use crate::schema;

/// An immutable view of every collection.
#[derive(Default)]
pub struct Snapshot {
    pub all: Vec<Arc<Collection>>,
    pub by_id: HashMap<String, Arc<Collection>>,
    pub by_name: HashMap<String, Arc<Collection>>,
}

impl Snapshot {
    fn build(collections: Vec<Collection>) -> Self {
        let mut all: Vec<Arc<Collection>> = collections.into_iter().map(Arc::new).collect();
        all.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.name.cmp(&b.name)));
        let by_id = all.iter().map(|c| (c.id.clone(), c.clone())).collect();
        let by_name = all.iter().map(|c| (c.name.clone(), c.clone())).collect();
        Snapshot {
            all,
            by_id,
            by_name,
        }
    }

    pub fn get(&self, name_or_id: &str) -> Option<Arc<Collection>> {
        self.by_name
            .get(name_or_id)
            .or_else(|| self.by_id.get(name_or_id))
            .cloned()
    }
}

#[derive(Clone, Default)]
pub struct CollectionStore {
    snapshot: Arc<ArcSwap<Snapshot>>,
}

impl CollectionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the snapshot with everything in `_collections`.
    pub async fn load(&self, ex: &dyn Executor) -> DbResult<()> {
        let all = fetch_all(ex).await?;
        self.replace(all);
        Ok(())
    }

    /// Replace the snapshot from an in-memory list (tests, imports).
    pub fn replace(&self, collections: Vec<Collection>) {
        self.snapshot.store(Arc::new(Snapshot::build(collections)));
    }

    pub fn all(&self) -> Arc<Snapshot> {
        self.snapshot.load_full()
    }

    pub fn get(&self, name_or_id: &str) -> Option<Arc<Collection>> {
        self.snapshot.load().get(name_or_id)
    }

    pub fn get_by_id(&self, id: &str) -> Option<Arc<Collection>> {
        self.snapshot.load().by_id.get(id).cloned()
    }

    pub fn get_by_name(&self, name: &str) -> Option<Arc<Collection>> {
        self.snapshot.load().by_name.get(name).cloned()
    }

    pub fn len(&self) -> usize {
        self.snapshot.load().all.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Persist a new collection, create its table/view and indexes, and
    /// reload the snapshot. All DDL and the `_collections` row share one
    /// transaction.
    pub async fn insert(
        &self,
        engine: &dyn Engine,
        collection: &Collection,
    ) -> DbResult<Arc<Collection>> {
        let backend = Backend::from_dialect(engine.dialect());
        let tx = engine.begin().await?;
        insert_in(&tx, backend, collection).await?;
        tx.commit().await?;
        self.load(engine).await?;
        self.get_by_id(&collection.id).ok_or(DbError::NotFound)
    }

    /// Persist changes to an existing collection (looked up by id in the
    /// current snapshot), migrate its table, and reload the snapshot.
    pub async fn update(
        &self,
        engine: &dyn Engine,
        collection: &Collection,
    ) -> DbResult<Arc<Collection>> {
        let previous = self.get_by_id(&collection.id).ok_or(DbError::NotFound)?;
        let backend = Backend::from_dialect(engine.dialect());
        let tx = engine.begin().await?;
        update_in(&tx, backend, &previous, collection).await?;
        tx.commit().await?;
        self.load(engine).await?;
        self.get_by_id(&collection.id).ok_or(DbError::NotFound)
    }

    /// Delete a collection (by name or id), drop its table/view, and
    /// reload the snapshot.
    pub async fn delete(&self, engine: &dyn Engine, name_or_id: &str) -> DbResult<()> {
        let existing = self.get(name_or_id).ok_or(DbError::NotFound)?;
        let tx = engine.begin().await?;
        delete_in(&tx, &existing).await?;
        tx.commit().await?;
        self.load(engine).await
    }
}

// --- transaction-scoped primitives ---------------------------------------
//
// The service layer wraps these in its own transaction when it needs to
// combine a collection change with other writes; the `CollectionStore`
// methods above are the convenience form.

/// Write the `_collections` row and create the physical table within
/// the caller's executor (usually a transaction). Does not reload the
/// store.
pub async fn insert_in(ex: &dyn Executor, backend: Backend, c: &Collection) -> DbResult<()> {
    let p = row_params(c)?;
    ex.execute(
        r#"INSERT INTO "_collections" ("id", "name", "type", "system", "fields", "indexes",
            "listRule", "viewRule", "createRule", "updateRule", "deleteRule",
            "options", "created", "updated")
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)"#,
        &p,
    )
    .await?;
    schema::sync(ex, backend, None, c).await
}

/// Update the `_collections` row and migrate the physical table.
pub async fn update_in(
    ex: &dyn Executor,
    backend: Backend,
    previous: &Collection,
    next: &Collection,
) -> DbResult<()> {
    let mut p = row_params(next)?;
    // Move the id to the end for the WHERE clause.
    let id = p.remove(0);
    p.push(id);
    let n = ex
        .execute(
            r#"UPDATE "_collections" SET "name" = $1, "type" = $2, "system" = $3, "fields" = $4,
                "indexes" = $5, "listRule" = $6, "viewRule" = $7, "createRule" = $8,
                "updateRule" = $9, "deleteRule" = $10, "options" = $11, "created" = $12,
                "updated" = $13
               WHERE "id" = $14"#,
            &p,
        )
        .await?;
    if n == 0 {
        return Err(DbError::NotFound);
    }
    schema::sync(ex, backend, Some(previous), next).await
}

/// Delete the `_collections` row and drop the table/view.
pub async fn delete_in(ex: &dyn Executor, c: &Collection) -> DbResult<()> {
    ex.execute(
        r#"DELETE FROM "_collections" WHERE "id" = $1"#,
        &[Sql::from(c.id.as_str())],
    )
    .await?;
    schema::drop_object(ex, c).await
}

const SELECT: &str = r#"SELECT "id", "name", "type", "system", "fields", "indexes",
    "listRule", "viewRule", "createRule", "updateRule", "deleteRule",
    "options", "created", "updated" FROM "_collections""#;

/// Every collection, straight from the table (bypasses the store).
pub async fn fetch_all(ex: &dyn Executor) -> DbResult<Vec<Collection>> {
    let rows = ex
        .query(&format!("{SELECT} ORDER BY \"created\", \"name\""), &[])
        .await?;
    rows.iter().map(row_to_collection).collect()
}

/// One collection by name or id, straight from the table.
pub async fn fetch(ex: &dyn Executor, name_or_id: &str) -> DbResult<Collection> {
    let row = ex
        .query_one(
            &format!("{SELECT} WHERE \"name\" = $1 OR \"id\" = $1"),
            &[Sql::from(name_or_id)],
        )
        .await?
        .ok_or(DbError::NotFound)?;
    row_to_collection(&row)
}

/// The keys of the PocketBase collection JSON that live in their own
/// columns; everything else goes into `options`.
const COLUMN_KEYS: &[&str] = &[
    "id",
    "name",
    "type",
    "system",
    "fields",
    "indexes",
    "listRule",
    "viewRule",
    "createRule",
    "updateRule",
    "deleteRule",
    "created",
    "updated",
];

fn rule(r: &Option<String>) -> Sql {
    match r {
        Some(s) => Sql::Text(s.clone()),
        None => Sql::Null,
    }
}

fn row_params(c: &Collection) -> DbResult<Vec<Sql>> {
    if c.id.is_empty() || c.name.is_empty() {
        return Err(DbError::InvalidIdentifier(
            "collection id and name are required".into(),
        ));
    }
    let mut options = match c.to_json() {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    for k in COLUMN_KEYS {
        options.remove(*k);
    }
    Ok(vec![
        Sql::from(c.id.as_str()),
        Sql::from(c.name.as_str()),
        Sql::from(c.collection_type.as_str()),
        Sql::from(c.system),
        Sql::Text(serde_json::to_string(&c.fields)?),
        Sql::Text(serde_json::to_string(&c.indexes)?),
        rule(&c.list_rule),
        rule(&c.view_rule),
        rule(&c.create_rule),
        rule(&c.update_rule),
        rule(&c.delete_rule),
        Sql::Text(Value::Object(options).to_string()),
        Sql::Text(c.created.to_pb_string()),
        Sql::Text(c.updated.to_pb_string()),
    ])
}

fn text(row: &Row, col: &str) -> String {
    row.get(col).cloned().unwrap_or(Sql::Null).into_string()
}

fn nullable(row: &Row, col: &str) -> Value {
    match row.get(col) {
        Some(Sql::Null) | None => Value::Null,
        Some(v) => Value::String(v.clone().into_string()),
    }
}

/// Rebuild the PocketBase JSON from a row and let `Collection`'s
/// `Deserialize` impl (defaults included) do the rest.
pub fn row_to_collection(row: &Row) -> DbResult<Collection> {
    let fields: Value = serde_json::from_str(&text(row, "fields")).unwrap_or_else(|_| json!([]));
    let indexes: Value = serde_json::from_str(&text(row, "indexes")).unwrap_or_else(|_| json!([]));
    let options: Value = serde_json::from_str(&text(row, "options")).unwrap_or_else(|_| json!({}));
    let mut m = match options {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    m.insert("id".into(), json!(text(row, "id")));
    m.insert("name".into(), json!(text(row, "name")));
    m.insert("type".into(), json!(text(row, "type")));
    m.insert(
        "system".into(),
        json!(row.get_i64("system").unwrap_or(0) != 0),
    );
    m.insert("fields".into(), fields);
    m.insert("indexes".into(), indexes);
    for k in [
        "listRule",
        "viewRule",
        "createRule",
        "updateRule",
        "deleteRule",
    ] {
        m.insert(k.into(), nullable(row, k));
    }
    let stamp = |col: &str| DateTime::parse(&text(row, col)).unwrap_or_default();
    m.insert("created".into(), json!(stamp("created")));
    m.insert("updated".into(), json!(stamp("updated")));
    Ok(serde_json::from_value(Value::Object(m))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::SqliteEngine;
    use crate::system::ensure_system_tables;
    use cratebase_core::{CollectionType, Field, FieldKind, FieldType};

    async fn engine() -> SqliteEngine {
        let e = SqliteEngine::open_memory().unwrap();
        ensure_system_tables(&e).await.unwrap();
        e
    }

    #[tokio::test]
    async fn insert_get_update_rename_delete_and_snapshot_swap() {
        let e = engine().await;
        let store = CollectionStore::new();
        store.load(&e).await.unwrap();
        assert!(store.is_empty());

        let users = Collection::default_users();
        let stored = store.insert(&e, &users).await.unwrap();
        assert_eq!(stored.id, "_pb_users_auth_");
        assert!(stored.is_auth());
        assert_eq!(stored.auth, users.auth);
        assert_eq!(stored.to_json(), users.to_json());
        assert!(e.table_exists("users").await.unwrap());

        let mut posts = Collection::new("posts", CollectionType::Base);
        posts.list_rule = Some(String::new());
        posts.view_rule = Some("published = true".into());
        posts.fields.insert(
            1,
            Field::new("title", FieldKind::default_for(FieldType::Text)),
        );
        store.insert(&e, &posts).await.unwrap();
        let before = store.all();
        assert_eq!(before.all.len(), 2);
        assert!(store.get("posts").is_some());
        assert!(store.get(&posts.id).is_some());
        assert_eq!(
            store.get_by_name("posts").unwrap().list_rule,
            Some(String::new())
        );
        assert_eq!(
            store.get_by_id(&posts.id).unwrap().view_rule.as_deref(),
            Some("published = true")
        );
        assert!(store.get_by_id("posts").is_none());
        assert!(store.get_by_name("posts").unwrap().create_rule.is_none());

        // Duplicate name is a unique violation on `_collections.name`.
        let mut dup = Collection::new("posts", CollectionType::Base);
        dup.id = "pbc_dup".into();
        let err = store.insert(&e, &dup).await.unwrap_err();
        assert!(matches!(err, DbError::UniqueViolation(_)), "{err:?}");
        assert_eq!(store.len(), 2);

        // Rename + add field.
        let mut renamed = (*store.get("posts").unwrap()).clone();
        renamed.name = "articles".into();
        renamed.fields.push(Field::new(
            "body",
            FieldKind::default_for(FieldType::Editor),
        ));
        store.update(&e, &renamed).await.unwrap();
        assert!(store.get("posts").is_none());
        assert!(store.get("articles").is_some());
        assert!(e.table_exists("articles").await.unwrap());
        assert!(e
            .table_columns("articles")
            .await
            .unwrap()
            .contains(&"body".to_string()));
        // The old snapshot is untouched (readers holding it stay consistent).
        assert!(before.get("posts").is_some());
        assert!(!Arc::ptr_eq(&before, &store.all()));

        let mut view = Collection::new("titles", CollectionType::View);
        view.view_query = "SELECT id, title FROM articles".into();
        let stored = store.insert(&e, &view).await.unwrap();
        assert!(stored.is_view());
        assert_eq!(stored.view_query, view.view_query);
        assert_eq!(
            fetch(&e, "titles").await.unwrap().view_query,
            view.view_query
        );

        store.delete(&e, "titles").await.unwrap();
        store.delete(&e, &renamed.id).await.unwrap();
        assert!(matches!(
            store.delete(&e, "nope").await,
            Err(DbError::NotFound)
        ));
        assert!(!e.table_exists("articles").await.unwrap());
        assert_eq!(store.len(), 1);
        assert_eq!(fetch_all(&e).await.unwrap().len(), 1);

        let mut ghost = Collection::new("ghost", CollectionType::Base);
        ghost.id = "pbc_ghost".into();
        assert!(matches!(
            store.update(&e, &ghost).await,
            Err(DbError::NotFound)
        ));
    }
    #[tokio::test]
    async fn oauth2_client_secret_survives_a_storage_round_trip() {
        // REAL BUG 5: `OAuth2Provider.client_secret` used to carry
        // `#[serde(skip_serializing)]`, so `to_json()` — the exact value
        // `row_params` writes to `_collections.options` — silently
        // dropped it on every save, and every reload came back with an
        // empty secret.
        let e = engine().await;
        let store = CollectionStore::new();
        store.load(&e).await.unwrap();

        let mut users = Collection::default_users();
        users.auth.oauth2.enabled = true;
        users
            .auth
            .oauth2
            .providers
            .push(cratebase_core::OAuth2Provider {
                name: "google".into(),
                client_id: "the-client-id".into(),
                client_secret: "the-client-secret".into(),
                ..Default::default()
            });
        let stored = store.insert(&e, &users).await.unwrap();
        assert_eq!(
            stored.auth.oauth2.providers[0].client_secret,
            "the-client-secret"
        );

        // And a fresh load from storage (a process restart, in effect)
        // sees the same thing, not just the in-memory `insert` return.
        let reloaded = CollectionStore::new();
        reloaded.load(&e).await.unwrap();
        assert_eq!(
            reloaded.get_by_name("users").unwrap().auth.oauth2.providers[0].client_secret,
            "the-client-secret"
        );
    }

    #[tokio::test]
    async fn failed_ddl_rolls_back_the_row() {
        let e = engine().await;
        let store = CollectionStore::new();
        let mut bad = Collection::new("bad", CollectionType::View);
        bad.view_query = "SELECT * FROM does_not_exist".into();
        assert!(store.insert(&e, &bad).await.is_err());
        assert!(matches!(fetch(&e, "bad").await, Err(DbError::NotFound)));
        assert!(store.get("bad").is_none());
    }
}
