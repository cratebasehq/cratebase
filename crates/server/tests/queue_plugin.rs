//! End-to-end coverage for the toggle-gated `queue` plugin
//! (`crates/server/src/queue.rs`): enabling `settings.queue.enabled`,
//! enqueuing a real job through `POST /api/plugins/queue/enqueue`, and
//! observing it run and its status update, all through the real HTTP
//! router — same harness shape as `tests/vector_fields.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use serde_json::{json, Value};
use tower::ServiceExt;

const SUPERUSER_EMAIL: &str = "admin@example.com";
const SUPERUSER_PASSWORD: &str = "hunter2hunter2";

struct Harness {
    app: App,
    token: String,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path().join("pb_data")));
        app.bootstrap().await.expect("bootstrap");
        let id = app
            .create_superuser(SUPERUSER_EMAIL, SUPERUSER_PASSWORD)
            .await
            .expect("superuser");
        let token = app
            .mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token");
        Harness {
            app,
            token,
            _dir: dir,
        }
    }

    /// Flip `settings.queue.enabled` on — the Queue plugin itself is now
    /// registered unconditionally by `App::bootstrap` (before the JS
    /// runtime even starts; see that call site's doc), so there is no
    /// second plugin to register here: the worker loop it already
    /// started is idle (reads the live setting every tick and does
    /// nothing while it's `false`) until this flips it. `calls` is bumped
    /// by a `"count"` handler registered on the app-wide `QueueHandle`.
    async fn enable_queue(&self, calls: Arc<AtomicUsize>) {
        self.app
            .queue_handle()
            .expect("the queue plugin is always registered")
            .register_handler("count", move |_payload| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            });

        let mut settings = (*self.app.settings()).clone();
        settings.queue.enabled = true;
        self.app.set_settings(settings).await.expect("enable queue");
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, Value) {
        let response = cratebase_server::router(self.app.clone())
            .oneshot(request)
            .await
            .expect("response");
        split(response).await
    }

    async fn admin(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", &self.token);
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        self.send(request).await
    }
}

async fn split(response: Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// This is the acceptance scenario spelled out for the batch: enable the
/// queue via settings, enqueue a real job via the API, confirm it runs
/// and its status updates — through the real HTTP layer, not a direct
/// module call.
#[tokio::test]
async fn enqueue_over_http_runs_and_updates_status() {
    let harness = Harness::new().await;
    let calls = Arc::new(AtomicUsize::new(0));
    harness.enable_queue(calls.clone()).await;
    assert!(harness.app.settings().queue.enabled);

    let (status, enqueued) = harness
        .admin(
            "POST",
            "/api/plugins/queue/enqueue",
            Some(json!({"queue": "count", "payload": {"hello": "world"}})),
        )
        .await;
    assert_eq!(status, 200, "{enqueued}");
    let id = enqueued["id"]
        .as_str()
        .expect("enqueue returns an id")
        .to_string();
    assert_eq!(enqueued["status"], "pending");

    // Force one worker pass right now rather than waiting out the live
    // plugin's production 1s tick interval — `QueueHandle::run_one_tick`
    // runs the exact same claim/run logic on demand.
    harness
        .app
        .queue_handle()
        .expect("queue plugin registered")
        .run_one_tick(
            &harness.app,
            Duration::from_millis(20),
            Duration::from_secs(1),
            Duration::from_secs(300),
        )
        .await;

    let (status, row) = harness
        .admin(
            "GET",
            &format!("/api/collections/_queue_jobs/records/{id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{row}");
    assert_eq!(row["status"], "completed");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "handler should run exactly once"
    );
}

/// Enqueueing (and the dead-letter API) works even while
/// `settings.queue.enabled` is off (the default) — a job just sits
/// `pending` until an operator turns processing on, live, with no
/// restart. This is deliberately different from the old
/// "route absent while disabled" behavior: the toggle now only ever
/// gates whether the worker loop *processes* `_queue_jobs`, never
/// whether the collection/API exists — see `crate::queue::QueuePlugin::setup`'s
/// doc.
#[tokio::test]
async fn enqueueing_works_while_disabled_but_the_job_is_never_processed() {
    let harness = Harness::new().await;
    assert!(
        !harness.app.settings().queue.enabled,
        "queue.enabled defaults to false"
    );

    let (status, enqueued) = harness
        .admin(
            "POST",
            "/api/plugins/queue/enqueue",
            Some(json!({"queue": "count", "payload": {}})),
        )
        .await;
    assert_eq!(status, 200, "{enqueued}");
    assert_eq!(enqueued["status"], "pending");
    let id = enqueued["id"].as_str().expect("id").to_string();

    // The live plugin's ticker checks `settings.queue.enabled` on every
    // pass and does nothing at all while it's off (see
    // `QueuePlugin::setup`'s doc) — well under its 1s interval is enough
    // margin to be confident no tick has run yet.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let (status, row) = harness
        .admin(
            "GET",
            &format!("/api/collections/_queue_jobs/records/{id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{row}");
    assert_eq!(
        row["status"], "pending",
        "a disabled queue must never process a job, even on a forced tick"
    );

    // Now the canonical route (superuser/API key, not just the plugin
    // alias) can retry/delete it regardless of the toggle too.
    let (status, _) = harness
        .admin("DELETE", &format!("/api/queue/jobs/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = harness
        .admin(
            "GET",
            &format!("/api/collections/_queue_jobs/records/{id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "deleted");
}
