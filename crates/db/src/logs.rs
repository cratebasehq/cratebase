//! `_logs` in the auxiliary database: request and application log rows
//! in PocketBase's shape (`{id, level, message, data, created}`).
//!
//! Filters arrive pre-compiled as `(where_sql, params)` with `$n`
//! placeholders; the service layer compiles PocketBase filter
//! expressions against the virtual schema (`level`, `message`,
//! `data.*`, `created`) before calling in here.

use cratebase_core::{record_id, DateTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::{Executor, Row, Sql};
use crate::error::{DbError, DbResult};

/// Rows per multi-row `INSERT`: 6 columns × 256 rows stays far below
/// SQLite's 32766 bound-parameter limit.
pub const BATCH_MAX: usize = 256;

/// Columns a sort expression may name.
pub const SORTABLE: &[&str] = &["id", "level", "message", "created", "updated", "rowid"];

const DEFAULT_SORT: &str = "-created";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: String,
    pub level: i64,
    pub message: String,
    pub data: Value,
    pub created: DateTime,
    /// Kept in the table for symmetry with every other system table;
    /// PocketBase does not expose it in the API, so neither do we.
    #[serde(skip)]
    pub updated: DateTime,
}

impl LogEntry {
    /// A new entry stamped now with a fresh record id.
    pub fn new(level: i64, message: impl Into<String>, data: Value) -> Self {
        let now = DateTime::now();
        LogEntry {
            id: record_id(),
            level,
            message: message.into(),
            data,
            created: now,
            updated: now,
        }
    }
}

/// A compiled `WHERE` fragment (without the keyword) and its bound
/// parameters, `$1`-numbered from one.
pub type Filter = (String, Vec<Sql>);

#[derive(Debug, Clone, Default)]
pub struct ListParams {
    pub page: i64,
    pub per_page: i64,
    pub filter: Option<Filter>,
    /// PocketBase sort syntax over [`SORTABLE`] columns:
    /// `-created,level`. Defaults to `-created`.
    pub sort: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    pub items: Vec<LogEntry>,
    pub page: i64,
    pub per_page: i64,
    pub total_items: i64,
    pub total_pages: i64,
}

/// One hourly bucket of `GET /api/logs/stats`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogStat {
    pub date: DateTime,
    pub total: i64,
}

/// Write `entries` in chunks of [`BATCH_MAX`] rows per statement.
pub async fn insert_batch(ex: &dyn Executor, entries: Vec<LogEntry>) -> DbResult<()> {
    for chunk in entries.chunks(BATCH_MAX) {
        let mut sql = String::from(
            r#"INSERT INTO "_logs" ("id", "level", "message", "data", "created", "updated") VALUES "#,
        );
        let mut params = Vec::with_capacity(chunk.len() * 6);
        for (i, e) in chunk.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            let b = i * 6;
            sql.push_str(&format!(
                "(${}, ${}, ${}, ${}, ${}, ${})",
                b + 1,
                b + 2,
                b + 3,
                b + 4,
                b + 5,
                b + 6
            ));
            params.push(Sql::Text(e.id.clone()));
            params.push(Sql::Int(e.level));
            params.push(Sql::Text(e.message.clone()));
            params.push(Sql::Text(e.data.to_string()));
            params.push(Sql::Text(e.created.to_pb_string()));
            params.push(Sql::Text(e.updated.to_pb_string()));
        }
        ex.execute(&sql, &params).await?;
    }
    Ok(())
}

/// Translate a PocketBase sort string into an `ORDER BY` clause over
/// the log columns.
pub fn order_by(sort: Option<&str>) -> DbResult<String> {
    let sort = sort
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_SORT);
    let mut parts = Vec::new();
    for raw in sort.split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let (col, dir) = match raw.strip_prefix('-') {
            Some(c) => (c, "DESC"),
            None => (raw.strip_prefix('+').unwrap_or(raw), "ASC"),
        };
        if !SORTABLE.contains(&col) {
            return Err(DbError::InvalidIdentifier(format!(
                "unsupported log sort field: {col}"
            )));
        }
        parts.push(format!("\"{col}\" {dir}"));
    }
    if parts.is_empty() {
        return order_by(None);
    }
    Ok(format!("ORDER BY {}", parts.join(", ")))
}

fn where_clause(filter: Option<&Filter>) -> (String, Vec<Sql>) {
    match filter {
        Some((sql, params)) if !sql.trim().is_empty() => (format!("WHERE {sql}"), params.clone()),
        _ => (String::new(), Vec::new()),
    }
}

/// One page of logs plus the total, PocketBase envelope shape.
pub async fn list(ex: &dyn Executor, params: ListParams) -> DbResult<LogPage> {
    let page = params.page.max(1);
    let per_page = params.per_page.clamp(1, 1000);
    let offset = (page - 1) * per_page;
    let (where_sql, bound) = where_clause(params.filter.as_ref());
    let order = order_by(params.sort.as_deref())?;

    let total_items = ex
        .query_scalar(
            &format!(r#"SELECT COUNT(*) FROM "_logs" {where_sql}"#),
            &bound,
        )
        .await?
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let rows = ex
        .query(
            &format!(
                r#"SELECT "id", "level", "message", "data", "created", "updated" FROM "_logs"
                   {where_sql} {order} LIMIT {per_page} OFFSET {offset}"#
            ),
            &bound,
        )
        .await?;
    let items = rows
        .iter()
        .map(row_to_entry)
        .collect::<DbResult<Vec<_>>>()?;
    let total_pages = if total_items == 0 {
        0
    } else {
        (total_items + per_page - 1) / per_page
    };
    Ok(LogPage {
        items,
        page,
        per_page,
        total_items,
        total_pages,
    })
}

pub async fn get(ex: &dyn Executor, id: &str) -> DbResult<LogEntry> {
    let row = ex
        .query_one(
            r#"SELECT "id", "level", "message", "data", "created", "updated" FROM "_logs" WHERE "id" = $1"#,
            &[Sql::from(id)],
        )
        .await?
        .ok_or(DbError::NotFound)?;
    row_to_entry(&row)
}

/// Hourly buckets, oldest first. SQLite only (the logs engine always
/// is): buckets are computed with `strftime`.
pub async fn stats(ex: &dyn Executor, filter: Option<Filter>) -> DbResult<Vec<LogStat>> {
    let (where_sql, bound) = where_clause(filter.as_ref());
    let rows = ex
        .query(
            &format!(
                r#"SELECT strftime('%Y-%m-%d %H:00:00.000Z', "created") AS "date", COUNT(*) AS "total"
                   FROM "_logs" {where_sql} GROUP BY 1 ORDER BY 1"#
            ),
            &bound,
        )
        .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let date = DateTime::parse(r.get_str("date")?)?;
            Some(LogStat {
                date,
                total: r.get_i64("total").unwrap_or(0),
            })
        })
        .collect())
}

/// Delete rows older than `days` days. Returns the number removed.
pub async fn delete_older_than(ex: &dyn Executor, days: i64) -> DbResult<u64> {
    let cutoff = DateTime::from_utc(chrono::Utc::now() - chrono::Duration::days(days.max(0)));
    ex.execute(
        r#"DELETE FROM "_logs" WHERE "created" < $1"#,
        &[Sql::Text(cutoff.to_pb_string())],
    )
    .await
}

fn row_to_entry(row: &Row) -> DbResult<LogEntry> {
    let data = match row.get_str("data") {
        Some(s) if !s.is_empty() => serde_json::from_str(s).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    Ok(LogEntry {
        id: row.get_str("id").unwrap_or("").to_string(),
        level: row.get_i64("level").unwrap_or(0),
        message: row.get_str("message").unwrap_or("").to_string(),
        data,
        created: DateTime::parse(row.get_str("created").unwrap_or("")).unwrap_or_default(),
        updated: DateTime::parse(row.get_str("updated").unwrap_or("")).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::SqliteEngine;
    use crate::system::ensure_logs_tables;
    use serde_json::json;

    async fn engine() -> SqliteEngine {
        let e = SqliteEngine::open_memory().unwrap();
        ensure_logs_tables(&e).await.unwrap();
        e
    }

    fn at(level: i64, msg: &str, ts: &str) -> LogEntry {
        let mut e = LogEntry::new(level, msg, json!({"type": "request", "status": 200}));
        e.created = DateTime::parse(ts).unwrap();
        e.updated = e.created;
        e
    }

    #[tokio::test]
    async fn insert_list_get_stats_and_cleanup() {
        let e = engine().await;
        let mut entries = vec![
            at(0, "GET /api/health", "2026-09-03 12:44:06.380Z"),
            at(
                0,
                "POST /api/collections/_superusers/auth-with-password",
                "2026-09-03 12:44:06.470Z",
            ),
            at(8, "boom", "2026-09-03 13:01:00.000Z"),
        ];
        // Past the batch boundary.
        for i in 0..BATCH_MAX {
            entries.push(at(4, &format!("warn {i}"), "2020-01-01 00:00:00.000Z"));
        }
        insert_batch(&e, entries.clone()).await.unwrap();
        assert_eq!(entries[0].id.len(), 15);

        let page = list(
            &e,
            ListParams {
                page: 1,
                per_page: 2,
                filter: None,
                sort: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(page.total_items, 3 + BATCH_MAX as i64);
        assert_eq!(page.total_pages, (3 + BATCH_MAX as i64 + 1) / 2);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].message, "boom");
        let v = serde_json::to_value(&page).unwrap();
        assert_eq!(v["perPage"], 2);
        assert_eq!(v["items"][0]["data"]["status"], 200);
        assert!(v["items"][0].get("updated").is_none());

        let filtered = list(
            &e,
            ListParams {
                page: 1,
                per_page: 10,
                filter: Some((
                    "\"level\" = $1 AND \"message\" LIKE $2".into(),
                    vec![Sql::Int(0), Sql::from("%api%")],
                )),
                sort: Some("created".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(filtered.total_items, 2);
        assert_eq!(filtered.items[0].message, "GET /api/health");
        assert!(list(
            &e,
            ListParams {
                page: 1,
                per_page: 1,
                filter: None,
                sort: Some("data.status".into())
            }
        )
        .await
        .is_err());

        let one = get(&e, &entries[2].id).await.unwrap();
        assert_eq!(one, entries[2]);
        assert!(matches!(get(&e, "missing").await, Err(DbError::NotFound)));

        let s = stats(
            &e,
            Some(("\"created\" > $1".into(), vec![Sql::from("2026-01-01")])),
        )
        .await
        .unwrap();
        assert_eq!(
            s,
            vec![
                LogStat {
                    date: DateTime::parse("2026-09-03 12:00:00.000Z").unwrap(),
                    total: 2
                },
                LogStat {
                    date: DateTime::parse("2026-09-03 13:00:00.000Z").unwrap(),
                    total: 1
                },
            ]
        );
        assert_eq!(
            serde_json::to_value(&s[0]).unwrap()["date"],
            "2026-09-03 12:00:00.000Z"
        );

        // The 2020 rows are older than 30 days; the 2026 rows are not.
        let removed = delete_older_than(&e, 30).await.unwrap();
        assert_eq!(removed, BATCH_MAX as u64);
        let removed = delete_older_than(&e, 0).await.unwrap();
        assert_eq!(removed, 3);
        assert_eq!(
            list(
                &e,
                ListParams {
                    page: 1,
                    per_page: 1,
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .total_items,
            0
        );
    }

    #[test]
    fn sort_parsing() {
        assert_eq!(order_by(None).unwrap(), "ORDER BY \"created\" DESC");
        assert_eq!(
            order_by(Some("-level,+created")).unwrap(),
            "ORDER BY \"level\" DESC, \"created\" ASC"
        );
        assert!(order_by(Some("nope")).is_err());
    }
}
