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
use cratebase_server::plugin::Plugin;
use cratebase_server::queue::{self, QueuePlugin};
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
        let app = App::new(Config::memory(dir.path()));
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

    /// Flip `settings.queue.enabled` on and do exactly what
    /// `App::bootstrap_inner` does for an *already*-enabled boot
    /// (`queue::ensure_collection` then `register_plugin`) — a real
    /// operator flips the flag and restarts, which re-runs bootstrap from
    /// scratch; this reproduces the same two steps against a live `App`
    /// instead of requiring a second process. `calls` is bumped by a
    /// `"count"` handler registered on the plugin before it goes live.
    async fn enable_queue(&self, calls: Arc<AtomicUsize>) {
        let mut settings = (*self.app.settings()).clone();
        settings.queue.enabled = true;
        self.app.set_settings(settings).await.expect("enable queue");

        queue::ensure_collection(&self.app)
            .await
            .expect("ensure _queue_jobs");

        let plugin = QueuePlugin::new().with_tick_interval(Duration::from_millis(20));
        let handle = plugin.handle();
        handle.register_handler("count", move |_payload| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
        plugin.setup(&self.app).expect("start queue worker");
        self.app
            .register_plugin(plugin)
            .expect("register queue plugin");
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

    // Poll `_queue_jobs` through the ordinary Records API (superuser-only
    // rules, same as `_cron_jobs`/`_webhooks`) until the worker tick picks
    // the job up and marks it completed.
    let mut final_status = None;
    for _ in 0..50 {
        let (status, row) = harness
            .admin(
                "GET",
                &format!("/api/collections/_queue_jobs/records/{id}"),
                None,
            )
            .await;
        assert_eq!(status, 200, "{row}");
        if row["status"] == "completed" {
            final_status = Some(row);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = final_status.expect("job should complete within the poll window");
    assert_eq!(row["status"], "completed");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "handler should run exactly once"
    );
}

/// A disabled queue (the default) never mounts the plugin route at all.
#[tokio::test]
async fn the_enqueue_route_is_absent_when_the_queue_is_disabled() {
    let harness = Harness::new().await;
    assert!(
        !harness.app.settings().queue.enabled,
        "queue.enabled defaults to false"
    );

    let (status, _) = harness
        .admin(
            "POST",
            "/api/plugins/queue/enqueue",
            Some(json!({"queue": "count", "payload": {}})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
