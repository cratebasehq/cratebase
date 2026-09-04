//! `POST /api/sql` — superuser only: run an ad-hoc SQL string straight
//! against the database and hand back whatever the driver returns.
//!
//! This is a deliberate trust boundary, not an oversight: a superuser can
//! already rewrite the schema, edit every record, and disable every API
//! rule from the dashboard, so handing them a raw SQL prompt adds no new
//! *capability* — it just skips the UI that would otherwise translate
//! their intent into schema/record operations. It sits at the same tier
//! as the collection-schema editor (`routes::collections`): both let a
//! superuser do something no field-level API rule can stop.
//!
//! # The read/write gate
//!
//! `write` defaults to `false`, and in that mode the statement is
//! rejected unless it trimmed-and-uppercased starts with `SELECT` or
//! `WITH` (a CTE feeding a `SELECT`). This is enforced here, server-side
//! — the dashboard's confirmation dialog is a courtesy, not the actual
//! guard, since a caller hitting the endpoint directly (curl, a script)
//! has no dialog to skip. The same classification also picks which
//! `Executor` method to call: a `SELECT`/`WITH` always goes through
//! [`cratebase_db::engine::Executor::query`] (even with `write: true` —
//! there is nothing to execute-and-discard about a read), everything
//! else goes through `execute`.
//!
//! # Why the real driver error, not a redacted one
//!
//! [`crate::http_error::ApiError`]'s usual `From<DbError>` impl
//! deliberately hides driver messages behind PocketBase's generic
//! wording (see `crates/db/src/error.rs`) — reasonable for a record
//! write where the message could leak schema/column details to a
//! non-superuser caller. Here the caller is already a superuser running
//! arbitrary SQL by choice, and the whole point of this endpoint is
//! debugging: swallowing "syntax error near..." in favour of "Something
//! went wrong" would make it useless. Errors are surfaced verbatim via
//! `to_string()` instead of the blanket conversion.
//!
//! # No row-cap query rewriting
//!
//! Wrapping an arbitrary caller-supplied statement in `SELECT * FROM (...)
//! LIMIT 500` is fragile — it breaks on statements that aren't a single
//! `SELECT` (a `WITH` chain ending in something else, trailing
//! semicolons, dialect quirks) and changes what the caller's own SQL
//! actually did. Instead the full result set is fetched and the
//! *response* is truncated to [`ROW_CAP`] rows, with `truncated: true`
//! when that happened — safe for the response payload, honest about
//! what ran.
//!
//! # Real cancellation on SQLite, bounded wait elsewhere
//!
//! [`QUERY_TIMEOUT`] used to only bound how long *this request* waits —
//! a caller that hit it got a clear error instead of a hung HTTP
//! connection, but the underlying statement kept running: both engines
//! run statements inside `tokio::task::spawn_blocking` (see
//! `crates/db/src/sqlite.rs`'s module doc), and racing a `timeout`
//! against a `spawn_blocking` future abandons the *future*, not the OS
//! thread. `Executor::query_interruptible`/`execute_interruptible` (see
//! `crates/db/src/engine.rs`) close that gap on SQLite: a companion
//! task calls `rusqlite::Connection::get_interrupt_handle().interrupt()`
//! on the exact connection running the statement once [`QUERY_TIMEOUT`]
//! elapses, so a pathological statement (an unindexed cross join, a
//! runaway recursive CTE) is actually stopped, not just abandoned by
//! this handler. Postgres has no equivalent wired up at this layer, so
//! there the same call falls back to the old bounded-wait-only
//! behavior — see that trait method's own doc for why.

use axum::routing::post;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use cratebase_db::engine::{Executor, Row, Sql};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiJson, ApiResult};

/// Response rows are truncated to this many entries; see the module doc.
const ROW_CAP: usize = 500;

/// How long an ad-hoc statement is allowed to run before the endpoint
/// gives up and returns an error; see the module doc.
const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub fn router() -> Router<App> {
    Router::new().route("/sql", post(run_sql))
}

#[derive(Debug, Deserialize)]
struct SqlRequest {
    sql: String,
    #[serde(default)]
    write: bool,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum SqlResponse {
    Read {
        columns: Vec<String>,
        rows: Vec<Map<String, Value>>,
        truncated: bool,
    },
    Write {
        #[serde(rename = "rowsAffected")]
        rows_affected: u64,
    },
}

/// Whether `sql` is a read: trimmed and case-insensitive, it starts with
/// `SELECT` or `WITH` (a CTE, which only ever feeds a `SELECT` in both
/// SQLite and Postgres). Anything else — `INSERT`, `UPDATE`, `DELETE`,
/// `CREATE`, `PRAGMA`, `VACUUM`, ... — is a write for the purposes of
/// both the read-only gate and the query/execute dispatch below.
fn is_read_statement(sql: &str) -> bool {
    let trimmed = sql.trim_start();
    let head: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    head == "SELECT" || head == "WITH"
}

async fn run_sql(
    axum::extract::State(app): axum::extract::State<App>,
    _su: RequireSuperuser,
    ApiJson(req): ApiJson<SqlRequest>,
) -> ApiResult<Json<SqlResponse>> {
    if req.sql.trim().is_empty() {
        return Err(ApiError::bad_request("sql must not be empty."));
    }

    let is_read = is_read_statement(&req.sql);

    if !req.write && !is_read {
        return Err(ApiError::bad_request(
            "Read-only mode: the statement must start with SELECT or WITH. \
             Pass \"write\": true to run other statements.",
        ));
    }

    if is_read {
        let rows = app
            .db()
            .query_interruptible(&req.sql, &[], QUERY_TIMEOUT)
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        let columns: Vec<String> = rows
            .first()
            .map(|r| r.columns.iter().cloned().collect())
            .unwrap_or_default();
        let truncated = rows.len() > ROW_CAP;
        let out_rows = rows.into_iter().take(ROW_CAP).map(row_to_map).collect();
        Ok(Json(SqlResponse::Read {
            columns,
            rows: out_rows,
            truncated,
        }))
    } else {
        let rows_affected = app
            .db()
            .execute_interruptible(&req.sql, &[], QUERY_TIMEOUT)
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        Ok(Json(SqlResponse::Write { rows_affected }))
    }
}

fn row_to_map(row: Row) -> Map<String, Value> {
    row.into_pairs().map(|(k, v)| (k, sql_to_json(v))).collect()
}

/// [`Sql`] → JSON, matching the record layer's TEXT/INTEGER/REAL/NULL
/// mapping (see `crates/db/src/engine.rs`'s module doc) with one
/// addition: a raw `BLOB` column has no field-type context here to
/// decode it against, so it comes back base64-encoded rather than as
/// lossy UTF-8.
fn sql_to_json(value: Sql) -> Value {
    match value {
        Sql::Null => Value::Null,
        Sql::Int(i) => Value::from(i),
        Sql::Real(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Sql::Text(s) => Value::String(s),
        Sql::Blob(b) => Value::String(BASE64.encode(b)),
    }
}
