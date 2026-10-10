//! `POST /api/llm/usage` and `GET /api/llm/usage/summary`: the `_llm_usage`
//! ledger for apps that call their LLM provider themselves (so the server
//! never sees the exact token counts) and want cost and totals tracked next
//! to their data. Unlike `crate::routes::llm` these routes are mounted
//! unconditionally: they do not need the chat gateway enabled.
//!
//! Superuser/API-key only — the ledger is an operator audit trail, the
//! same trust tier as the `_llm_usage` collection's own (null) rules.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use cratebase_core::AppError;
use cratebase_db::engine::{Executor, Sql};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiQuery, ApiResult};
use crate::llm::{self, UsageEntry};

pub fn router() -> Router<App> {
    Router::new()
        .route("/llm/usage", post(log))
        .route("/llm/usage/summary", get(summary))
}

fn field_error(name: &str, why: &str) -> ApiError {
    ApiError::bad_request(format!("{name}: {why}"))
}

fn opt_string(body: &Map<String, Value>, name: &str) -> Result<Option<String>, ApiError> {
    match body.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(field_error(name, "must be a string")),
    }
}

fn opt_count(body: &Map<String, Value>, name: &str) -> Result<Option<i64>, ApiError> {
    match body.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(i) if i >= 0 => Ok(Some(i)),
            _ => Err(field_error(name, "must be a non-negative integer")),
        },
        Some(_) => Err(field_error(name, "must be a non-negative integer")),
    }
}

fn parse_entry(body: &Value) -> Result<UsageEntry, ApiError> {
    let Some(body) = body.as_object() else {
        return Err(ApiError::bad_request("body must be a JSON object."));
    };
    let model = opt_string(body, "model")?
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| field_error("model", "is required"))?;
    let ok = match body.get("ok") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(field_error("ok", "must be a boolean")),
    };
    Ok(UsageEntry {
        caller_id: None,
        model,
        prompt_tokens: opt_count(body, "promptTokens")?
            .ok_or_else(|| field_error("promptTokens", "is required"))?,
        completion_tokens: opt_count(body, "completionTokens")?
            .ok_or_else(|| field_error("completionTokens", "is required"))?,
        tokens_cached: opt_count(body, "tokensCached")?,
        tokens_reasoning: opt_count(body, "tokensReasoning")?,
        latency_ms: opt_count(body, "latencyMs")?,
        ttft_ms: opt_count(body, "ttftMs")?,
        finish_reason: opt_string(body, "finishReason")?,
        ok,
        error: opt_string(body, "error")?,
        surface: opt_string(body, "surface")?,
        purpose: opt_string(body, "purpose")?,
        actor: opt_string(body, "actor")?,
        session_id: opt_string(body, "sessionId")?,
        run_id: opt_string(body, "runId")?,
        call_id: opt_string(body, "callId")?,
        source: opt_string(body, "source")?,
        note: opt_string(body, "note")?,
    })
}

async fn log(
    State(app): State<App>,
    su: RequireSuperuser,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let mut entry = parse_entry(&body)?;
    entry.caller_id = Some(su.0.id.clone());
    let cost = llm::cost_usd(
        &app.settings().llm,
        &entry.model,
        entry.prompt_tokens,
        entry.completion_tokens,
        entry.tokens_cached.unwrap_or(0),
    );
    let record = llm::record_usage(&app, entry).await.map_err(ApiError)?;
    Ok(Json(json!({ "id": record.id(), "costUsd": cost })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SummaryQuery {
    group_by: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

async fn summary(
    State(app): State<App>,
    _su: RequireSuperuser,
    ApiQuery(q): ApiQuery<SummaryQuery>,
) -> ApiResult<Json<Value>> {
    let key_expr = match q.group_by.as_deref().unwrap_or("model") {
        "model" => r#""model""#,
        "surface" => r#""surface""#,
        "purpose" => r#""purpose""#,
        "actor" => r#""actor""#,
        "day" => r#"substr("created", 1, 10)"#,
        _ => {
            return Err(field_error(
                "groupBy",
                "must be one of model, surface, purpose, actor, day",
            ))
        }
    };

    let mut conditions = Vec::new();
    let mut params = Vec::new();
    if let Some(from) = q.from.as_deref().filter(|s| !s.is_empty()) {
        params.push(Sql::from(from.to_string()));
        conditions.push(format!(r#""created" >= ${}"#, params.len()));
    }
    if let Some(to) = q.to.as_deref().filter(|s| !s.is_empty()) {
        params.push(Sql::from(to.to_string()));
        conditions.push(format!(r#""created" <= ${}"#, params.len()));
    }
    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        r#"SELECT {key_expr} AS "key",
                  COUNT(*) AS "calls",
                  CAST(COALESCE(SUM("promptTokens"), 0) AS BIGINT) AS "promptTokens",
                  CAST(COALESCE(SUM("completionTokens"), 0) AS BIGINT) AS "completionTokens",
                  CAST(COALESCE(SUM("costUsd"), 0) AS DOUBLE PRECISION) AS "costUsd"
           FROM "_llm_usage" {where_clause}
           GROUP BY 1
           ORDER BY "costUsd" DESC, "key" ASC"#
    );
    let rows = app
        .db()
        .query(&sql, &params)
        .await
        .map_err(|e| ApiError(AppError::from(e)))?;

    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "key": row.get_str("key").unwrap_or(""),
                "calls": row.get_i64("calls").unwrap_or(0),
                "promptTokens": row.get_i64("promptTokens").unwrap_or(0),
                "completionTokens": row.get_i64("completionTokens").unwrap_or(0),
                "costUsd": row.get_f64("costUsd").unwrap_or(0.0),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}
