//! Run history for cron jobs (`_cronRuns`) and the multi-node advisory
//! lock that keeps a Postgres cluster from running the same tick twice.
//!
//! # Why this exists as its own module, not inside `crate::cron`
//!
//! `crate::cron::CronService` is a deliberately DB-agnostic in-process
//! scheduler (see its own module doc): it has no `App`, no database, no
//! idea any of this exists. Both callers that *do* have an `App` —
//! `crate::cron_jobs::run_custom_job` (SQL `_cron_jobs` rows) and
//! `crate::jsvm_host::HostApi::register_cron`'s closure (JS `cronAdd`
//! jobs, which have no row of their own anywhere else) — wrap their job
//! body in [`run_locked_with_history`] at the call site instead, so
//! `CronService` itself stays exactly as generic as it was.
//!
//! # Multi-node safety
//!
//! Each node's own `CronService` decides *locally and independently*
//! that a job is due this minute (see that module's per-`(job, minute)`
//! guard) — on Postgres, with more than one node sharing a database,
//! every node reaches [`run_locked_with_history`] for the same job at
//! roughly the same instant. [`try_claim_tick`] resolves that race with
//! `pg_try_advisory_xact_lock(hashtext(job_id))`: a *transaction-scoped*
//! advisory lock, auto-released at that transaction's commit rather than
//! needing an explicit unlock — the right choice specifically because the
//! lock is only ever taken inside a short-lived transaction opened just
//! for this check (`App::run_in_transaction`), so there's no session-scoped
//! lock that could ever survive back into a *pooled* connection and leak
//! across completely unrelated later work (the well-known footgun with
//! `pg_advisory_lock`'s plain, session-scoped form under a connection
//! pool). Whichever node's `pg_try_advisory_xact_lock` call returns `true`
//! runs the job and writes the one `_cronRuns` row for this tick; every
//! other node sees `false` and skips silently — not a failure, just "a
//! sibling node already has this". On SQLite (never actually multi-node)
//! this check is skipped entirely and every call proceeds.

use std::time::Instant;

use cratebase_core::{AppError, DateTime, Record};
use cratebase_db::engine::{Executor, Sql};
use cratebase_filter::Dialect;

use crate::app::App;

pub const COLLECTION: &str = "_cronRuns";

pub const SOURCE_SQL: &str = "sql";
pub const SOURCE_JS: &str = "js";

pub const STATUS_SUCCESS: &str = "success";
pub const STATUS_ERROR: &str = "error";

/// Run `body` — but only after winning [`try_claim_tick`]'s race, and
/// only if it's won. Records one `_cronRuns` row for the run (`source` is
/// [`SOURCE_SQL`] or [`SOURCE_JS`]; `body` returns `Ok(message)` for a
/// [`STATUS_SUCCESS`] row or `Err(message)` for a [`STATUS_ERROR`] one).
/// A node that loses the race returns immediately, before `body` ever
/// runs and before any row is written — only the winner's run is
/// recorded, so `_cronRuns` reflects "what actually happened", once per
/// tick, not "every node that considered it".
pub async fn run_locked_with_history<F, Fut>(app: &App, job_id: &str, source: &str, body: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    if !try_claim_tick(app, job_id).await {
        tracing::debug!(job = %job_id, "cron: another node already claimed this tick");
        return;
    }
    let started = DateTime::now();
    let start_instant = Instant::now();
    let (status, message) = match body().await {
        Ok(message) => (STATUS_SUCCESS, message),
        Err(message) => (STATUS_ERROR, message),
    };
    let duration_ms = start_instant.elapsed().as_millis() as i64;
    record_run(app, job_id, source, status, &message, started, duration_ms).await;
}

/// `true` when this node should run the job this tick. Always `true` on
/// SQLite (never multi-node) or if the lock query itself errors (a
/// transient connectivity problem is far more likely to also break the
/// job's own DB work than to cause a real double-run, so this fails open
/// rather than risk the job silently never running on *any* node).
async fn try_claim_tick(app: &App, job_id: &str) -> bool {
    if !matches!(app.db().dialect(), Dialect::Postgres) {
        return true;
    }
    let job_id = job_id.to_string();
    let result = app
        .run_in_transaction(|tx| {
            let job_id = job_id.clone();
            async move {
                let row = tx
                    .query_one(
                        "SELECT pg_try_advisory_xact_lock(hashtext($1)) AS locked",
                        &[Sql::from(job_id)],
                    )
                    .await
                    .map_err(AppError::from)?;
                Ok(row
                    .and_then(|r| r.get("locked").and_then(Sql::as_i64))
                    .map(|v| v != 0)
                    .unwrap_or(false))
            }
        })
        .await;
    match result {
        Ok(locked) => locked,
        Err(e) => {
            tracing::warn!(error = %e, job = %job_id, "cron: advisory lock check failed, running anyway");
            true
        }
    }
}

async fn record_run(
    app: &App,
    job_id: &str,
    source: &str,
    status: &str,
    message: &str,
    started_at: DateTime,
    duration_ms: i64,
) {
    let Some(collection) = app.db().collections.get_by_name(COLLECTION) else {
        tracing::warn!("_cronRuns collection missing; dropping cron run record");
        return;
    };
    let mut row = Record::new(collection);
    row.set("jobId", serde_json::json!(job_id));
    row.set("source", serde_json::json!(source));
    row.set("status", serde_json::json!(status));
    row.set("startedAt", serde_json::json!(started_at.to_pb_string()));
    row.set("durationMs", serde_json::json!(duration_ms));
    row.set("message", serde_json::json!(message));
    if let Err(e) = cratebase_db::records::create(app.db(), &app.db().collections, &mut row).await {
        tracing::warn!(error = %e, job = %job_id, "failed to record a cron run");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path().join("pb_data")));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    #[tokio::test]
    async fn sqlite_always_claims_the_tick() {
        let (app, _dir) = test_app().await;
        assert!(try_claim_tick(&app, "some-job").await);
        // A second, "concurrent" check also succeeds — SQLite is never
        // multi-node, so there is nothing to arbitrate.
        assert!(try_claim_tick(&app, "some-job").await);
    }

    #[tokio::test]
    async fn run_locked_with_history_records_success_and_failure() {
        let (app, _dir) = test_app().await;

        run_locked_with_history(&app, "job-a", SOURCE_JS, || async {
            Ok("did the thing".to_string())
        })
        .await;
        run_locked_with_history(&app, "job-b", SOURCE_SQL, || async {
            Err("boom".to_string())
        })
        .await;

        let rows = app
            .db()
            .query(
                &format!(r#"SELECT * FROM "{COLLECTION}" ORDER BY "jobId""#),
                &[],
            )
            .await
            .expect("query _cronRuns");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get_str("jobId"), Some("job-a"));
        assert_eq!(rows[0].get_str("source"), Some(SOURCE_JS));
        assert_eq!(rows[0].get_str("status"), Some(STATUS_SUCCESS));
        assert_eq!(rows[0].get_str("message"), Some("did the thing"));
        assert!(rows[0].get_i64("durationMs").unwrap_or(-1) >= 0);

        assert_eq!(rows[1].get_str("jobId"), Some("job-b"));
        assert_eq!(rows[1].get_str("source"), Some(SOURCE_SQL));
        assert_eq!(rows[1].get_str("status"), Some(STATUS_ERROR));
        assert_eq!(rows[1].get_str("message"), Some("boom"));
    }
}
