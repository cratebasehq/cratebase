//! Run history for cron jobs (`_cronRuns`) and the multi-node tick claim
//! that keeps a Postgres cluster from running the same scheduled tick
//! twice.
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
//! roughly the same instant, carrying the *same*
//! [`crate::cron::TickAt`]: `CronService::tick` floors it to the minute
//! identically on every node, so two nodes racing the same scheduled
//! minute always agree on the value.
//!
//! [`claim_tick`] resolves that race durably rather than with a timing
//! window: it inserts the `_cronRuns` row for this run itself —
//! `status = "running"` — guarded by a UNIQUE index on (`jobId`,
//! `tickAt`) with `ON CONFLICT ... DO NOTHING` (Postgres) / `INSERT OR
//! IGNORE` (SQLite). Only the node whose insert actually lands a row has
//! won the claim; every other node's insert affects zero rows and it
//! skips silently, before the job body ever runs — not a failure, just
//! "a sibling node already has this". The winner updates that same row
//! in place once its body finishes ([`finish_run`]), so `_cronRuns` ends
//! up with exactly one row per tick either way.
//!
//! This replaces an earlier `pg_try_advisory_xact_lock(hashtext(job_id))`
//! check: a transaction-scoped advisory lock, auto-released the instant
//! the *checking* transaction committed — well before the job body (and
//! its own row write) had necessarily finished. A slower sibling node
//! could, and in CI did, pass the same check after the first node had
//! already committed and run the same tick a second time (see
//! `crates/server/tests/cron_multi_node.rs`'s regression test). It was
//! also keyed on `job_id` alone, not the specific tick, so it could not
//! have told two different ticks of the same job apart even if it had
//! been durable. A row-claim guarded by a real UNIQUE constraint has no
//! such window: the constraint is checked and enforced by the database
//! itself, atomically, independent of when either node's transaction
//! commits.
//!
//! On SQLite (never actually multi-node) every call still claims its own
//! tick — `INSERT OR IGNORE` never rejects the sole node's own insert
//! since nothing else could have raced it — so single-node behavior is
//! unchanged: exactly one `_cronRuns` row per run.

use std::time::Instant;

use cratebase_core::{DateTime, Record};
use cratebase_db::engine::{Executor, Sql};
use cratebase_filter::Dialect;

use crate::app::App;

pub const COLLECTION: &str = "_cronRuns";

pub const SOURCE_SQL: &str = "sql";
pub const SOURCE_JS: &str = "js";

pub const STATUS_RUNNING: &str = "running";
pub const STATUS_SUCCESS: &str = "success";
pub const STATUS_ERROR: &str = "error";

/// The result of [`claim_tick`].
enum Claim {
    /// This call's insert landed the `_cronRuns` row — the id to update
    /// once the job body finishes.
    Won(String),
    /// Another row already claims this `(job_id, tick_at)` pair — a
    /// sibling node, or this node's own earlier attempt. Skip entirely.
    Lost,
    /// The claim mechanism itself failed (a transient DB error, or the
    /// `_cronRuns` collection is unexpectedly missing). Run the job body
    /// anyway — the same fail-open reasoning the old advisory-lock check
    /// used: a DB problem here is far more likely to also break the
    /// job's own work than to cause a real double-run, and never running
    /// a job on *any* node is the worse failure mode — but there is no
    /// claimed row to update afterward, so the outcome is recorded with
    /// a best-effort plain insert instead.
    Degraded,
}

/// Run `body` — but only after durably claiming `tick_at` for `job_id`
/// (see this module's doc), and only if that claim is won. Records one
/// `_cronRuns` row for the run (`source` is [`SOURCE_SQL`] or
/// [`SOURCE_JS`]; `body` returns `Ok(message)` for a [`STATUS_SUCCESS`]
/// row or `Err(message)` for a [`STATUS_ERROR`] one). A node that loses
/// the claim returns immediately, before `body` ever runs and before any
/// row is written — only the winner's run is recorded, so `_cronRuns`
/// reflects "what actually happened", once per tick, not "every node
/// that considered it".
pub async fn run_locked_with_history<F, Fut>(
    app: &App,
    job_id: &str,
    source: &str,
    tick_at: DateTime,
    body: F,
) where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    match claim_tick(app, job_id, source, tick_at).await {
        Claim::Lost => {
            tracing::debug!(job = %job_id, tick_at = %tick_at, "cron: another node already claimed this tick");
        }
        Claim::Won(run_id) => {
            let (status, message, duration_ms) = run_body(body).await;
            finish_run(app, &run_id, status, &message, duration_ms).await;
        }
        Claim::Degraded => {
            let (status, message, duration_ms) = run_body(body).await;
            record_run_unclaimed(app, job_id, source, status, &message, tick_at, duration_ms).await;
        }
    }
}

async fn run_body<F, Fut>(body: F) -> (&'static str, String, i64)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let start_instant = Instant::now();
    let (status, message) = match body().await {
        Ok(message) => (STATUS_SUCCESS, message),
        Err(message) => (STATUS_ERROR, message),
    };
    (status, message, start_instant.elapsed().as_millis() as i64)
}

/// Insert the `_cronRuns` row for this run, `status = "running"`, if no
/// row already claims `(job_id, tick_at)`. See [`Claim`]'s doc for what
/// each outcome means.
async fn claim_tick(app: &App, job_id: &str, source: &str, tick_at: DateTime) -> Claim {
    if app.db().collections.get_by_name(COLLECTION).is_none() {
        tracing::warn!("_cronRuns collection missing; cannot claim cron tick, running unclaimed");
        return Claim::Degraded;
    }
    let id = cratebase_core::record_id();
    let now = DateTime::now().to_pb_string();
    let insert_sql = match app.db().dialect() {
        Dialect::Postgres => format!(
            r#"INSERT INTO "{COLLECTION}"
                   ("id", "jobId", "source", "status", "startedAt", "tickAt", "durationMs", "message", "created", "updated")
               VALUES ($1, $2, $3, $4, $5, $6, 0, '', $7, $7)
               ON CONFLICT ("jobId", "tickAt") WHERE "tickAt" IS NOT NULL DO NOTHING"#
        ),
        Dialect::Sqlite => format!(
            r#"INSERT OR IGNORE INTO "{COLLECTION}"
                   ("id", "jobId", "source", "status", "startedAt", "tickAt", "durationMs", "message", "created", "updated")
               VALUES ($1, $2, $3, $4, $5, $6, 0, '', $7, $7)"#
        ),
    };
    let result = app
        .db()
        .execute(
            &insert_sql,
            &[
                Sql::from(id.clone()),
                Sql::from(job_id.to_string()),
                Sql::from(source.to_string()),
                Sql::from(STATUS_RUNNING),
                Sql::from(now.clone()),
                Sql::from(tick_at.to_pb_string()),
                Sql::from(now),
            ],
        )
        .await;
    match result {
        Ok(n) if n > 0 => Claim::Won(id),
        Ok(_) => Claim::Lost,
        Err(e) => {
            tracing::warn!(error = %e, job = %job_id, "cron: tick claim insert failed, running unclaimed");
            Claim::Degraded
        }
    }
}

/// Update a won claim's row with the finished run's outcome.
async fn finish_run(app: &App, run_id: &str, status: &str, message: &str, duration_ms: i64) {
    if let Err(e) = app
        .db()
        .execute(
            &format!(
                r#"UPDATE "{COLLECTION}" SET "status" = $1, "durationMs" = $2, "message" = $3 WHERE "id" = $4"#
            ),
            &[
                Sql::from(status),
                Sql::from(duration_ms),
                Sql::from(message.to_string()),
                Sql::from(run_id.to_string()),
            ],
        )
        .await
    {
        tracing::warn!(error = %e, run_id = %run_id, "failed to record a cron run's outcome");
    }
}

/// Best-effort fallback for [`Claim::Degraded`]: a plain insert with no
/// conflict handling, through the generic record API (unlike
/// [`claim_tick`]'s raw SQL, since there is no claim race left to win
/// here — the claim step itself already failed).
async fn record_run_unclaimed(
    app: &App,
    job_id: &str,
    source: &str,
    status: &str,
    message: &str,
    tick_at: DateTime,
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
    row.set(
        "startedAt",
        serde_json::json!(DateTime::now().to_pb_string()),
    );
    row.set("tickAt", serde_json::json!(tick_at.to_pb_string()));
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
    async fn sqlite_claims_every_tick_since_nothing_else_can_race_it() {
        let (app, _dir) = test_app().await;
        let tick_at = DateTime::now();
        assert!(matches!(
            claim_tick(&app, "some-job", SOURCE_JS, tick_at).await,
            Claim::Won(_)
        ));
        // A distinct tick for the same job claims too.
        let other_tick = DateTime::from_utc(tick_at.inner() + chrono::Duration::minutes(1));
        assert!(matches!(
            claim_tick(&app, "some-job", SOURCE_JS, other_tick).await,
            Claim::Won(_)
        ));
        // The *same* tick again (e.g. this node reconsidering it) loses.
        assert!(matches!(
            claim_tick(&app, "some-job", SOURCE_JS, tick_at).await,
            Claim::Lost
        ));
    }

    #[tokio::test]
    async fn run_locked_with_history_records_success_and_failure() {
        let (app, _dir) = test_app().await;

        run_locked_with_history(&app, "job-a", SOURCE_JS, DateTime::now(), || async {
            Ok("did the thing".to_string())
        })
        .await;
        run_locked_with_history(&app, "job-b", SOURCE_SQL, DateTime::now(), || async {
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
        assert!(rows[0].get_str("tickAt").is_some_and(|s| !s.is_empty()));

        assert_eq!(rows[1].get_str("jobId"), Some("job-b"));
        assert_eq!(rows[1].get_str("source"), Some(SOURCE_SQL));
        assert_eq!(rows[1].get_str("status"), Some(STATUS_ERROR));
        assert_eq!(rows[1].get_str("message"), Some("boom"));
    }

    #[tokio::test]
    async fn a_second_claim_of_the_same_tick_never_runs_the_body() {
        let (app, _dir) = test_app().await;
        let tick_at = DateTime::now();

        run_locked_with_history(&app, "job-a", SOURCE_SQL, tick_at, || async {
            Ok("first".to_string())
        })
        .await;
        run_locked_with_history(&app, "job-a", SOURCE_SQL, tick_at, || async {
            panic!("must never run: the tick is already claimed");
        })
        .await;

        let rows = app
            .db()
            .query(&format!(r#"SELECT * FROM "{COLLECTION}""#), &[])
            .await
            .expect("query _cronRuns");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get_str("message"), Some("first"));
    }
}
