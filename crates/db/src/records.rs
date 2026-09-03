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
    obj.insert("collectionName".into(), Value::String(collection.name.clone()));

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
        obj.insert(field.name.clone(), column.to_json(field.field_type, multiple));
    }

    Ok(Value::Object(obj))
}

/// Parse `sort=-created,name` into an `ORDER BY` clause. Unknown fields are
/// rejected rather than silently ignored so typos surface immediately.
fn build_order_by(collection: &Collection, backend: Backend, sort: Option<&str>) -> DbResult<String> {
    let sort = sort.unwrap_or("-created");
    let mut parts = Vec::new();
    for token in sort.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let (name, desc) = match token.strip_prefix('-') {
            Some(rest) => (rest, true),
            None => (token, false),
        };
        if name != "id" && name != "created" && name != "updated" && collection.field(name).is_none() {
            return Err(DbError::InvalidIdentifier(format!("unknown sort field '{name}'")));
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
            let resolver = CollectionResolver {
                collection,
                backend: db.backend,
                ctx,
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

    let rows = sqlx::query_with(&list_sql, list_args).fetch_all(&db.pool).await?;
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
    let sql = format!("SELECT * FROM {table} WHERE {id_col} = ${}{extra_clause}", n + 1);

    let row = sqlx::query_with(&sql, args).fetch_optional(&db.pool).await?;
    match row {
        Some(r) => row_to_record(&r, collection),
        None => Err(DbError::NotFound),
    }
}

pub async fn create_record(db: &Db, collection: &Collection, data: Map<String, Value>) -> DbResult<Value> {
    let normalized = crate::validate::validate_and_normalize(db, collection, &data, false).await?;

    let id = new_id();
    let ts = now();
    let table = db.backend.quote_ident(&collection.table_name())?;

    let mut columns = vec!["id".to_string(), "created".to_string(), "updated".to_string()];
    let mut args = AnyArguments::default();
    args.add(id.clone()).map_err(encode_err)?;
    args.add(ts.clone()).map_err(encode_err)?;
    args.add(ts).map_err(encode_err)?;

    for field in &collection.schema {
        let value = normalized.get(&field.name).cloned().unwrap_or(Value::Null);
        let multiple = is_multiple(field);
        let column = ColumnValue::from_json(field.field_type, multiple, &value);
        columns.push(field.name.clone());
        column.bind(&mut args).map_err(encode_err)?;
    }

    let quoted_cols: DbResult<Vec<String>> = columns.iter().map(|c| db.backend.quote_ident(c)).collect();
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
    let normalized = crate::validate::validate_and_normalize(db, collection, &data, true).await?;
    if normalized.is_empty() {
        return fetch_by_id(db, collection, id).await;
    }

    let table = db.backend.quote_ident(&collection.table_name())?;
    let ts = now();
    let mut sets = vec![format!("{} = $1", db.backend.quote_ident("updated")?)];
    let mut args = AnyArguments::default();
    args.add(ts).map_err(encode_err)?;

    let mut idx = 2;
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
    let table = db.backend.quote_ident(&collection.table_name())?;
    let sql = format!("DELETE FROM {table} WHERE {} = $1", db.backend.quote_ident("id")?);
    let result = sqlx::query(&sql).bind(id).execute(&db.pool).await?;
    if result.rows_affected() == 0 {
        Err(DbError::NotFound)
    } else {
        Ok(())
    }
}
