//! Cross-process proof that a cron tick runs on exactly one node when two
//! independent `App` instances share one Postgres database: each node's
//! own in-process `CronService` decides *locally* that a job is due (see
//! that module's doc), so both nodes fire their own copy of the same
//! `_cron_jobs`/`cronAdd` job at essentially the same instant —
//! `crate::cron_history::run_locked_with_history`'s Postgres advisory
//! lock is what has to actually resolve that race, not anything in
//! `CronService` itself (which has no idea another node exists).
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
    row.set("sql", serde_json::json!("SELECT 1"));
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

    // Fire both nodes' own schedulers for the exact same instant, as
    // concurrently as this process can manage — the scenario the
    // advisory lock exists for.
    let now = chrono::Utc::now();
    let (fired_a, fired_b) = tokio::join!(async { app_a.cron().tick(now) }, async {
        app_b.cron().tick(now)
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

    // `tick` only spawns the job body (`run_locked_with_history`, which
    // itself opens a transaction for the advisory-lock check before
    // running anything) — give both a moment to actually finish.
    let mut rows = Vec::new();
    for _ in 0..50 {
        rows = app_a
            .db()
            .query(
                r#"SELECT * FROM "_cronRuns" WHERE "jobId" = $1"#,
                &[cratebase_db::engine::Sql::from(job_id.clone())],
            )
            .await
            .expect("query _cronRuns");
        if !rows.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    assert_eq!(
        rows.len(),
        1,
        "exactly one node's advisory-lock-guarded run should have recorded a \
         _cronRuns row for this tick, not zero and not both"
    );
    assert_eq!(rows[0].get_str("source"), Some("sql"));
    assert_eq!(rows[0].get_str("status"), Some("success"));
}
