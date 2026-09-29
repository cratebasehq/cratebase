//! Cross-process proof that `crate::queue`'s claim is really race-free on
//! Postgres, not just inside one process's own transaction pool: two
//! independent `App` instances, each with its own connection pool, share
//! one `_queue_jobs` table and both run their worker loop against it at
//! once. Every enqueued job must be claimed and run by exactly one of
//! them — never zero (stuck forever), never two (a `FOR UPDATE SKIP
//! LOCKED` bug would show up as a job's handler firing twice).
//!
//! Skips (rather than fails) when `TEST_POSTGRES_URL` is unset, matching
//! `crates/server/tests/postgres_multi_node.rs`'s convention.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cratebase_db::{Db, Executor};
use cratebase_server::app::App;
use cratebase_server::config::Config;
use cratebase_server::queue::{self, EnqueueOptions};

// The live worker loop ticks once a second per node (see
// `QueuePlugin::new`'s default `tick_interval`), claiming (at most) one
// job per tick, so `JOB_COUNT` jobs across two nodes need at least
// `JOB_COUNT / 2` seconds even under perfect conditions — kept modest,
// with a generous deadline below, so this stays reliable on a loaded or
// slow CI runner rather than racing its own timeout.
const JOB_COUNT: usize = 24;

fn node_config(url: &str, dir: &std::path::Path) -> Config {
    Config {
        database_url: url.to_string(),
        log_requests: false,
        secret: "test-secret-0123456789".into(),
        ..Config::for_data_dir(dir.join("pb_data"))
    }
}

async fn enable_queue_with_handler(
    app: &App,
    seen: Arc<Mutex<HashSet<String>>>,
    calls: Arc<AtomicUsize>,
) {
    app.queue_handle()
        .expect("queue plugin is always registered")
        .register_handler("multi-node-job", move |payload| {
            let seen = seen.clone();
            let calls = calls.clone();
            async move {
                let id = payload["id"].as_str().unwrap_or_default().to_string();
                let first_time = seen.lock().unwrap().insert(id.clone());
                calls.fetch_add(1, Ordering::SeqCst);
                if !first_time {
                    return Err(format!("job {id} was claimed and run more than once"));
                }
                Ok(())
            }
        });
    let mut settings = (*app.settings()).clone();
    settings.queue.enabled = true;
    app.set_settings(settings)
        .await
        .expect("enable queue live, no restart");
}

#[tokio::test]
async fn every_job_is_claimed_and_run_exactly_once_across_two_nodes() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping every_job_is_claimed_and_run_exactly_once_across_two_nodes: \
             TEST_POSTGRES_URL not set"
        );
        return;
    };

    // Fresh schema before either node opens its own pool — same
    // convention as `postgres_multi_node.rs`.
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
    let app_b = App::new(node_config(&url, dir_b.path()));
    app_a.bootstrap().await.expect("bootstrap node A");
    app_b.bootstrap().await.expect("bootstrap node B");

    let seen = Arc::new(Mutex::new(HashSet::new()));
    let calls_a = Arc::new(AtomicUsize::new(0));
    let calls_b = Arc::new(AtomicUsize::new(0));
    enable_queue_with_handler(&app_a, seen.clone(), calls_a.clone()).await;
    enable_queue_with_handler(&app_b, seen.clone(), calls_b.clone()).await;

    // Enqueue every job from node A before either node's worker loop has
    // had a real chance to run (both loops tick on a live 1s interval
    // started at `bootstrap`, so this easily wins that race in practice;
    // it would not matter for correctness even if it didn't).
    let mut ids = Vec::with_capacity(JOB_COUNT);
    for i in 0..JOB_COUNT {
        let outcome = queue::enqueue_job(
            &app_a,
            "multi-node-job",
            serde_json::json!({ "id": format!("job-{i}") }),
            EnqueueOptions::default(),
        )
        .await
        .expect("enqueue");
        ids.push(outcome.id);
    }

    // Poll until every row is `completed`, or fail after a generous
    // timeout — each node's own ticker is a real 1s-interval background
    // loop here (not `run_one_tick`), since the point of this test is
    // exactly that two independent, live worker loops don't race.
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    loop {
        let rows = app_a
            .db()
            .query(
                r#"SELECT "status" FROM "_queue_jobs" WHERE "queue" = 'multi-node-job'"#,
                &[],
            )
            .await
            .expect("query _queue_jobs");
        let completed = rows
            .iter()
            .filter(|r| r.get_str("status") == Some(queue::STATUS_COMPLETED))
            .count();
        if completed == JOB_COUNT {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "only {completed}/{JOB_COUNT} jobs completed within the timeout \
                 (seen so far: {})",
                seen.lock().unwrap().len()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert_eq!(
        seen.lock().unwrap().len(),
        JOB_COUNT,
        "every job id must have been seen exactly once"
    );
    assert_eq!(
        calls_a.load(Ordering::SeqCst) + calls_b.load(Ordering::SeqCst),
        JOB_COUNT,
        "the handler must fire exactly {JOB_COUNT} times total across both nodes, \
         not once per node per job"
    );
    // Each node did *some* of the work — proof this actually exercised
    // cross-node contention rather than one node claiming everything
    // before the other's pool even finished connecting. Not a hard
    // guarantee (a slow CI runner could still see one node win every
    // race), so this is a soft assertion: log, don't fail, if it's ever
    // lopsided.
    eprintln!(
        "node A ran {} job(s), node B ran {} job(s)",
        calls_a.load(Ordering::SeqCst),
        calls_b.load(Ordering::SeqCst)
    );
}
