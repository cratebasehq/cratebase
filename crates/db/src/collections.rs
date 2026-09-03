use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use cratebase_core::field::{Field, FieldType};
use cratebase_core::{AuthOptions, Collection, CollectionType};
use sqlx::any::AnyRow;
use sqlx::Row;

use crate::backend::Backend;
use crate::error::{DbError, DbResult};
use crate::pool::Db;

/// Process-wide cache of collection metadata, keyed both by id and by
/// name. Every API request addresses a collection and needs its schema
/// and rules; before this cache existed that was one `SELECT` plus two
/// `serde_json::from_str` calls (schema, auth options) per request —
/// measured in `benchmarks/` as a fixed ~200-400µs tax PocketBase (which
/// keeps collections in memory) doesn't pay.
///
/// Populated lazily on first lookup, refreshed by [`create_collection`] /
/// [`update_collection`] and evicted by [`delete_collection`]. The
/// transaction-scoped `_tx` getters bypass it (they exist precisely to
/// read uncommitted state). Two server processes sharing one SQLite file
/// will not see each other's schema changes until restart — the same
/// limitation PocketBase has, and a documented non-goal for now.
#[derive(Default)]
pub struct CollectionCache {
    inner: RwLock<CacheInner>,
}

#[derive(Default)]
struct CacheInner {
    by_id: HashMap<String, Arc<Collection>>,
    by_name: HashMap<String, Arc<Collection>>,
}

impl CollectionCache {
    pub fn get_by_id(&self, id: &str) -> Option<Arc<Collection>> {
        self.inner.read().unwrap().by_id.get(id).cloned()
    }

    pub fn get_by_name(&self, name: &str) -> Option<Arc<Collection>> {
        self.inner.read().unwrap().by_name.get(name).cloned()
    }

    /// Insert or replace. A rename is handled by evicting whatever entry
    /// previously carried this id before inserting under the new name.
    pub fn put(&self, collection: Collection) -> Arc<Collection> {
        let arc = Arc::new(collection);
        let mut inner = self.inner.write().unwrap();
        if let Some(old) = inner.by_id.remove(&arc.id) {
            inner.by_name.remove(&old.name);
        }
        inner.by_id.insert(arc.id.clone(), arc.clone());
        inner.by_name.insert(arc.name.clone(), arc.clone());
        arc
    }

    pub fn evict_id(&self, id: &str) {
        let mut inner = self.inner.write().unwrap();
        if let Some(old) = inner.by_id.remove(id) {
            inner.by_name.remove(&old.name);
        }
    }

    pub fn clear(&self) {
        let mut inner = self.inner.write().unwrap();
        inner.by_id.clear();
        inner.by_name.clear();
    }
}

fn row_to_collection(row: &AnyRow) -> DbResult<Collection> {
    let type_str: String = row.try_get("type")?;
    let collection_type = match type_str.as_str() {
        "auth" => CollectionType::Auth,
        "view" => CollectionType::View,
        _ => CollectionType::Base,
    };
    let schema_json: String = row.try_get("schema")?;
    let schema: Vec<Field> = serde_json::from_str(&schema_json)
        .map_err(|e| DbError::InvalidIdentifier(format!("corrupt schema json: {e}")))?;
    let auth_options_json: String = row.try_get("auth_options")?;
    let auth_options: AuthOptions = serde_json::from_str(&auth_options_json)
        .map_err(|e| DbError::InvalidIdentifier(format!("corrupt auth_options json: {e}")))?;

    Ok(Collection {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        collection_type,
        schema,
        list_rule: row.try_get("list_rule")?,
        view_rule: row.try_get("view_rule")?,
        create_rule: row.try_get("create_rule")?,
        update_rule: row.try_get("update_rule")?,
        delete_rule: row.try_get("delete_rule")?,
        auth_options,
        view_query: row.try_get("view_query")?,
        created: row.try_get("created")?,
        updated: row.try_get("updated")?,
    })
}

pub async fn list_collections(db: &Db) -> DbResult<Vec<Collection>> {
    let rows = sqlx::query("SELECT * FROM _collections ORDER BY name")
        .fetch_all(&db.pool)
        .await?;
    rows.iter().map(row_to_collection).collect()
}

pub async fn get_collection_by_id(db: &Db, id: &str) -> DbResult<Collection> {
    if let Some(c) = db.collections.get_by_id(id) {
        return Ok((*c).clone());
    }
    let row = sqlx::query("SELECT * FROM _collections WHERE id = $1")
        .bind(id)
        .fetch_optional(&db.pool)
        .await?
        .ok_or(DbError::NotFound)?;
    let collection = row_to_collection(&row)?;
    db.collections.put(collection.clone());
    Ok(collection)
}

pub async fn get_collection_by_name(db: &Db, name: &str) -> DbResult<Collection> {
    if let Some(c) = db.collections.get_by_name(name) {
        return Ok((*c).clone());
    }
    let row = sqlx::query("SELECT * FROM _collections WHERE name = $1")
        .bind(name)
        .fetch_optional(&db.pool)
        .await?
        .ok_or(DbError::NotFound)?;
    let collection = row_to_collection(&row)?;
    db.collections.put(collection.clone());
    Ok(collection)
}

/// Resolve a collection addressed by either its id or its name (every
/// `:collection` path param accepts both) with at most one query.
pub async fn get_collection_by_id_or_name(db: &Db, id_or_name: &str) -> DbResult<Collection> {
    if let Some(c) = db
        .collections
        .get_by_name(id_or_name)
        .or_else(|| db.collections.get_by_id(id_or_name))
    {
        return Ok((*c).clone());
    }
    let row = sqlx::query("SELECT * FROM _collections WHERE name = $1 OR id = $1")
        .bind(id_or_name)
        .fetch_optional(&db.pool)
        .await?
        .ok_or(DbError::NotFound)?;
    let collection = row_to_collection(&row)?;
    db.collections.put(collection.clone());
    Ok(collection)
}

/// Transaction-scoped counterpart to [`get_collection_by_id`]. Reads
/// through `tx` rather than the pool so a caller already holding the
/// pool's only checked-out connection (e.g. `/api/batch`, whose SQLite
/// test pool is sized 1) doesn't self-deadlock waiting to acquire a
/// second one.
pub async fn get_collection_by_id_tx(
    tx: &mut crate::records::RecordTx,
    id: &str,
) -> DbResult<Collection> {
    let row = sqlx::query("SELECT * FROM _collections WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(DbError::NotFound)?;
    row_to_collection(&row)
}

/// Transaction-scoped counterpart to [`get_collection_by_name`]. See
/// [`get_collection_by_id_tx`] for why this exists.
pub async fn get_collection_by_name_tx(
    tx: &mut crate::records::RecordTx,
    name: &str,
) -> DbResult<Collection> {
    let row = sqlx::query("SELECT * FROM _collections WHERE name = $1")
        .bind(name)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(DbError::NotFound)?;
    row_to_collection(&row)
}

pub async fn create_collection(db: &Db, collection: &Collection) -> DbResult<()> {
    let type_str = match collection.collection_type {
        CollectionType::Base => "base",
        CollectionType::Auth => "auth",
        CollectionType::View => "view",
    };
    let schema_json = serde_json::to_string(&collection.schema).unwrap();
    let auth_options_json = serde_json::to_string(&collection.auth_options).unwrap();

    sqlx::query(
        "INSERT INTO _collections
         (id, name, type, schema, list_rule, view_rule, create_rule, update_rule, delete_rule, auth_options, view_query, created, updated)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(&collection.id)
    .bind(&collection.name)
    .bind(type_str)
    .bind(&schema_json)
    .bind(&collection.list_rule)
    .bind(&collection.view_rule)
    .bind(&collection.create_rule)
    .bind(&collection.update_rule)
    .bind(&collection.delete_rule)
    .bind(&auth_options_json)
    .bind(&collection.view_query)
    .bind(&collection.created)
    .bind(&collection.updated)
    .execute(&db.pool)
    .await
    .map_err(|e| map_unique_violation(e, "name"))?;

    sync_table(db, collection, None).await?;
    db.collections.put(collection.clone());
    Ok(())
}

pub async fn update_collection(
    db: &Db,
    previous: &Collection,
    updated: &Collection,
) -> DbResult<()> {
    let type_str = match updated.collection_type {
        CollectionType::Base => "base",
        CollectionType::Auth => "auth",
        CollectionType::View => "view",
    };
    let schema_json = serde_json::to_string(&updated.schema).unwrap();
    let auth_options_json = serde_json::to_string(&updated.auth_options).unwrap();

    sqlx::query(
        "UPDATE _collections SET
            name = $1, type = $2, schema = $3, list_rule = $4, view_rule = $5,
            create_rule = $6, update_rule = $7, delete_rule = $8, auth_options = $9,
            view_query = $10, updated = $11
         WHERE id = $12",
    )
    .bind(&updated.name)
    .bind(type_str)
    .bind(&schema_json)
    .bind(&updated.list_rule)
    .bind(&updated.view_rule)
    .bind(&updated.create_rule)
    .bind(&updated.update_rule)
    .bind(&updated.delete_rule)
    .bind(&auth_options_json)
    .bind(&updated.view_query)
    .bind(&updated.updated)
    .bind(&updated.id)
    .execute(&db.pool)
    .await
    .map_err(|e| map_unique_violation(e, "name"))?;

    // Evict before the DDL rather than after: if `sync_table` fails
    // half-way, the next lookup re-reads the persisted row instead of
    // serving a stale schema.
    db.collections.evict_id(&updated.id);
    sync_table(db, updated, Some(previous)).await?;
    db.collections.put(updated.clone());
    Ok(())
}

pub async fn delete_collection(db: &Db, collection: &Collection) -> DbResult<()> {
    db.collections.evict_id(&collection.id);
    sqlx::query("DELETE FROM _collections WHERE id = $1")
        .bind(&collection.id)
        .execute(&db.pool)
        .await?;
    let table = db.backend.quote_ident(&collection.table_name())?;
    let drop_sql = if collection.collection_type == CollectionType::View {
        format!("DROP VIEW IF EXISTS {table}")
    } else {
        format!("DROP TABLE IF EXISTS {table}")
    };
    sqlx::query(&drop_sql).execute(&db.pool).await?;
    Ok(())
}

fn map_unique_violation(e: sqlx::Error, field: &str) -> DbError {
    if let sqlx::Error::Database(db_err) = &e {
        if db_err.is_unique_violation() {
            return DbError::UniqueViolation(field.to_string());
        }
    }
    DbError::Sqlx(e)
}

/// The physical storage "shape" a field occupies. Changing shape (e.g.
/// text -> number, or single -> multiple) requires dropping and recreating
/// the column, which clears any data already stored in it.
fn physical_kind(field: &Field) -> &'static str {
    let multiple = field.field_type.supports_multiple() && field.options.multiple.unwrap_or(false);
    if multiple {
        return "text";
    }
    match field.field_type {
        FieldType::Number => "number",
        FieldType::Bool => "bool",
        _ => "text",
    }
}

fn sql_type_for(backend: Backend, field: &Field) -> &'static str {
    match physical_kind(field) {
        "number" => backend.number_type(),
        "bool" => backend.bool_type(),
        _ => backend.text_type(),
    }
}

/// Create or incrementally alter the physical table backing `collection` so
/// its columns match `collection.schema`. New fields get `ADD COLUMN`;
/// removed fields get `DROP COLUMN`; fields whose physical shape changed are
/// dropped and re-added (data loss on that column only — schema changes to
/// a field's type or multiplicity are inherently destructive without a full
/// migration/backfill system, which is out of scope for v1).
///
/// `View` collections take a different path entirely: instead of a table,
/// `cb_<name>` is created as a real SQL `VIEW` over `collection.view_query`.
/// Every list/filter/sort/pagination code path in `records::list_records`/
/// `get_record` runs unmodified against it — a view is queryable exactly
/// like a table for `SELECT`. Recreated unconditionally (DROP + CREATE) on
/// every call since `view_query` may have changed and there's no portable
/// `CREATE OR REPLACE VIEW` across backends (SQLite has no such syntax).
pub async fn sync_table(
    db: &Db,
    collection: &Collection,
    previous: Option<&Collection>,
) -> DbResult<()> {
    let backend = db.backend;
    let table = backend.quote_ident(&collection.table_name())?;

    if collection.collection_type == CollectionType::View {
        let query = collection.view_query.as_deref().ok_or_else(|| {
            DbError::InvalidIdentifier("view collection is missing 'view_query'".into())
        })?;
        sqlx::query(&format!("DROP VIEW IF EXISTS {table}"))
            .execute(&db.pool)
            .await?;
        sqlx::query(&format!("CREATE VIEW {table} AS {query}"))
            .execute(&db.pool)
            .await?;
        return Ok(());
    }

    match previous {
        None => {
            let mut cols = vec![
                format!("{} TEXT PRIMARY KEY", backend.quote_ident("id")?),
                format!("{} TEXT NOT NULL", backend.quote_ident("created")?),
                format!("{} TEXT NOT NULL", backend.quote_ident("updated")?),
            ];
            if collection.is_auth() {
                cols.push(format!(
                    "{} TEXT NOT NULL",
                    backend.quote_ident(collection.auth_options.identity_field())?
                ));
                cols.push(format!(
                    "{} TEXT NOT NULL",
                    backend.quote_ident("password_hash")?
                ));
                cols.push(format!(
                    "{} {} NOT NULL",
                    backend.quote_ident("verified")?,
                    backend.bool_type()
                ));
            }
            for f in &collection.schema {
                cols.push(format!(
                    "{} {}",
                    backend.quote_ident(&f.name)?,
                    sql_type_for(backend, f)
                ));
            }
            let sql = format!("CREATE TABLE IF NOT EXISTS {table} ({})", cols.join(", "));
            sqlx::query(&sql).execute(&db.pool).await?;
            if collection.is_auth() {
                let identity = collection.auth_options.identity_field();
                let idx = backend.quote_ident(&format!("idx_{}_{identity}", collection.name))?;
                let identity_col = backend.quote_ident(identity)?;
                sqlx::query(&format!(
                    "CREATE UNIQUE INDEX IF NOT EXISTS {idx} ON {table} ({identity_col})"
                ))
                .execute(&db.pool)
                .await?;
            }
        }
        Some(prev) => {
            for f in &collection.schema {
                match prev.field(&f.name) {
                    None => {
                        let sql = format!(
                            "ALTER TABLE {table} ADD COLUMN {} {}",
                            backend.quote_ident(&f.name)?,
                            sql_type_for(backend, f)
                        );
                        sqlx::query(&sql).execute(&db.pool).await?;
                    }
                    Some(pf) if physical_kind(pf) != physical_kind(f) => {
                        let drop_sql = format!(
                            "ALTER TABLE {table} DROP COLUMN {}",
                            backend.quote_ident(&f.name)?
                        );
                        sqlx::query(&drop_sql).execute(&db.pool).await?;
                        let add_sql = format!(
                            "ALTER TABLE {table} ADD COLUMN {} {}",
                            backend.quote_ident(&f.name)?,
                            sql_type_for(backend, f)
                        );
                        sqlx::query(&add_sql).execute(&db.pool).await?;
                    }
                    Some(_) => {}
                }
            }
            for pf in &prev.schema {
                if collection.field(&pf.name).is_none() {
                    let sql = format!(
                        "ALTER TABLE {table} DROP COLUMN {}",
                        backend.quote_ident(&pf.name)?
                    );
                    sqlx::query(&sql).execute(&db.pool).await?;
                }
            }
        }
    }

    // Every list request defaults to `ORDER BY created DESC` (see
    // `records::build_order_by`); without an index that is a full scan
    // into a sorter per request, growing superlinearly with table size.
    let created_idx = backend.quote_ident(&format!("idx_{}_created", collection.name))?;
    let created_col = backend.quote_ident("created")?;
    sqlx::query(&format!(
        "CREATE INDEX IF NOT EXISTS {created_idx} ON {table} ({created_col} DESC)"
    ))
    .execute(&db.pool)
    .await?;

    for f in &collection.schema {
        let idx_name = format!("idx_{}_{}", collection.name, f.name);
        let idx = backend.quote_ident(&idx_name)?;
        if f.unique {
            let sql = format!(
                "CREATE UNIQUE INDEX IF NOT EXISTS {idx} ON {table} ({})",
                backend.quote_ident(&f.name)?
            );
            sqlx::query(&sql).execute(&db.pool).await?;
        } else {
            let sql = format!("DROP INDEX IF EXISTS {idx}");
            sqlx::query(&sql).execute(&db.pool).await?;
        }
    }

    Ok(())
}
