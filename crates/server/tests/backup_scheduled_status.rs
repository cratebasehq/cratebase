//! A `settings.backups.cron` failure used to be visible only as a
//! `tracing::error!` line (`App::sync_backup_cron`) — nothing an operator
//! would see short of already tailing logs at the right moment. This
//! proves the fix end to end: force `create_scheduled` to fail
//! deterministically (no network involved — an S3 backup target with an
//! empty bucket fails `Storage`'s own config validation synchronously),
//! then confirm the failure is both queryable through the same REST API
//! the dashboard's Backups page uses (`GET /api/backups/storage-info`)
//! and recorded in `_audit_log`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_core::settings::{Backups, S3};
use cratebase_core::Settings;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use serde_json::Value;
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

    async fn raw(&self, request: Request<Body>) -> Response {
        cratebase_server::router(self.app.clone())
            .oneshot(request)
            .await
            .expect("response")
    }

    async fn get_auth(&self, uri: &str) -> (StatusCode, Value) {
        let response = self
            .raw(
                Request::get(uri)
                    .header("authorization", &self.token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, body)
    }
}

/// Points `settings.backups.s3` at a target `Storage::backups_from_settings`
/// itself rejects synchronously (`crates/storage/src/lib.rs`'s own
/// `from_settings_builds_s3_when_enabled` test proves an empty `bucket`
/// with S3 `enabled` is a config error) — so `create_scheduled` fails
/// deterministically, with no real S3/network dependency and no flakiness.
async fn break_backup_storage(app: &App) {
    let settings = Settings {
        backups: Backups {
            s3: S3 {
                enabled: true,
                bucket: String::new(),
                region: "us-east-1".into(),
                endpoint: "http://localhost:1".into(),
                access_key: "ak".into(),
                secret: "sk".into(),
                force_path_style: true,
            },
            ..Backups::default()
        },
        ..Settings::default()
    };
    app.set_settings(settings).await.expect("set_settings");
}

#[tokio::test]
async fn a_scheduled_backup_failure_is_recorded_and_visible_via_the_api() {
    let harness = Harness::new().await;
    break_backup_storage(&harness.app).await;

    // Before any scheduled run, storage-info carries no lastScheduled at
    // all — nothing to show yet.
    let (status, before) = harness.get_auth("/api/backups/storage-info").await;
    assert_eq!(status, StatusCode::OK);
    assert!(before.get("lastScheduled").is_none(), "{before:?}");

    let err = cratebase_server::routes::backups::create_scheduled(&harness.app)
        .await
        .expect_err("backup storage is broken, so the scheduled run must fail");
    let failure_message = err.to_string();

    // 1. Visible through the same API the dashboard's Backups page reads.
    let (status, after) = harness.get_auth("/api/backups/storage-info").await;
    assert_eq!(status, StatusCode::OK);
    let last_scheduled = after
        .get("lastScheduled")
        .expect("lastScheduled must be present after a scheduled run");
    assert_eq!(last_scheduled["ok"], false);
    assert!(last_scheduled["at"].is_string());
    assert_eq!(last_scheduled["message"], failure_message.as_str());

    // 2. Visible in `_audit_log`, so it shows up next to every other
    // consequential superuser-facing event.
    let (status, logs) = harness
        .get_auth("/api/collections/_audit_log/records?filter=action='backup.scheduled_failed'")
        .await;
    assert_eq!(status, StatusCode::OK);
    let items = logs["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "{logs:?}");
    assert_eq!(items[0]["target"], "settings.backups.cron");
    // A cron job has no caller behind it, so `actor` (a relation field)
    // serializes the same way an unset relation always does: empty, not
    // some human's id.
    assert_eq!(items[0]["actor"], "", "a cron job has no actor: {logs:?}");
    assert!(
        items[0]["meta"]["error"]
            .as_str()
            .unwrap()
            .contains(&failure_message)
            || failure_message.contains(items[0]["meta"]["error"].as_str().unwrap()),
        "{logs:?}"
    );
}

/// A *successful* scheduled backup must not spam `_audit_log` — only a
/// failure is audit-worthy (see `record_scheduled_backup_result`'s doc).
#[tokio::test]
async fn a_successful_scheduled_backup_is_not_audit_logged_but_is_still_visible() {
    let harness = Harness::new().await;

    cratebase_server::routes::backups::create_scheduled(&harness.app)
        .await
        .expect("in-memory sqlite backup should succeed");

    let (status, info) = harness.get_auth("/api/backups/storage-info").await;
    assert_eq!(status, StatusCode::OK);
    let last_scheduled = info
        .get("lastScheduled")
        .expect("lastScheduled must be present");
    assert_eq!(last_scheduled["ok"], true);
    assert!(last_scheduled.get("message").is_none() || last_scheduled["message"].is_null());

    let (status, logs) = harness
        .get_auth("/api/collections/_audit_log/records?filter=action='backup.scheduled_failed'")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(logs["items"].as_array().unwrap().len(), 0, "{logs:?}");
}
