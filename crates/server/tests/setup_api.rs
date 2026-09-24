//! `POST /api/setup` / `GET /api/setup/status` — first-run superuser
//! creation (`crates/server/src/routes/setup.rs`).
//!
//! This endpoint had no test coverage at all before this file: it is
//! reachable only on a fresh, unauthenticated instance, which every other
//! test harness in this crate bypasses by calling `App::create_superuser`
//! directly during setup. Two properties matter enough to pin down here:
//!
//! * **The install token.** `POST /api/setup` must refuse a request with
//!   no token, or the wrong one, and only accept the one `App::bootstrap`
//!   generated (or `CB_SETUP_TOKEN` set explicitly).
//! * **The race.** The handler used to check "does a superuser exist" and
//!   insert as two separate, unsynchronized statements, so N concurrent
//!   calls could all pass the check before any of them inserted —
//!   `concurrent_setup_calls_create_exactly_one_superuser` fires several
//!   at once and asserts only one ever succeeds and only one row exists.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use cratebase_db::Executor;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "test-install-token-0123456789";

/// An in-memory `Config` with a fixed, known setup token — no env var, so
/// tests in this file can run in parallel without racing each other over
/// a process-wide environment variable.
fn memory_config(dir: &std::path::Path) -> Config {
    Config {
        setup_token: Some(TOKEN.to_string()),
        ..Config::memory(dir)
    }
}

/// A freshly bootstrapped, unclaimed app: no superuser, `setup_token()`
/// set to [`TOKEN`].
async fn boot() -> (App, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let app = App::new(memory_config(dir.path()));
    app.bootstrap().await.expect("bootstrap");
    (app, dir)
}

async fn send(app: &App, request: Request<Body>) -> (StatusCode, Value) {
    let response = cratebase_server::router(app.clone())
        .oneshot(request)
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json body")
    };
    (status, value)
}

async fn get_status(app: &App) -> (StatusCode, Value) {
    send(
        app,
        Request::get("/api/setup/status")
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

async fn post_setup(app: &App, body: Value) -> (StatusCode, Value) {
    send(
        app,
        Request::post("/api/setup")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
}

async fn superuser_count(app: &App) -> usize {
    app.db()
        .query(r#"SELECT "id" FROM "_superusers""#, &[])
        .await
        .expect("count superusers")
        .len()
}

#[tokio::test]
async fn status_reports_needs_setup_until_a_superuser_exists() {
    let (app, _dir) = boot().await;
    let (status, body) = get_status(&app).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "needsSetup": true }));

    let (status, _) = post_setup(
        &app,
        json!({
            "email": "owner@example.com",
            "password": "hunter2hunter2",
            "passwordConfirm": "hunter2hunter2",
            "token": TOKEN,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = get_status(&app).await;
    assert_eq!(body, json!({ "needsSetup": false }));
}

#[tokio::test]
async fn setup_without_a_token_is_rejected() {
    let (app, _dir) = boot().await;
    let (status, body) = post_setup(
        &app,
        json!({
            "email": "owner@example.com",
            "password": "hunter2hunter2",
            "passwordConfirm": "hunter2hunter2",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body["message"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("token"));
    assert_eq!(superuser_count(&app).await, 0);
}

#[tokio::test]
async fn setup_with_the_wrong_token_is_rejected() {
    let (app, _dir) = boot().await;
    let (status, _) = post_setup(
        &app,
        json!({
            "email": "owner@example.com",
            "password": "hunter2hunter2",
            "passwordConfirm": "hunter2hunter2",
            "token": "definitely-not-it",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(superuser_count(&app).await, 0);
}

#[tokio::test]
async fn setup_with_the_right_token_creates_the_owner_and_then_closes() {
    let (app, _dir) = boot().await;
    let (status, _) = post_setup(
        &app,
        json!({
            "email": "owner@example.com",
            "password": "hunter2hunter2",
            "passwordConfirm": "hunter2hunter2",
            "token": TOKEN,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(superuser_count(&app).await, 1);

    // Closed for good — even the right token doesn't reopen it.
    let (status, body) = post_setup(
        &app,
        json!({
            "email": "second@example.com",
            "password": "hunter2hunter2",
            "passwordConfirm": "hunter2hunter2",
            "token": TOKEN,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["message"], json!("A superuser already exists."));
    assert_eq!(superuser_count(&app).await, 1);
}

#[tokio::test]
async fn setup_accepts_the_token_as_a_header_too() {
    let (app, _dir) = boot().await;
    let (status, _) = send(
        &app,
        Request::post("/api/setup")
            .header("content-type", "application/json")
            .header("x-setup-token", TOKEN)
            .body(Body::from(
                json!({
                    "email": "owner@example.com",
                    "password": "hunter2hunter2",
                    "passwordConfirm": "hunter2hunter2",
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(superuser_count(&app).await, 1);
}

/// The race this file exists to close. Before the fix, "does a superuser
/// exist" and the insert were two separate statements with nothing
/// synchronizing them, so more than one of these concurrent calls could
/// pass the check before either had inserted — two owners on one fresh
/// instance. Now they share one write scope (SQLite's single writer lock
/// here; an advisory lock on Postgres, see `concurrent_setup_calls_...
/// _on_postgres` below), so exactly one can ever win.
#[tokio::test]
async fn concurrent_setup_calls_create_exactly_one_superuser() {
    let (app, _dir) = boot().await;
    const N: usize = 8;
    let mut tasks = Vec::with_capacity(N);
    for i in 0..N {
        let app = app.clone();
        tasks.push(tokio::spawn(async move {
            post_setup(
                &app,
                json!({
                    "email": format!("owner{i}@example.com"),
                    "password": "hunter2hunter2",
                    "passwordConfirm": "hunter2hunter2",
                    "token": TOKEN,
                }),
            )
            .await
        }));
    }

    let mut ok = 0;
    let mut forbidden = 0;
    for task in tasks {
        match task.await.expect("task panicked").0 {
            StatusCode::OK => ok += 1,
            StatusCode::FORBIDDEN => forbidden += 1,
            other => panic!("unexpected status {other}"),
        }
    }
    assert_eq!(ok, 1, "exactly one concurrent setup call should succeed");
    assert_eq!(forbidden, N - 1);
    assert_eq!(superuser_count(&app).await, 1);
}

/// Same race, same assertion, against a real Postgres instance — the
/// engine `count_owners`'s `FOR UPDATE` trick can't help with (an empty
/// table has no row to lock), which is exactly why `create_first_superuser`
/// takes `pg_advisory_xact_lock` instead. Skips when `TEST_POSTGRES_URL`
/// is unset, matching every other Postgres-only test in this crate.
#[tokio::test]
async fn concurrent_setup_calls_create_exactly_one_superuser_on_postgres() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping concurrent_setup_calls_create_exactly_one_superuser_on_postgres: \
             TEST_POSTGRES_URL not set"
        );
        return;
    };

    // Fresh schema on its own short-lived connection, before the app
    // opens its own pool — same convention as `postgres_multi_node.rs`.
    {
        let dir = tempfile::tempdir().unwrap();
        let wipe = cratebase_db::Db::connect(&url, &dir.path().to_string_lossy())
            .await
            .expect("connect to wipe schema");
        wipe.execute("DROP SCHEMA public CASCADE", &[])
            .await
            .unwrap();
        wipe.execute("CREATE SCHEMA public", &[]).await.unwrap();
        wipe.close().await.unwrap();
    }

    let dir = tempfile::tempdir().unwrap();
    let app = App::new(Config {
        database_url: url,
        log_requests: false,
        secret: "test-secret-0123456789".into(),
        setup_token: Some(TOKEN.to_string()),
        ..Config::for_data_dir(dir.path())
    });
    app.bootstrap().await.expect("bootstrap");

    const N: usize = 8;
    let mut tasks = Vec::with_capacity(N);
    for i in 0..N {
        let app = app.clone();
        tasks.push(tokio::spawn(async move {
            post_setup(
                &app,
                json!({
                    "email": format!("owner{i}@example.com"),
                    "password": "hunter2hunter2",
                    "passwordConfirm": "hunter2hunter2",
                    "token": TOKEN,
                }),
            )
            .await
        }));
    }

    let mut ok = 0;
    let mut forbidden = 0;
    for task in tasks {
        match task.await.expect("task panicked").0 {
            StatusCode::OK => ok += 1,
            StatusCode::FORBIDDEN => forbidden += 1,
            other => panic!("unexpected status {other}"),
        }
    }
    assert_eq!(ok, 1, "exactly one concurrent setup call should succeed");
    assert_eq!(forbidden, N - 1);
    assert_eq!(superuser_count(&app).await, 1);
}
