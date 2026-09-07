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
//! # Row cap enforced at the source for plain `SELECT`s
//!
//! A bare `SELECT ...` (no leading `WITH`, no trailing statements after
//! a `;`) is wrapped as `SELECT * FROM (<original>) AS
//! __cratebase_capped LIMIT <cap>+1` before it reaches the driver, so a
//! `SELECT * FROM huge_table` never materializes more than `ROW_CAP + 1`
//! rows in memory regardless of the table's real size. A `WITH` chain
//! (which might not end in a single `SELECT`'s row shape — see the
//! `is_read_statement` doc) or a statement containing more than one
//! trailing semicolon-separated piece is *not* wrapped, for exactly the
//! fragility reasons above: the full result set is fetched and the
//! *response* truncated to [`ROW_CAP`] instead, same as before this
//! change. This narrows, rather than closes, the unbounded-memory gap —
//! narrowing to "the common case is capped at the source" is worth
//! doing even though the general case still isn't.
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
pub(crate) fn is_read_statement(sql: &str) -> bool {
    let trimmed = sql.trim_start();
    let head: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    head == "SELECT" || head == "WITH"
}

/// Wrap a bare `SELECT` in an outer `LIMIT` so the driver itself never
/// materializes more than `cap + 1` rows, or `None` when the statement
/// isn't safely wrappable (see the module doc's "Row cap enforced at
/// the source" section): a `WITH` chain, or anything with more than one
/// semicolon-separated piece (a naive check — a semicolon inside a
/// string literal falls back to `None` too, which is a safe, merely
/// conservative failure mode, not a wrong-result one).
fn cappable_select(sql: &str, cap: usize) -> Option<String> {
    let trimmed = sql.trim();
    let head: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    if head != "SELECT" {
        return None;
    }
    let body = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    if body.contains(';') {
        return None;
    }
    Some(format!(
        "SELECT * FROM ({body}) AS __cratebase_capped LIMIT {}",
        cap + 1
    ))
}

/// Whether `sql` mentions `table` at all — a deliberately coarse,
/// no-parser check (matches `"_superusers"`, a bare `_superusers`, or
/// any other quoting a driver accepts) rather than trying to actually
/// parse the statement. False positives just make an unrelated write
/// ask again with `write: true` or get rejected once more than it has
/// to; a false negative is a way through the guard, so this stays
/// intentionally over-broad.
fn references_table(sql: &str, table: &str) -> bool {
    sql.to_ascii_lowercase()
        .contains(&table.to_ascii_lowercase())
}

async fn run_sql(
    axum::extract::State(app): axum::extract::State<App>,
    su: RequireSuperuser,
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

    // `_audit_log` has to stay append-only end to end (see
    // `crate::audit`'s module doc on why a rule string cannot express
    // that) — there is no legitimate reason for even a superuser to
    // bulk-edit or erase it, raw SQL included, so any non-read
    // statement mentioning it is refused outright, owner or not.
    // `_superusers` writes need the same owner-only gate raw SQL would
    // otherwise let a merely-`admin` superuser route around (see
    // `routes::records`'s `_superusers` guard doc) — `write: true`
    // running `UPDATE "_superusers" SET "role" = 'owner' ...` is exactly
    // the self-promotion path that guard exists to close.
    if !is_read {
        if references_table(&req.sql, crate::audit::COLLECTION) {
            return Err(ApiError::bad_request(
                "_audit_log cannot be modified through raw SQL.",
            ));
        }
        if references_table(&req.sql, cratebase_core::SUPERUSERS_COLLECTION)
            && !crate::extract::RequireOwner::holds(&su.0)
        {
            return Err(ApiError::forbidden(
                "Only an owner can run write SQL against _superusers.",
            ));
        }
        // `_cron_jobs` rows run their `sql` on a timer with no rule
        // enforcement — the same self-promotion path as a direct
        // `_superusers` write, just one tick later. Same owner-only gate.
        if references_table(&req.sql, cratebase_core::CRON_JOBS_COLLECTION)
            && !crate::extract::RequireOwner::holds(&su.0)
        {
            return Err(ApiError::forbidden(
                "Only an owner can run write SQL against _cron_jobs.",
            ));
        }
    }

    if is_read {
        let query_sql = cappable_select(&req.sql, ROW_CAP).unwrap_or_else(|| req.sql.clone());
        let rows = app
            .db()
            .query_interruptible(&query_sql, &[], QUERY_TIMEOUT)
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

/// `pub(crate)`: also used by `jsvm_host::raw_query` to shape
/// `$app.rawQuery`'s plain-object rows the same way this endpoint does.
pub(crate) fn row_to_map(row: Row) -> Map<String, Value> {
    row.into_pairs().map(|(k, v)| (k, sql_to_json(v))).collect()
}

/// [`Sql`] → JSON, matching the record layer's TEXT/INTEGER/REAL/NULL
/// mapping (see `crates/db/src/engine.rs`'s module doc) with one
/// addition: a raw `BLOB` column has no field-type context here to
/// decode it against, so it comes back base64-encoded rather than as
/// lossy UTF-8.
pub(crate) fn sql_to_json(value: Sql) -> Value {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cappable_select_wraps_a_bare_select() {
        let wrapped = cappable_select("select * from posts", 500).unwrap();
        assert_eq!(
            wrapped,
            "SELECT * FROM (select * from posts) AS __cratebase_capped LIMIT 501"
        );
    }

    #[test]
    fn cappable_select_tolerates_a_single_trailing_semicolon() {
        let wrapped = cappable_select("SELECT id FROM posts;  ", 10).unwrap();
        assert_eq!(
            wrapped,
            "SELECT * FROM (SELECT id FROM posts) AS __cratebase_capped LIMIT 11"
        );
    }

    #[test]
    fn cappable_select_declines_a_with_chain() {
        assert!(cappable_select("WITH x AS (SELECT 1) SELECT * FROM x", 500).is_none());
    }

    #[test]
    fn cappable_select_declines_multiple_statements() {
        assert!(cappable_select("SELECT 1; SELECT 2", 500).is_none());
    }

    #[test]
    fn cappable_select_declines_a_write() {
        assert!(cappable_select("DELETE FROM posts", 500).is_none());
    }

    #[test]
    fn is_read_statement_accepts_select_and_with_only() {
        assert!(is_read_statement("  select 1"));
        assert!(is_read_statement("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(!is_read_statement("DELETE FROM posts"));
        assert!(!is_read_statement("insert into posts values (1)"));
    }
}
