//! `/api/logs` — superuser only.
//!
//! The rows come from `_logs` in the auxiliary database. Filters use the
//! ordinary PocketBase filter grammar, compiled against a **synthetic
//! collection** describing the log table (`level` number, `message` text,
//! `data` json, `created` autodate). That is what makes
//! `data.method = "GET"` and `level >= 8` work with no special-casing:
//! `data` is a json field, so the compiler emits the same JSON extraction
//! it would for a user collection.
//!
//! Every read flushes the background log writer first, so a request made
//! a millisecond ago is already visible — without it the dashboard shows
//! a stale page for up to one flush interval.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use cratebase_core::{Collection, CollectionType, Field, FieldKind};
use cratebase_db::logs::{self, LogEntry, LogPage, LogStat};
use cratebase_db::Sql;
use cratebase_filter::{Dialect, RequestPath, Resolver};
use serde::Deserialize;
use serde_json::Value;

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiQuery, ApiResult};

pub fn router() -> Router<App> {
    Router::new()
        .route("/logs", get(list))
        .route("/logs/stats", get(stats))
        .route("/logs/{id}", get(view))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    #[serde(default)]
    page: Option<i64>,
    #[serde(default)]
    per_page: Option<i64>,
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    sort: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct StatsQuery {
    #[serde(default)]
    filter: Option<String>,
}

async fn list(
    State(app): State<App>,
    _su: RequireSuperuser,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> ApiResult<Json<LogPage>> {
    app.logger().flush().await;
    let page = logs::list(
        &*app.db().logs,
        logs::ListParams {
            page: query.page.unwrap_or(1).max(1),
            // PocketBase caps `perPage` at 1000 (see KNOWN_DIVERGENCES §18).
            per_page: query.per_page.unwrap_or(30).clamp(1, 1000),
            filter: compile_filter(query.filter.as_deref())?,
            sort: query.sort,
        },
    )
    .await?;
    Ok(Json(page))
}

async fn view(
    State(app): State<App>,
    _su: RequireSuperuser,
    Path(id): Path<String>,
) -> ApiResult<Json<LogEntry>> {
    app.logger().flush().await;
    Ok(Json(logs::get(&*app.db().logs, &id).await?))
}

async fn stats(
    State(app): State<App>,
    _su: RequireSuperuser,
    ApiQuery(query): ApiQuery<StatsQuery>,
) -> ApiResult<Json<Vec<LogStat>>> {
    app.logger().flush().await;
    Ok(Json(
        logs::stats(&*app.db().logs, compile_filter(query.filter.as_deref())?).await?,
    ))
}

/// Compile a PocketBase filter expression into the `(WHERE, params)` pair
/// `cratebase_db::logs` expects.
fn compile_filter(filter: Option<&str>) -> Result<Option<logs::Filter>, ApiError> {
    let Some(src) = filter.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let resolver = LogResolver::new();
    let compiled = cratebase_filter::parse_and_compile(src, &resolver, 0)?;
    if !compiled.joins.is_empty() {
        // The log schema has no relations, so this can only mean the
        // filter reached for one.
        return Err(ApiError::bad_request(
            cratebase_core::AppError::DEFAULT_BAD_REQUEST,
        ));
    }
    let params = compiled.params.into_iter().map(json_to_sql).collect();
    Ok(Some((compiled.sql, params)))
}

fn json_to_sql(value: Value) -> Sql {
    match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Int(b as i64),
        Value::Number(n) => n
            .as_i64()
            .map(Sql::Int)
            .or_else(|| n.as_f64().map(Sql::Real))
            .unwrap_or(Sql::Null),
        Value::String(s) => Sql::Text(s),
        other => Sql::Text(other.to_string()),
    }
}

/// The virtual `_logs` "collection" filters compile against.
struct LogResolver {
    collection: Collection,
}

impl LogResolver {
    fn new() -> Self {
        // Named `_logs` so the compiler qualifies columns as
        // `"_logs"."level"`, which is exactly the table the query selects
        // from.
        let mut collection = Collection::new("_logs", CollectionType::Base);
        collection.system = true;
        // `Collection::new` seeds `id`, `created` and `updated`; add the
        // three columns that carry the payload.
        let extra = vec![
            Field::new(
                "level",
                FieldKind::Number {
                    min: None,
                    max: None,
                    only_int: true,
                },
            ),
            Field::new(
                "message",
                FieldKind::Text {
                    min: 0,
                    max: 0,
                    pattern: String::new(),
                    autogenerate_pattern: String::new(),
                    primary_key: false,
                },
            ),
            Field::new("data", FieldKind::Json { max_size: 0 }),
        ];
        let position = collection.fields.len().saturating_sub(2);
        collection.fields.splice(position..position, extra);
        collection.assign_field_ids();
        LogResolver { collection }
    }
}

impl Resolver for LogResolver {
    fn root(&self) -> &Collection {
        &self.collection
    }

    fn collection(&self, _name_or_id: &str) -> Option<Arc<Collection>> {
        // No relations and no `@collection.X` against the log table.
        None
    }

    fn request_value(&self, _path: &RequestPath) -> Value {
        // `@request.*` is meaningless here; PocketBase treats an unknown
        // macro as an empty value rather than an error.
        Value::Null
    }

    fn body_has(&self, _key: &str) -> bool {
        false
    }

    fn dialect(&self) -> Dialect {
        // The logs database is always SQLite, on every backend (spec §5.1).
        Dialect::Sqlite
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_filter_compiles_to_none() {
        assert!(compile_filter(None).unwrap().is_none());
        assert!(compile_filter(Some("   ")).unwrap().is_none());
    }

    #[test]
    fn level_and_json_paths_compile() {
        let (sql, params) = compile_filter(Some("level >= 8")).unwrap().unwrap();
        assert!(sql.contains("\"_logs\".\"level\""), "{sql}");
        assert_eq!(params.len(), 1);

        let (sql, params) = compile_filter(Some(r#"data.method = "GET""#))
            .unwrap()
            .unwrap();
        assert!(sql.contains("\"_logs\".\"data\""), "{sql}");
        assert_eq!(params, vec![Sql::Text("GET".into())]);

        let (sql, _) = compile_filter(Some(r#"message ~ "health""#))
            .unwrap()
            .unwrap();
        assert!(sql.to_uppercase().contains("LIKE"), "{sql}");
    }

    #[test]
    fn a_malformed_filter_is_a_400() {
        let err = compile_filter(Some("nonsense field")).unwrap_err();
        assert_eq!(err.error.status(), 400);
    }
}
