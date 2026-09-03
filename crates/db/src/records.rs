use cratebase_core::field::FieldType;
use cratebase_core::{new_id, now, Collection};
use cratebase_filter::CompiledFilter;
use serde_json::{Map, Value};
use sqlx::any::{AnyArguments, AnyRow};
use sqlx::{Arguments, Row};

use crate::backend::Backend;
use crate::error::{DbError, DbResult};
use crate::pool::Db;
use crate::resolver::{CollectionResolver, RequestContext};
use crate::value::{bind_filter_value, ColumnValue};

pub struct ListParams<'a> {
    pub filter: Option<&'a str>,
    pub sort: Option<&'a str>,
    pub page: i64,
    pub per_page: i64,
}

pub struct ListResult {
    pub items: Vec<Value>,
    pub page: i64,
    pub per_page: i64,
    pub total_items: i64,
    pub total_pages: i64,
}

fn is_multiple(field: &cratebase_core::Field) -> bool {
    field.field_type.supports_multiple() && field.options.multiple.unwrap_or(false)
}

fn row_to_record(row: &AnyRow, collection: &Collection) -> DbResult<Value> {
    let mut obj = Map::new();
    obj.insert("id".into(), Value::String(row.try_get("id")?));
    obj.insert("created".into(), Value::String(row.try_get("created")?));
    obj.insert("updated".into(), Value::String(row.try_get("updated")?));
    obj.insert("collectionId".into(), Value::String(collection.id.clone()));
    obj.insert(
        "collectionName".into(),
        Value::String(collection.name.clone()),
    );
    if collection.is_auth() {
        let identity = collection.auth_options.identity_field();
        let value: Option<String> = row.try_get(identity)?;
        obj.insert(
            identity.to_string(),
            value.map(Value::String).unwrap_or(Value::Null),
        );
        let verified: Option<i64> = row.try_get("verified")?;
        obj.insert(
            "verified".into(),
            Value::Bool(verified.map(|n| n != 0).unwrap_or(false)),
        );
    }

    for field in &collection.schema {
        let multiple = is_multiple(field);
        let column = if multiple {
            let s: Option<String> = row.try_get(field.name.as_str())?;
            ColumnValue::Text(s)
        } else {
            match field.field_type {
                FieldType::Number => {
                    let n: Option<f64> = row.try_get(field.name.as_str())?;
                    ColumnValue::Number(n)
                }
                FieldType::Bool => {
                    let b: Option<i64> = row.try_get(field.name.as_str())?;
                    ColumnValue::Bool(b.map(|n| n != 0))
                }
                _ => {
                    let s: Option<String> = row.try_get(field.name.as_str())?;
                    ColumnValue::Text(s)
                }
            }
        };
        obj.insert(
            field.name.clone(),
            column.to_json(field.field_type, multiple),
        );
    }

    Ok(Value::Object(obj))
}

/// Parse `sort=-created,name` into an `ORDER BY` clause. Unknown fields are
/// rejected rather than silently ignored so typos surface immediately.
fn build_order_by(
    collection: &Collection,
    backend: Backend,
    sort: Option<&str>,
) -> DbResult<String> {
    let sort = sort.unwrap_or("-created");
    let mut parts = Vec::new();
    for token in sort.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let (name, desc) = match token.strip_prefix('-') {
            Some(rest) => (rest, true),
            None => (token, false),
        };
        if name != "id"
            && name != "created"
            && name != "updated"
            && collection.field(name).is_none()
        {
            return Err(DbError::InvalidIdentifier(format!(
                "unknown sort field '{name}'"
            )));
        }
        let quoted = backend.quote_ident(name)?;
        parts.push(format!("{quoted} {}", if desc { "DESC" } else { "ASC" }));
    }
    if parts.is_empty() {
        parts.push(format!("{} DESC", backend.quote_ident("created")?));
    }
    Ok(parts.join(", "))
}

fn encode_err(e: sqlx::error::BoxDynError) -> DbError {
    DbError::Sqlx(sqlx::Error::Encode(e))
}

async fn bind_all<'q>(params: Vec<Value>) -> DbResult<AnyArguments<'q>> {
    let mut args = AnyArguments::default();
    for p in params {
        bind_filter_value(&mut args, p).map_err(encode_err)?;
    }
    Ok(args)
}

fn map_unique_violation(e: sqlx::Error) -> DbError {
    if let sqlx::Error::Database(db_err) = &e {
        if db_err.is_unique_violation() {
            return DbError::UniqueViolation(
                db_err
                    .constraint()
                    .map(str::to_string)
                    .unwrap_or_else(|| "unique".to_string()),
            );
        }
    }
    DbError::Sqlx(e)
}

async fn fetch_by_id(db: &Db, collection: &Collection, id: &str) -> DbResult<Value> {
    let table = db.backend.quote_ident(&collection.table_name())?;
    let id_col = db.backend.quote_ident("id")?;
    let sql = format!("SELECT * FROM {table} WHERE {id_col} = $1");
    let row = sqlx::query(&sql).bind(id).fetch_optional(&db.pool).await?;
    match row {
        Some(r) => row_to_record(&r, collection),
        None => Err(DbError::NotFound),
    }
}

/// Total row count for a collection's table, no filter/pagination. Used by
/// the stats/metrics extension point rather than the paginated list path.
pub async fn count_records(db: &Db, collection: &Collection) -> DbResult<i64> {
    let table = db.backend.quote_ident(&collection.table_name())?;
    let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(&db.pool)
        .await?;
    Ok(count)
}

pub async fn list_records(
    db: &Db,
    collection: &Collection,
    ctx: &RequestContext,
    rule_filter: Option<CompiledFilter>,
    params: ListParams<'_>,
) -> DbResult<ListResult> {
    let table = db.backend.quote_ident(&collection.table_name())?;
    let offset_so_far = rule_filter.as_ref().map(|f| f.params.len()).unwrap_or(0);

    let user_filter = match params.filter {
        Some(expr) if !expr.trim().is_empty() => {
            let related = crate::resolver::load_related_collections(db, collection, expr).await?;
            let resolver = CollectionResolver {
                collection,
                backend: db.backend,
                ctx,
                use_data_for_fields: false,
                related,
            };
            Some(cratebase_filter::parse_and_compile(
                expr,
                &resolver,
                db.backend.dialect(),
                offset_so_far,
            )?)
        }
        _ => None,
    };

    let mut clauses = Vec::new();
    let mut params_vec = Vec::new();
    if let Some(f) = rule_filter {
        clauses.push(f.sql);
        params_vec.extend(f.params);
    }
    if let Some(f) = user_filter {
        clauses.push(f.sql);
        params_vec.extend(f.params);
    }
    let where_clause = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    let order_by = build_order_by(collection, db.backend, params.sort)?;

    let count_sql = format!("SELECT COUNT(*) FROM {table}{where_clause}");
    let count_args = bind_all(params_vec.clone()).await?;
    let total_items: i64 = sqlx::query_scalar_with(&count_sql, count_args)
        .fetch_one(&db.pool)
        .await?;

    let per_page = params.per_page.clamp(1, 500);
    let page = params.page.max(1);
    let offset = (page - 1) * per_page;

    let n = params_vec.len();
    let list_sql = format!(
        "SELECT * FROM {table}{where_clause} ORDER BY {order_by} LIMIT ${} OFFSET ${}",
        n + 1,
        n + 2
    );
    let mut list_args = bind_all(params_vec).await?;
    list_args.add(per_page).map_err(encode_err)?;
    list_args.add(offset).map_err(encode_err)?;

    let rows = sqlx::query_with(&list_sql, list_args)
        .fetch_all(&db.pool)
        .await?;
    let items = rows
        .iter()
        .map(|r| row_to_record(r, collection))
        .collect::<DbResult<Vec<_>>>()?;

    let total_pages = if total_items == 0 {
        0
    } else {
        (total_items + per_page - 1) / per_page
    };

    Ok(ListResult {
        items,
        page,
        per_page,
        total_items,
        total_pages,
    })
}

/// Fetch a single record, applying the caller's `viewRule` as an additional
/// WHERE condition so a record the rule denies looks identical to one that
/// doesn't exist.
pub async fn get_record(
    db: &Db,
    collection: &Collection,
    id: &str,
    rule_filter: Option<CompiledFilter>,
) -> DbResult<Value> {
    let table = db.backend.quote_ident(&collection.table_name())?;
    let id_col = db.backend.quote_ident("id")?;

    let n = rule_filter.as_ref().map(|f| f.params.len()).unwrap_or(0);
    let extra_sql = rule_filter.as_ref().map(|f| f.sql.clone());
    let mut args = bind_all(rule_filter.map(|f| f.params).unwrap_or_default()).await?;
    args.add(id.to_string()).map_err(encode_err)?;

    let extra_clause = extra_sql.map(|s| format!(" AND {s}")).unwrap_or_default();
    let sql = format!(
        "SELECT * FROM {table} WHERE {id_col} = ${}{extra_clause}",
        n + 1
    );

    let row = sqlx::query_with(&sql, args)
        .fetch_optional(&db.pool)
        .await?;
    match row {
        Some(r) => row_to_record(&r, collection),
        None => Err(DbError::NotFound),
    }
}

/// Transaction-scoped counterpart to [`get_record`]. Reads through `tx`
/// rather than the pool — see [`crate::collections::get_collection_by_id_tx`]
/// for why this exists.
pub async fn get_record_tx(
    tx: &mut RecordTx,
    backend: Backend,
    collection: &Collection,
    id: &str,
    rule_filter: Option<CompiledFilter>,
) -> DbResult<Value> {
    let table = backend.quote_ident(&collection.table_name())?;
    let id_col = backend.quote_ident("id")?;

    let n = rule_filter.as_ref().map(|f| f.params.len()).unwrap_or(0);
    let extra_sql = rule_filter.as_ref().map(|f| f.sql.clone());
    let mut args = bind_all(rule_filter.map(|f| f.params).unwrap_or_default()).await?;
    args.add(id.to_string()).map_err(encode_err)?;

    let extra_clause = extra_sql.map(|s| format!(" AND {s}")).unwrap_or_default();
    let sql = format!(
        "SELECT * FROM {table} WHERE {id_col} = ${}{extra_clause}",
        n + 1
    );

    let row = sqlx::query_with(&sql, args)
        .fetch_optional(&mut **tx)
        .await?;
    match row {
        Some(r) => row_to_record(&r, collection),
        None => Err(DbError::NotFound),
    }
}

pub async fn create_record(
    db: &Db,
    collection: &Collection,
    data: Map<String, Value>,
) -> DbResult<Value> {
    create_record_with_id(db, collection, new_id(), data).await
}

/// Set every `Autodate` field's stored value to `ts`, for the fields
/// configured to fire on this lifecycle event (`onCreate` or `onUpdate`).
/// Runs after `validate::validate_and_normalize`, which never lets a
/// client-supplied value reach `normalized` for this field type — the
/// value here is always server-computed.
fn apply_autodate_fields(
    collection: &Collection,
    normalized: &mut Map<String, Value>,
    ts: &str,
    on_create: bool,
) {
    for field in &collection.schema {
        if field.field_type != FieldType::Autodate {
            continue;
        }
        let fires = if on_create {
            field.options.on_create.unwrap_or(false)
        } else {
            field.options.on_update.unwrap_or(false)
        };
        if fires {
            normalized.insert(field.name.clone(), Value::String(ts.to_string()));
        }
    }
}

/// Like [`create_record`] but with a caller-chosen id. Used when the id
/// must be known before the row exists — e.g. uploaded files are stored
/// under a key derived from the record id, so the server layer generates
/// the id up front, uploads to storage, then creates the row with that
/// same id.
pub async fn create_record_with_id(
    db: &Db,
    collection: &Collection,
    id: String,
    data: Map<String, Value>,
) -> DbResult<Value> {
    if collection.is_view() {
        return Err(DbError::ViewReadOnly);
    }
    let mut normalized =
        crate::validate::validate_and_normalize(db, collection, &data, false).await?;

    let ts = now();
    apply_autodate_fields(collection, &mut normalized, &ts, true);
    let table = db.backend.quote_ident(&collection.table_name())?;

    let mut columns = vec![
        "id".to_string(),
        "created".to_string(),
        "updated".to_string(),
    ];
    let mut args = AnyArguments::default();
    args.add(id.clone()).map_err(encode_err)?;
    args.add(ts.clone()).map_err(encode_err)?;
    args.add(ts).map_err(encode_err)?;

    // `email`/`password_hash` are physical columns on every Auth-typed
    // collection's table (see `collections::sync_table`) but are not part
    // of its user-editable `schema`, so they're pulled from the raw
    // request `data` here rather than the schema-validated `normalized`
    // map. The server layer is responsible for hashing the password and
    // validating the email before it reaches this function.
    if collection.is_auth() {
        let identity = collection.auth_options.identity_field();
        if let Some(value) = data.get(identity).and_then(Value::as_str) {
            columns.push(identity.to_string());
            ColumnValue::Text(Some(value.to_string()))
                .bind(&mut args)
                .map_err(encode_err)?;
        }
        if let Some(hash) = data.get("password_hash").and_then(Value::as_str) {
            columns.push("password_hash".to_string());
            ColumnValue::Text(Some(hash.to_string()))
                .bind(&mut args)
                .map_err(encode_err)?;
        }
        // New accounts always start unverified — never client-settable —
        // regardless of whether `requireEmailVerification` currently
        // gates login, so flipping that setting later needs no backfill.
        columns.push("verified".to_string());
        ColumnValue::Bool(Some(false))
            .bind(&mut args)
            .map_err(encode_err)?;
    }

    for field in &collection.schema {
        let value = normalized.get(&field.name).cloned().unwrap_or(Value::Null);
        let multiple = is_multiple(field);
        let column = ColumnValue::from_json(field.field_type, multiple, &value);
        columns.push(field.name.clone());
        column.bind(&mut args).map_err(encode_err)?;
    }

    let quoted_cols: DbResult<Vec<String>> =
        columns.iter().map(|c| db.backend.quote_ident(c)).collect();
    let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("${i}")).collect();
    let sql = format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        quoted_cols?.join(", "),
        placeholders.join(", ")
    );
    sqlx::query_with(&sql, args)
        .execute(&db.pool)
        .await
        .map_err(map_unique_violation)?;

    fetch_by_id(db, collection, &id).await
}

pub async fn update_record(
    db: &Db,
    collection: &Collection,
    id: &str,
    data: Map<String, Value>,
) -> DbResult<Value> {
    if collection.is_view() {
        return Err(DbError::ViewReadOnly);
    }
    let mut normalized =
        crate::validate::validate_and_normalize(db, collection, &data, true).await?;
    let ts = now();
    apply_autodate_fields(collection, &mut normalized, &ts, false);

    let identity = collection.auth_options.identity_field();
    let auth_identity = if collection.is_auth() {
        data.get(identity).and_then(Value::as_str)
    } else {
        None
    };
    let auth_password_hash = if collection.is_auth() {
        data.get("password_hash").and_then(Value::as_str)
    } else {
        None
    };
    if normalized.is_empty() && auth_identity.is_none() && auth_password_hash.is_none() {
        return fetch_by_id(db, collection, id).await;
    }

    let table = db.backend.quote_ident(&collection.table_name())?;
    let mut sets = vec![format!("{} = $1", db.backend.quote_ident("updated")?)];
    let mut args = AnyArguments::default();
    args.add(ts).map_err(encode_err)?;

    let mut idx = 2;
    if let Some(value) = auth_identity {
        sets.push(format!("{} = ${idx}", db.backend.quote_ident(identity)?));
        args.add(value.to_string()).map_err(encode_err)?;
        idx += 1;
    }
    if let Some(hash) = auth_password_hash {
        sets.push(format!(
            "{} = ${idx}",
            db.backend.quote_ident("password_hash")?
        ));
        args.add(hash.to_string()).map_err(encode_err)?;
        idx += 1;
    }
    for field in &collection.schema {
        if let Some(value) = normalized.get(&field.name) {
            let multiple = is_multiple(field);
            let column = ColumnValue::from_json(field.field_type, multiple, value);
            sets.push(format!("{} = ${idx}", db.backend.quote_ident(&field.name)?));
            column.bind(&mut args).map_err(encode_err)?;
            idx += 1;
        }
    }
    args.add(id.to_string()).map_err(encode_err)?;

    let sql = format!(
        "UPDATE {table} SET {} WHERE {} = ${idx}",
        sets.join(", "),
        db.backend.quote_ident("id")?
    );
    let result = sqlx::query_with(&sql, args)
        .execute(&db.pool)
        .await
        .map_err(map_unique_violation)?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }

    fetch_by_id(db, collection, id).await
}

pub async fn delete_record(db: &Db, collection: &Collection, id: &str) -> DbResult<()> {
    if collection.is_view() {
        return Err(DbError::ViewReadOnly);
    }
    let table = db.backend.quote_ident(&collection.table_name())?;
    let sql = format!(
        "DELETE FROM {table} WHERE {} = $1",
        db.backend.quote_ident("id")?
    );
    let result = sqlx::query(&sql).bind(id).execute(&db.pool).await?;
    if result.rows_affected() == 0 {
        Err(DbError::NotFound)
    } else {
        Ok(())
    }
}

/// Look up an auth-record's id, password hash, and verified status by its
/// identity field (e.g. email or username), for the password login
/// endpoint. One query instead of an id lookup followed by separate field
/// lookups.
pub async fn find_auth_credentials(
    db: &Db,
    collection: &Collection,
    identity_value: &str,
) -> DbResult<Option<(String, String, bool)>> {
    let table = db.backend.quote_ident(&collection.table_name())?;
    let sql = format!(
        "SELECT {}, {}, {} FROM {table} WHERE {} = $1",
        db.backend.quote_ident("id")?,
        db.backend.quote_ident("password_hash")?,
        db.backend.quote_ident("verified")?,
        db.backend
            .quote_ident(collection.auth_options.identity_field())?,
    );
    let row = sqlx::query(&sql)
        .bind(identity_value)
        .fetch_optional(&db.pool)
        .await?;
    Ok(match row {
        Some(r) => {
            let verified: i64 = r.try_get("verified")?;
            Some((r.try_get("id")?, r.try_get("password_hash")?, verified != 0))
        }
        None => None,
    })
}

/// Flips an auth record's `verified` column to `true`. Not a `schema`
/// field (same reason `password_hash` isn't), so it bypasses
/// `update_record`'s normal validated-fields path — used only by the
/// `confirm-verification` endpoint after checking a `VerifyEmail` token.
pub async fn set_verified(db: &Db, collection: &Collection, id: &str) -> DbResult<()> {
    let table = db.backend.quote_ident(&collection.table_name())?;
    let sql = format!(
        "UPDATE {table} SET {} = $1 WHERE {} = $2",
        db.backend.quote_ident("verified")?,
        db.backend.quote_ident("id")?
    );
    let mut args = AnyArguments::default();
    ColumnValue::Bool(Some(true))
        .bind(&mut args)
        .map_err(encode_err)?;
    args.add(id.to_string()).map_err(encode_err)?;
    let result = sqlx::query_with(&sql, args).execute(&db.pool).await?;
    if result.rows_affected() == 0 {
        Err(DbError::NotFound)
    } else {
        Ok(())
    }
}

/// A [`sqlx::Any`] transaction obtained from [`crate::pool::Db::pool`].
/// `AnyPool::begin()` yields `'static` because the transaction owns its
/// pooled connection outright, so this alias needs no lifetime parameter.
pub type RecordTx = sqlx::Transaction<'static, sqlx::Any>;

/// Transaction-scoped counterpart to [`fetch_by_id`]. Reads through `tx`
/// rather than the pool so a caller can read back a row it just wrote in
/// the same uncommitted transaction (e.g. the `/api/batch` endpoint).
async fn fetch_by_id_tx(
    tx: &mut RecordTx,
    backend: Backend,
    collection: &Collection,
    id: &str,
) -> DbResult<Value> {
    let table = backend.quote_ident(&collection.table_name())?;
    let id_col = backend.quote_ident("id")?;
    let sql = format!("SELECT * FROM {table} WHERE {id_col} = $1");
    let row = sqlx::query(&sql).bind(id).fetch_optional(&mut **tx).await?;
    match row {
        Some(r) => row_to_record(&r, collection),
        None => Err(DbError::NotFound),
    }
}

/// Transaction-scoped counterpart to [`create_record_with_id`], used by the
/// `/api/batch` endpoint so every sub-request's write lands on the same
/// connection inside one SQL transaction: if a later sub-request fails,
/// dropping `tx` without committing undoes this insert along with every
/// other write already made through it in the batch.
///
/// Unlike [`create_record_with_id`], this does not call
/// `validate::validate_and_normalize` itself — the caller runs that (and
/// rule evaluation) against the pool *before* opening the transaction, so
/// only the actual row mutation is transactional. `data` is the raw
/// (auth-prepared) payload, needed for the `email`/`password_hash`
/// physical columns exactly as in `create_record_with_id`; `normalized` is
/// its already-validated schema-field subset.
pub async fn create_record_with_id_tx(
    tx: &mut RecordTx,
    backend: Backend,
    collection: &Collection,
    id: String,
    data: Map<String, Value>,
    mut normalized: Map<String, Value>,
) -> DbResult<Value> {
    let ts = now();
    apply_autodate_fields(collection, &mut normalized, &ts, true);
    let table = backend.quote_ident(&collection.table_name())?;

    let mut columns = vec![
        "id".to_string(),
        "created".to_string(),
        "updated".to_string(),
    ];
    let mut args = AnyArguments::default();
    args.add(id.clone()).map_err(encode_err)?;
    args.add(ts.clone()).map_err(encode_err)?;
    args.add(ts).map_err(encode_err)?;

    if collection.is_auth() {
        let identity = collection.auth_options.identity_field();
        if let Some(value) = data.get(identity).and_then(Value::as_str) {
            columns.push(identity.to_string());
            ColumnValue::Text(Some(value.to_string()))
                .bind(&mut args)
                .map_err(encode_err)?;
        }
        if let Some(hash) = data.get("password_hash").and_then(Value::as_str) {
            columns.push("password_hash".to_string());
            ColumnValue::Text(Some(hash.to_string()))
                .bind(&mut args)
                .map_err(encode_err)?;
        }
        columns.push("verified".to_string());
        ColumnValue::Bool(Some(false))
            .bind(&mut args)
            .map_err(encode_err)?;
    }

    for field in &collection.schema {
        let value = normalized.get(&field.name).cloned().unwrap_or(Value::Null);
        let multiple = is_multiple(field);
        let column = ColumnValue::from_json(field.field_type, multiple, &value);
        columns.push(field.name.clone());
        column.bind(&mut args).map_err(encode_err)?;
    }

    let quoted_cols: DbResult<Vec<String>> =
        columns.iter().map(|c| backend.quote_ident(c)).collect();
    let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("${i}")).collect();
    let sql = format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        quoted_cols?.join(", "),
        placeholders.join(", ")
    );
    sqlx::query_with(&sql, args)
        .execute(&mut **tx)
        .await
        .map_err(map_unique_violation)?;

    fetch_by_id_tx(tx, backend, collection, &id).await
}

/// Transaction-scoped counterpart to [`update_record`] — see
/// [`create_record_with_id_tx`] for why validation happens outside `tx`.
pub async fn update_record_tx(
    tx: &mut RecordTx,
    backend: Backend,
    collection: &Collection,
    id: &str,
    data: Map<String, Value>,
    mut normalized: Map<String, Value>,
) -> DbResult<Value> {
    let ts = now();
    apply_autodate_fields(collection, &mut normalized, &ts, false);

    let identity = collection.auth_options.identity_field();
    let auth_identity = if collection.is_auth() {
        data.get(identity).and_then(Value::as_str)
    } else {
        None
    };
    let auth_password_hash = if collection.is_auth() {
        data.get("password_hash").and_then(Value::as_str)
    } else {
        None
    };
    if normalized.is_empty() && auth_identity.is_none() && auth_password_hash.is_none() {
        return fetch_by_id_tx(tx, backend, collection, id).await;
    }

    let table = backend.quote_ident(&collection.table_name())?;
    let mut sets = vec![format!("{} = $1", backend.quote_ident("updated")?)];
    let mut args = AnyArguments::default();
    args.add(ts).map_err(encode_err)?;

    let mut idx = 2;
    if let Some(value) = auth_identity {
        sets.push(format!("{} = ${idx}", backend.quote_ident(identity)?));
        args.add(value.to_string()).map_err(encode_err)?;
        idx += 1;
    }
    if let Some(hash) = auth_password_hash {
        sets.push(format!("{} = ${idx}", backend.quote_ident("password_hash")?));
        args.add(hash.to_string()).map_err(encode_err)?;
        idx += 1;
    }
    for field in &collection.schema {
        if let Some(value) = normalized.get(&field.name) {
            let multiple = is_multiple(field);
            let column = ColumnValue::from_json(field.field_type, multiple, value);
            sets.push(format!("{} = ${idx}", backend.quote_ident(&field.name)?));
            column.bind(&mut args).map_err(encode_err)?;
            idx += 1;
        }
    }
    args.add(id.to_string()).map_err(encode_err)?;

    let sql = format!(
        "UPDATE {table} SET {} WHERE {} = ${idx}",
        sets.join(", "),
        backend.quote_ident("id")?
    );
    let result = sqlx::query_with(&sql, args)
        .execute(&mut **tx)
        .await
        .map_err(map_unique_violation)?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }

    fetch_by_id_tx(tx, backend, collection, id).await
}

/// Transaction-scoped counterpart to [`delete_record`].
pub async fn delete_record_tx(
    tx: &mut RecordTx,
    backend: Backend,
    collection: &Collection,
    id: &str,
) -> DbResult<()> {
    let table = backend.quote_ident(&collection.table_name())?;
    let sql = format!(
        "DELETE FROM {table} WHERE {} = $1",
        backend.quote_ident("id")?
    );
    let result = sqlx::query(&sql).bind(id).execute(&mut **tx).await?;
    if result.rows_affected() == 0 {
        Err(DbError::NotFound)
    } else {
        Ok(())
    }
}
