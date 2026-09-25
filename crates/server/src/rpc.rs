//! Custom SQL RPC: the `_rpc` collection's own save-time validation, and
//! `POST /api/rpc/{name}`.
//!
//! # Named parameters, not positional
//!
//! `_rpc.sql` uses `:name` placeholders (a single colon; `::cast` is left
//! alone — see [`rewrite_named_placeholders`]), each declared in the
//! record's `params` array (`{name, type, required, default}`). A call's
//! JSON body supplies values by name; they are validated/coerced against
//! the declared `type` (`text|number|bool|json|date`) and then rewritten
//! to the engine's positional `$1..$n` and bound as real driver
//! parameters — never string-interpolated into `sql`, so an injection
//! attempt in a parameter value can only ever be the *value* of a bound
//! parameter, never additional SQL.
//!
//! # Reading `rule`/`sql`/... straight off the row, not through `Record`
//!
//! [`call_rpc`] reads the `_rpc` row it looks up with a plain `SELECT *`
//! and pulls fields off the raw [`cratebase_db::engine::Row`] rather than
//! decoding it into a [`cratebase_core::Record`] first. This matters for
//! exactly one field: `rule`. `column_value`/`decode_column`
//! (`crates/db/src/records.rs`) round-trip a `json`-typed field's stored
//! `NULL` faithfully, but collapse a stored *empty string* back to `NULL`
//! on the way out (the same normalization that keeps a plain `text`
//! field from ever mixing `NULL` and `''` for "empty") — which would
//! silently turn `rule = ""` ("anyone") back into "superuser only" the
//! moment it round-trips through `Record`. The raw `Row` has no such
//! collapse: `Sql::Null` and `Sql::Text("")` are exactly what was
//! written. This is a real, narrow limitation of reading `_rpc` back
//! through the generic Records API (a saved `rule = ""` will *display*
//! as `null` there) that this module's own execution path simply avoids
//! by never going through it.
//!
//! # Read-only enforcement
//!
//! `readOnly` (default `true`) reuses exactly the mechanism
//! `crates/server/src/routes/sql_console.rs` already proved closes the
//! "a write disguised as a read" gap: the statement runs through
//! [`cratebase_db::engine::Executor::query_interruptible`], which wraps
//! it in `BEGIN READ ONLY` + `SET LOCAL statement_timeout` on Postgres
//! (rejecting a data-modifying CTE, not just a bare `UPDATE`) and SQLite's
//! already-`query_only` reader pool. A `readOnly: false` RPC instead runs
//! through the writer path — real cancellation is not available there on
//! either backend for a statement that might return rows (see
//! `Executor::query_interruptible`'s own doc on the write-path gap), so
//! it only gets a bounded wait, and on SQLite specifically a write's
//! `RETURNING` rows are never available (the writer path drains and
//! discards them — see `crates/db/src/sqlite.rs`'s `run_execute`), so a
//! `readOnly: false` RPC's `items` come back empty there.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use cratebase_core::{is_valid_identifier, AppError, FieldError};
use cratebase_db::context::RequestContext;
use cratebase_db::engine::{quote_ident, Executor, Row, Sql};
use cratebase_db::{rules, CollectionResolver};
use cratebase_filter::Dialect;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::events::RecordEvent;
use crate::extract::RequestInfo;
use crate::hooks::{Event, Handler};
use crate::http_error::{ApiError, ApiResult};

/// `_rpc.timeoutMs`'s create-time default (see [`apply_create_defaults`]).
const DEFAULT_TIMEOUT_MS: i64 = 5000;
/// `_rpc.maxRows`'s create-time default.
const DEFAULT_MAX_ROWS: i64 = 1000;

pub fn router() -> Router<App> {
    Router::new().route("/rpc/{name}", post(call_rpc))
}

/// `_rpc.rule` is stored as `json`, not `text` (see the collection's own
/// doc for why), but the wire contract — and `rule-field.tsx`'s reuse on
/// the dashboard — is a plain string or `null`, exactly like a
/// collection's own `listRule`/`viewRule`. The generic `json` field
/// validator (`crates/db/src/validate.rs`'s `json`) requires a *string*
/// value to itself already be valid JSON text — PocketBase's
/// `json.Valid` check, meant for a pre-encoded multipart field — so a
/// bare filter expression like `@request.auth.id != ''` fails it
/// outright. Wrapping it in one more layer of JSON-string-encoding here,
/// before it ever reaches that validator, satisfies it (the encoded text
/// always parses: it's just a JSON string) and happens to fix the same
/// problem on the way *out* too: `column_value`'s "empty text collapses
/// to `NULL`" normalization (shared by every json-shaped field) never
/// sees an empty column, because the stored text for `rule: ""` is
/// `"\"\""` — two characters, never actually empty — so `rule: ""`
/// round-trips as `""` through the ordinary Records API, not `null`.
/// [`crate::rpc::call_rpc`] undoes the same encoding when it reads a row
/// back (see that module's own doc on reading straight off the row).
pub(crate) fn normalize_rule_field(input: &mut Map<String, Value>) {
    if let Some(Value::String(s)) = input.get("rule") {
        // Two layers: `crates/db/src/validate.rs`'s `coerce_record` (run
        // before validation, on every write) already unwraps a json
        // field's string value through `serde_json::from_str` once on
        // its own, so a single layer here would already be back to the
        // bare, non-JSON rule text by the time `validate::json` sees it.
        let once = Value::String(s.clone()).to_string();
        let twice = Value::String(once).to_string();
        input.insert("rule".to_string(), Value::String(twice));
    }
}

/// Fill in `_rpc`'s create-time defaults for the fields a generic
/// `Bool`/`Number` zero-value (`false`/`0`) would otherwise silently
/// stand in for. There is no schema-level "default value" mechanism in
/// this codebase (see `crates/core/src/collection.rs`'s `_rpc` doc), so
/// this runs on the raw JSON request body — before it becomes a
/// [`cratebase_core::Record`], which would already have every field at
/// its zero value and could no longer tell "the caller wrote `false`"
/// from "the caller wrote nothing" — exactly the way
/// `routes::records::create_record` calls this for `_rpc` specifically,
/// mirroring its own `is_cron_jobs`/`is_superusers` special cases.
pub(crate) fn apply_create_defaults(input: &mut Map<String, Value>) {
    input
        .entry("readOnly".to_string())
        .or_insert(Value::Bool(true));
    input
        .entry("timeoutMs".to_string())
        .or_insert(json!(DEFAULT_TIMEOUT_MS));
    input
        .entry("maxRows".to_string())
        .or_insert(json!(DEFAULT_MAX_ROWS));
}

/// Register `_rpc`'s save-time validation. Called once from
/// [`App::bootstrap`].
pub fn bind_hooks(app: &App) {
    app.hooks().on_record_create.bind(
        Handler::new(|e: &mut RecordEvent| {
            Box::pin(async move {
                validate(e).await?;
                e.next().await
            })
        })
        .with_tags([cratebase_core::RPC_COLLECTION]),
    );
    app.hooks().on_record_update.bind(
        Handler::new(|e: &mut RecordEvent| {
            Box::pin(async move {
                validate(e).await?;
                e.next().await
            })
        })
        .with_tags([cratebase_core::RPC_COLLECTION]),
    );
}

fn field_error(field: &str, message: impl Into<String>) -> AppError {
    AppError::validation(
        "An error occurred while validating the submitted data.",
        [(
            field.to_string(),
            FieldError::new("validation_invalid_format", message.into()),
        )],
    )
}

/// Save-time validation for an `_rpc` create/update: `name` is a valid
/// slug (it becomes a URL segment, `POST /api/rpc/{name}`), `sql` is a
/// single statement referencing only declared parameters, and `sql`
/// parses against the real driver (`Engine::prepare_check`) — a broken
/// or multi-statement definition is rejected here, not on its first real
/// call.
async fn validate(e: &mut RecordEvent) -> Result<(), AppError> {
    let name = e
        .record
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !is_valid_identifier(name) {
        return Err(field_error(
            "name",
            "Must be a valid identifier: letters, digits and underscores, not starting with a digit.",
        ));
    }

    let sql = e
        .record
        .get("sql")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if !is_single_statement(&sql) {
        return Err(field_error(
            "sql",
            "Only a single SQL statement is allowed.",
        ));
    }

    let specs =
        parse_param_specs(e.record.get("params")).map_err(|msg| field_error("params", msg))?;
    let (rewritten, referenced) = rewrite_named_placeholders(&sql);
    for param in &referenced {
        if !specs.iter().any(|s| &s.name == param) {
            return Err(field_error(
                "sql",
                format!("references undeclared parameter :{param} — add it to params."),
            ));
        }
    }

    e.app
        .prepare_check(&rewritten)
        .await
        .map_err(|err| field_error("sql", err.to_string()))?;

    Ok(())
}

/// Whether `sql` is exactly one statement: same conservative, no-parser
/// heuristic `crates/server/src/routes/sql_console.rs`'s `cappable_select`
/// already uses (tolerate one trailing `;`, reject any further `;`).
pub(crate) fn is_single_statement(sql: &str) -> bool {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return false;
    }
    let body = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    !body.contains(';')
}

/// Rewrite `:name` placeholders into positional `$1..$n`, in first-
/// occurrence order (a name used more than once reuses the same `$n`).
/// Returns the rewritten SQL and the declared-parameter names in that
/// same placeholder order.
///
/// A `::` cast (`col::text`) is left alone: a `:` immediately preceded or
/// followed by another `:` never starts a placeholder, and a bare `:`
/// not followed by an identifier character is just punctuation (`a ? :
/// b`-style ternaries never appear in SQL, but this keeps the scan
/// total rather than assuming otherwise).
pub(crate) fn rewrite_named_placeholders(sql: &str) -> (String, Vec<String>) {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut order: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ':' {
            let prev_colon = i > 0 && chars[i - 1] == ':';
            let next_colon = i + 1 < chars.len() && chars[i + 1] == ':';
            let starts_ident =
                i + 1 < chars.len() && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_');
            if !prev_colon && !next_colon && starts_ident {
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_')
                {
                    end += 1;
                }
                let name: String = chars[start..end].iter().collect();
                let idx = match order.iter().position(|n| n == &name) {
                    Some(pos) => pos,
                    None => {
                        order.push(name);
                        order.len() - 1
                    }
                };
                out.push('$');
                out.push_str(&(idx + 1).to_string());
                i = end;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    (out, order)
}

/// One entry of `_rpc.params`: `{name, type, required, default}`.
#[derive(Debug, Clone, Deserialize)]
struct RpcParamSpec {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    default: Option<Value>,
}

const PARAM_TYPES: &[&str] = &["text", "number", "bool", "json", "date"];

fn parse_param_specs(value: Option<&Value>) -> Result<Vec<RpcParamSpec>, String> {
    let items = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err("params must be a JSON array".to_string()),
    };
    let mut specs = Vec::with_capacity(items.len());
    for item in items {
        let spec: RpcParamSpec =
            serde_json::from_value(item.clone()).map_err(|e| format!("invalid param spec: {e}"))?;
        if !is_valid_identifier(&spec.name) {
            return Err(format!(
                "invalid param name '{}': expected letters, digits and underscores",
                spec.name
            ));
        }
        if !PARAM_TYPES.contains(&spec.ty.as_str()) {
            return Err(format!(
                "invalid param type '{}' for '{}': expected one of {}",
                spec.ty,
                spec.name,
                PARAM_TYPES.join("|")
            ));
        }
        specs.push(spec);
    }
    Ok(specs)
}

/// Resolve a call's declared parameters against the JSON body supplied:
/// missing-but-defaulted and missing-but-optional fall back accordingly,
/// missing-and-required is a 400, and every present value is coerced to
/// its declared `type` (also a 400 on mismatch — never silently
/// stringified into something that would bind as the wrong SQL type).
fn resolve_call_params(
    specs: &[RpcParamSpec],
    provided: &Map<String, Value>,
) -> Result<Map<String, Value>, ApiError> {
    let mut out = Map::new();
    for spec in specs {
        let value = match provided.get(&spec.name) {
            Some(v) => v.clone(),
            None => match &spec.default {
                Some(d) => d.clone(),
                None if spec.required => {
                    return Err(ApiError::bad_request(format!(
                        "missing required parameter '{}'",
                        spec.name
                    )))
                }
                None => Value::Null,
            },
        };
        out.insert(spec.name.clone(), coerce_param(spec, value)?);
    }
    Ok(out)
}

fn coerce_param(spec: &RpcParamSpec, value: Value) -> Result<Value, ApiError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let bad = || ApiError::bad_request(format!("parameter '{}' must be a {}", spec.name, spec.ty));
    match spec.ty.as_str() {
        "text" | "date" => match value {
            Value::String(_) => Ok(value),
            _ => Err(bad()),
        },
        "number" => match &value {
            Value::Number(_) => Ok(value),
            Value::String(s) => s
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
                .ok_or_else(bad),
            _ => Err(bad()),
        },
        "bool" => match value {
            Value::Bool(_) => Ok(value),
            _ => Err(bad()),
        },
        // Any JSON value is a valid "json" parameter.
        _ => Ok(value),
    }
}

/// Bind resolved parameters positionally, in the order
/// [`rewrite_named_placeholders`] first saw each name.
fn bind_positional(order: &[String], resolved: &Map<String, Value>) -> Vec<Sql> {
    order
        .iter()
        .map(|name| {
            resolved
                .get(name)
                .map(cratebase_db::query::to_param)
                .unwrap_or(Sql::Null)
        })
        .collect()
}

/// `rule` read straight off the row — see the module doc's "Reading
/// `rule`/`sql`/... straight off the row" section for why, and
/// [`normalize_rule_field`] for the JSON-string-encoding this undoes.
/// Anything that isn't exactly "absent" (`Sql::Null`) or "a JSON-encoded
/// string" is treated as `None` (superuser-only) — the safe default for
/// data written some other way (a direct SQL insert, say).
fn rule_from_row(row: &Row) -> Option<String> {
    match row.get("rule") {
        Some(Sql::Text(s)) if !s.is_empty() => match serde_json::from_str::<Value>(s) {
            Ok(Value::String(inner)) => Some(inner),
            _ => None,
        },
        _ => None,
    }
}

async fn call_rpc(
    State(app): State<App>,
    Path(name): Path<String>,
    info: RequestInfo,
    body: axum::body::Bytes,
) -> ApiResult<Json<Value>> {
    let params_body: Map<String, Value> = if body.is_empty() {
        Map::new()
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(Value::Object(m)) => m,
            Ok(Value::Null) => Map::new(),
            Ok(_) => return Err(ApiError::bad_request("request body must be a JSON object")),
            Err(e) => return Err(ApiError::bad_request(format!("invalid JSON body: {e}"))),
        }
    };

    let db = app.db();
    let rpc_collection = db
        .collections
        .get_by_name(cratebase_core::RPC_COLLECTION)
        .ok_or_else(|| ApiError::internal("_rpc collection missing"))?;

    let row = db
        .query_one(
            &format!(
                "SELECT * FROM {} WHERE \"name\" = $1",
                quote_ident(rpc_collection.table_name())
            ),
            &[Sql::from(name.clone())],
        )
        .await
        .map_err(|e| ApiError(AppError::from(e)))?;
    let Some(row) = row else {
        return Err(ApiError::not_found("Unknown RPC."));
    };

    let sql = row
        .get("sql")
        .and_then(Sql::as_str)
        .unwrap_or_default()
        .to_string();
    let specs = parse_param_specs(
        row.get("params")
            .and_then(Sql::as_str)
            .filter(|s| !s.trim().is_empty())
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .as_ref(),
    )
    .map_err(ApiError::internal)?;
    let read_only = row.get("readOnly").and_then(Sql::as_i64).unwrap_or(1) != 0;
    let timeout_ms = row
        .get("timeoutMs")
        .and_then(Sql::as_i64)
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_TIMEOUT_MS) as u64;
    let max_rows = row
        .get("maxRows")
        .and_then(Sql::as_i64)
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_ROWS) as usize;
    let rule = rule_from_row(&row);

    let resolved_params = resolve_call_params(&specs, &params_body)?;

    let ctx: RequestContext = info.with_body(resolved_params.clone()).to_context();
    let resolver =
        CollectionResolver::new(rpc_collection.clone(), &db.collections, &ctx, db.dialect());
    let allowed = rules::check_create_rule(db, &resolver, &rule)
        .await
        .map_err(|e| ApiError(AppError::from(e)))?;
    if !allowed {
        return Err(ApiError::forbidden(
            "Only superusers can perform this action.",
        ));
    }

    let (rewritten, order) = rewrite_named_placeholders(&sql);
    let bound_params = bind_positional(&order, &resolved_params);
    let timeout = Duration::from_millis(timeout_ms);

    let rows: Vec<Row> = if read_only {
        let sql_to_run = crate::routes::sql_console::cappable_select(&rewritten, max_rows)
            .unwrap_or_else(|| rewritten.clone());
        db.query_interruptible(&sql_to_run, &bound_params, timeout)
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?
    } else {
        match db.dialect() {
            // Postgres has no separate read-only connection tier, so a
            // write can go through `query` directly and still get any
            // `RETURNING` rows back.
            Dialect::Postgres => tokio::time::timeout(timeout, db.query(&rewritten, &bound_params))
                .await
                .map_err(|_| {
                    ApiError::bad_request(format!("rpc '{name}' timed out after {timeout_ms}ms"))
                })?
                .map_err(|e| ApiError::bad_request(e.to_string()))?,
            // SQLite's writer path drains and discards any rows a write
            // returns (see the module doc) — `execute` is the honest
            // choice here, and `items` comes back empty.
            Dialect::Sqlite => {
                tokio::time::timeout(timeout, db.execute(&rewritten, &bound_params))
                    .await
                    .map_err(|_| {
                        ApiError::bad_request(format!(
                            "rpc '{name}' timed out after {timeout_ms}ms"
                        ))
                    })?
                    .map_err(|e| ApiError::bad_request(e.to_string()))?;
                Vec::new()
            }
        }
    };

    let items: Vec<Map<String, Value>> = rows
        .into_iter()
        .take(max_rows)
        .map(crate::routes::sql_console::row_to_map)
        .collect();

    Ok(Json(json!({ "items": items })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_single_statement_tolerates_one_trailing_semicolon() {
        assert!(is_single_statement("SELECT 1"));
        assert!(is_single_statement("SELECT 1;"));
        assert!(is_single_statement("SELECT 1;  "));
        assert!(!is_single_statement("SELECT 1; SELECT 2"));
        assert!(!is_single_statement("SELECT 1; DROP TABLE t"));
        assert!(!is_single_statement(""));
        assert!(!is_single_statement("   "));
    }

    #[test]
    fn rewrite_named_placeholders_maps_names_positionally_and_reuses_repeats() {
        let (sql, order) = rewrite_named_placeholders("SELECT * FROM t WHERE a = :x AND b = :y");
        assert_eq!(sql, "SELECT * FROM t WHERE a = $1 AND b = $2");
        assert_eq!(order, vec!["x".to_string(), "y".to_string()]);

        // A name used twice reuses the same placeholder.
        let (sql, order) = rewrite_named_placeholders("SELECT * FROM t WHERE a = :x OR b = :x");
        assert_eq!(sql, "SELECT * FROM t WHERE a = $1 OR b = $1");
        assert_eq!(order, vec!["x".to_string()]);
    }

    #[test]
    fn rewrite_named_placeholders_leaves_casts_alone() {
        let (sql, order) = rewrite_named_placeholders("SELECT id::text FROM t WHERE a = :x");
        assert_eq!(sql, "SELECT id::text FROM t WHERE a = $1");
        assert_eq!(order, vec!["x".to_string()]);
    }

    #[test]
    fn rewrite_named_placeholders_no_params() {
        let (sql, order) = rewrite_named_placeholders("SELECT 1");
        assert_eq!(sql, "SELECT 1");
        assert!(order.is_empty());
    }

    #[test]
    fn parse_param_specs_accepts_a_well_formed_array() {
        let specs = parse_param_specs(Some(&json!([
            { "name": "q", "type": "text", "required": true },
            { "name": "limit", "type": "number", "default": 10 },
        ])))
        .unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "q");
        assert!(specs[0].required);
        assert_eq!(specs[1].default, Some(json!(10)));
    }

    #[test]
    fn parse_param_specs_rejects_bad_type_and_shape() {
        assert!(parse_param_specs(Some(&json!("nope"))).is_err());
        assert!(parse_param_specs(Some(&json!([{ "name": "x", "type": "bogus" }]))).is_err());
        assert!(parse_param_specs(Some(&json!([{ "name": "1x", "type": "text" }]))).is_err());
    }

    #[test]
    fn parse_param_specs_none_and_null_are_empty() {
        assert!(parse_param_specs(None).unwrap().is_empty());
        assert!(parse_param_specs(Some(&Value::Null)).unwrap().is_empty());
    }

    #[test]
    fn resolve_call_params_applies_defaults_and_rejects_missing_required() {
        let specs = vec![
            RpcParamSpec {
                name: "q".into(),
                ty: "text".into(),
                required: true,
                default: None,
            },
            RpcParamSpec {
                name: "limit".into(),
                ty: "number".into(),
                required: false,
                default: Some(json!(10)),
            },
        ];
        let provided = json!({ "q": "hi" }).as_object().unwrap().clone();
        let resolved = resolve_call_params(&specs, &provided).unwrap();
        assert_eq!(resolved["q"], json!("hi"));
        assert_eq!(resolved["limit"], json!(10));

        let empty = Map::new();
        assert!(resolve_call_params(&specs, &empty).is_err());
    }

    #[test]
    fn coerce_param_type_checks_and_rejects_mismatches() {
        let spec = RpcParamSpec {
            name: "n".into(),
            ty: "number".into(),
            required: false,
            default: None,
        };
        assert_eq!(coerce_param(&spec, json!(5)).unwrap(), json!(5));
        assert_eq!(coerce_param(&spec, json!("5.5")).unwrap(), json!(5.5));
        assert!(
            coerce_param(&spec, json!("not a number"))
                .unwrap_err()
                .error
                .status()
                == 400
        );
        assert!(coerce_param(&spec, json!(true)).is_err());

        let bool_spec = RpcParamSpec {
            name: "b".into(),
            ty: "bool".into(),
            required: false,
            default: None,
        };
        assert_eq!(coerce_param(&bool_spec, json!(true)).unwrap(), json!(true));
        assert!(coerce_param(&bool_spec, json!("true")).is_err());

        // null always passes through regardless of declared type.
        assert_eq!(coerce_param(&spec, Value::Null).unwrap(), Value::Null);
    }

    #[test]
    fn bind_positional_maps_resolved_values_in_placeholder_order() {
        let resolved = json!({ "x": 1, "y": "hi" }).as_object().unwrap().clone();
        let bound = bind_positional(&["y".to_string(), "x".to_string()], &resolved);
        assert_eq!(bound, vec![Sql::Text("hi".into()), Sql::Int(1)]);
        // A name with no resolved value (shouldn't happen given save-time
        // validation, but must not panic) binds NULL.
        let bound = bind_positional(&["missing".to_string()], &resolved);
        assert_eq!(bound, vec![Sql::Null]);
    }
}
