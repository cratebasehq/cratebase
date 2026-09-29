//! Cross-process proof that a cron tick runs on exactly one node when two
//! independent `App` instances share one Postgres database: each node's
//! own in-process `CronService` decides *locally* that a job is due (see
//! that module's doc), so both nodes fire their own copy of the same
//! `_cron_jobs`/`cronAdd` job at essentially the same instant —
//! `crate::cron_history::run_locked_with_history`'s `(jobId, tickAt)`
//! claim is what has to actually resolve that race, not anything in
//! `CronService` itself (which has no idea another node exists).
//!
//! Two scenarios, both against the same real `sql` body (an `INSERT`
//! into a plain counter table, `cron_hits`) so "exactly one execution of
//! the body" is checked directly, not inferred from the `_cronRuns` row
//! count alone:
//!
//! - **Concurrent**: both nodes tick the same instant as concurrently as
//!   this process can manage — the scenario a transaction-scoped
//!   advisory lock could lose (see `crate::cron_history`'s doc for why).
//! - **Sequential**: node A's tick is given time to run to full
//!   completion (row written, `cron_hits` incremented) *before* node B
//!   ticks the very same instant — proof the guard is a durable claim on
//!   the tick itself, not a lock that only helps while both nodes happen
//!   to overlap in time.
//!
//! Skips when `TEST_POSTGRES_URL` is unset, matching
//! `tests/postgres_multi_node.rs`'s convention.

use cratebase_db::{Db, Executor};
use cratebase_server::app::App;
use cratebase_server::config::Config;

fn node_config(url: &str, dir: &std::path::Path) -> Config {
    Config {
        database_url: url.to_string(),
        log_requests: false,
        secret: "test-secret-0123456789".into(),
        ..Config::for_data_dir(dir.join("pb_data"))
    }
}

/// Polls `check` (up to 5s, 50ms apart) until it returns `true`, or gives
/// up and returns `false`.
async fn wait_until<F, Fut>(mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..100 {
        if check().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

async fn hit_count(db: &Db) -> i64 {
    db.query_scalar("SELECT COUNT(*) FROM cron_hits", &[])
        .await
        .expect("count cron_hits")
        .and_then(|v| v.as_i64())
        .unwrap_or(-1)
}

async fn run_count(db: &Db, job_id: &str) -> i64 {
    db.query_scalar(
        r#"SELECT COUNT(*) FROM "_cronRuns" WHERE "jobId" = $1"#,
        &[cratebase_db::engine::Sql::from(job_id.to_string())],
    )
    .await
    .expect("count _cronRuns")
    .and_then(|v| v.as_i64())
    .unwrap_or(-1)
}

#[tokio::test]
async fn a_sql_cron_tick_runs_on_exactly_one_node() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!("skipping a_sql_cron_tick_runs_on_exactly_one_node: TEST_POSTGRES_URL not set");
        return;
    };

    {
        let dir = tempfile::tempdir().unwrap();
        let wipe = Db::connect(&url, &dir.path().to_string_lossy())
            .await
            .expect("connect to wipe schema");
        wipe.execute("DROP SCHEMA public CASCADE", &[])
            .await
            .unwrap();
        wipe.execute("CREATE SCHEMA public", &[]).await.unwrap();
        // The job's own body: a plain counter table with no dedupe key of
        // its own, so "how many rows are in here" is a direct count of how
        // many times the body actually ran — independent of `_cronRuns`,
        // which is the very thing under test.
        wipe.execute("CREATE TABLE cron_hits (id SERIAL PRIMARY KEY)", &[])
            .await
            .unwrap();
        wipe.close().await.unwrap();
    }

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let app_a = App::new(node_config(&url, dir_a.path()));
    app_a.bootstrap().await.expect("bootstrap node A");

    // Node A creates the custom cron job; node B's own bootstrap (which
    // ends with `cron_jobs::sync_all`, loading every `_cron_jobs` row and
    // registering it with node B's *own* `CronService`) picks it up from
    // the shared table during its own startup.
    let collection = app_a
        .db()
        .collections
        .get_by_name("_cron_jobs")
        .expect("_cron_jobs exists");
    let mut row = cratebase_core::Record::new(collection);
    row.set("name", serde_json::json!("multi-node-test-job"));
    row.set("expression", serde_json::json!("* * * * *"));
    row.set(
        "sql",
        serde_json::json!("INSERT INTO cron_hits DEFAULT VALUES"),
    );
    row.set("enabled", serde_json::json!(true));
    cratebase_db::records::create(app_a.db(), &app_a.db().collections, &mut row)
        .await
        .expect("create custom cron job");
    let job_id = format!("custom:{}", row.id());
    // `cratebase_db::records::create` is the raw DB-layer write with no
    // concept of hooks (those are a server-crate concern layered around
    // `crate::routes::records::write_record`) — sync node A's own
    // scheduler explicitly rather than going through the real HTTP route
    // just for this setup step.
    cratebase_server::cron_jobs::sync_all(&app_a).await;

    let app_b = App::new(node_config(&url, dir_b.path()));
    app_b.bootstrap().await.expect("bootstrap node B");

    assert!(app_a.cron().has(&job_id), "node A registered the job");
    assert!(app_b.cron().has(&job_id), "node B registered the job too");

    // --- Scenario 1: concurrent -----------------------------------------
    //
    // Fire both nodes' own schedulers for the exact same instant, as
    // concurrently as this process can manage.
    let tick_1 = chrono::Utc::now();
    let (fired_a, fired_b) = tokio::join!(async { app_a.cron().tick(tick_1) }, async {
        app_b.cron().tick(tick_1)
    });
    assert_eq!(
        fired_a,
        vec![job_id.clone()],
        "node A considered the job due"
    );
    assert_eq!(
        fired_b,
        vec![job_id.clone()],
        "node B considered the job too"
    );

    assert!(
        wait_until(|| async { hit_count(app_a.db()).await == 1 }).await,
        "the job body should have run exactly once by now"
    );
    // A generous extra grace period: if the guard were still racy (the
    // bug this test exists to catch), a second, slower write would land
    // some time after the first — give it every chance to show up before
    // asserting it didn't.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    assert_eq!(
        hit_count(app_a.db()).await,
        1,
        "concurrent ticks: the job body must run exactly once, not zero and not twice"
    );
    assert_eq!(
        run_count(app_a.db(), &job_id).await,
        1,
        "concurrent ticks: exactly one node's claim should have recorded a _cronRuns row"
    );
    let rows = app_a
        .db()
        .query(
            r#"SELECT * FROM "_cronRuns" WHERE "jobId" = $1"#,
            &[cratebase_db::engine::Sql::from(job_id.clone())],
        )
        .await
        .expect("query _cronRuns");
    assert_eq!(rows[0].get_str("source"), Some("sql"));
    assert_eq!(rows[0].get_str("status"), Some("success"));
    assert!(
        rows[0].get_str("tickAt").is_some_and(|s| !s.is_empty()),
        "the claimed row records its canonical tick"
    );

    // --- Scenario 2: sequential ------------------------------------------
    //
    // A later tick, so both nodes' local per-(job, minute) guards treat it
    // as a fresh one. Node A's tick is given time to run to *full*
    // completion — row written, `cron_hits` incremented — before node B
    // ever ticks the same instant. A transaction-scoped advisory lock
    // (the old guard) would have already released by the time node A's
    // job body, let alone node B's tick, even starts, so this sequencing
    // specifically proves the claim is durable rather than merely a
    // "same instant" race fix.
    let tick_2 = tick_1 + chrono::Duration::minutes(1);
    let fired_a_2 = app_a.cron().tick(tick_2);
    assert_eq!(fired_a_2, vec![job_id.clone()], "node A considered it due");
    assert!(
        wait_until(|| async { hit_count(app_a.db()).await == 2 }).await,
        "node A's tick should have fully completed by now"
    );
    assert_eq!(
        run_count(app_a.db(), &job_id).await,
        2,
        "node A's own run should already be recorded before node B ticks at all"
    );

    let fired_b_2 = app_b.cron().tick(tick_2);
    assert_eq!(fired_b_2, vec![job_id.clone()], "node B considered it too");
    // Node B's tick has nothing to win: give it every chance to (wrongly)
    // run anyway before asserting it didn't.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    assert_eq!(
        hit_count(app_a.db()).await,
        2,
        "sequential ticks: node B must not re-run a tick node A already completed"
    );
    assert_eq!(
        run_count(app_a.db(), &job_id).await,
        2,
        "sequential ticks: still exactly one _cronRuns row per tick"
    );
}
