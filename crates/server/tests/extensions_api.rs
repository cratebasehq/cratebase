//! `GET/POST/DELETE /api/db/extensions[/{name}]` (see
//! `crates/server/src/routes/extensions.rs`).
//!
//! Postgres-only: skips (rather than fails) when `TEST_POSTGRES_URL` is
//! unset, and wipes `public` on that shared database before running,
//! exactly `crates/db/tests/postgres.rs`'s convention (a fresh schema
//! also means no leftover extension from a previous run — `DROP SCHEMA
//! ... CASCADE` takes any extension installed into `public` with it).

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_db::{Db, Executor};
use cratebase_server::app::App;
use cratebase_server::config::Config;
use serde_json::Value;
use tower::ServiceExt;

/// Serializes every test in this file against the one shared
/// `TEST_POSTGRES_URL` database, same reasoning as
/// `crates/db/tests/postgres.rs`'s `ONE_TEST_AT_A_TIME`.
static ONE_TEST_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn node_config(url: &str, dir: &std::path::Path) -> Config {
    Config {
        database_url: url.to_string(),
        log_requests: false,
        secret: "test-secret-0123456789".into(),
        ..Config::for_data_dir(dir)
    }
}

async fn split(response: Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read body");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

async fn request(
    app: &App,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let router = cratebase_server::router(app.clone());
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", token);
    let body = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    split(
        router
            .oneshot(builder.body(body).unwrap())
            .await
            .expect("response"),
    )
    .await
}

async fn owner_token(app: &App) -> String {
    let id = app
        .create_superuser("owner@example.com", "password12345")
        .await
        .expect("create owner superuser");
    app.mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
        .await
        .expect("mint token")
}

#[tokio::test]
async fn extensions_are_postgres_only_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(Config {
        database_url: "sqlite::memory:".into(),
        log_requests: false,
        secret: "test-secret-0123456789".into(),
        ..Config::for_data_dir(dir.path())
    });
    app.bootstrap().await.expect("bootstrap");
    let token = owner_token(&app).await;

    let (status, body) = request(&app, "GET", "/api/db/extensions", &token, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body:?}");
}

#[tokio::test]
async fn list_install_and_drop_a_postgres_extension() {
    let _guard = ONE_TEST_AT_A_TIME.lock().await;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!("skipping list_install_and_drop_a_postgres_extension: TEST_POSTGRES_URL not set");
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

    let dir = tempfile::tempdir().unwrap();
    let app = App::new(node_config(&url, dir.path()));
    app.bootstrap().await.expect("bootstrap");
    let token = owner_token(&app).await;

    // Not installed yet.
    let (status, body) = request(&app, "GET", "/api/db/extensions", &token, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    let items = body["items"].as_array().expect("items array");
    assert!(
        !items.is_empty(),
        "pg_available_extensions should be non-empty"
    );
    let trgm = items
        .iter()
        .find(|e| e["name"] == "pg_trgm")
        .expect("pg_trgm listed as available");
    assert_eq!(trgm["installed"], false);
    assert_eq!(trgm["installedVersion"], Value::Null);

    // A malformed name never reaches the database.
    let (status, body) = request(
        &app,
        "POST",
        "/api/db/extensions/pg_trgm%3B%20DROP%20TABLE%20users",
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");

    // Install it.
    let (status, body) = request(&app, "POST", "/api/db/extensions/pg_trgm", &token, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["installed"], true);

    // The install is audited.
    let audit_count = app
        .db()
        .query_scalar(
            r#"SELECT COUNT(*) FROM "_audit_log" WHERE "action" = 'extension.create'"#,
            &[],
        )
        .await
        .unwrap()
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    assert_eq!(audit_count, 1);

    // Idempotent: installing again (IF NOT EXISTS) still succeeds (and is
    // audited again — every successful call is logged, whether or not it
    // actually changed anything, same as the rest of `_audit_log`).
    let (status, _) = request(&app, "POST", "/api/db/extensions/pg_trgm", &token, None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = request(&app, "GET", "/api/db/extensions", &token, None).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    let trgm = items.iter().find(|e| e["name"] == "pg_trgm").unwrap();
    assert_eq!(trgm["installed"], true);
    assert!(trgm["installedVersion"].is_string());
    assert_eq!(trgm["schema"], "public");

    // Drop it.
    let (status, body) = request(&app, "DELETE", "/api/db/extensions/pg_trgm", &token, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["installed"], false);

    let (status, body) = request(&app, "GET", "/api/db/extensions", &token, None).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    let trgm = items.iter().find(|e| e["name"] == "pg_trgm").unwrap();
    assert_eq!(trgm["installed"], false);

    let audit_count = app
        .db()
        .query_scalar(
            r#"SELECT COUNT(*) FROM "_audit_log" WHERE "action" = 'extension.delete'"#,
            &[],
        )
        .await
        .unwrap()
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    assert_eq!(audit_count, 1);
}

/// `App::postgis_available()` (the cached flag `crates/filter`'s geo
/// acceleration reads synchronously, see `AppInner::postgis_available`'s
/// doc) starts `false`, flips to `true` the moment `postgis` is
/// installed through this API — not just after a restart — and back to
/// `false` on drop.
#[tokio::test]
async fn installing_postgis_updates_the_cached_flag_immediately() {
    let _guard = ONE_TEST_AT_A_TIME.lock().await;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping installing_postgis_updates_the_cached_flag_immediately: TEST_POSTGRES_URL not set"
        );
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

    let dir = tempfile::tempdir().unwrap();
    let app = App::new(node_config(&url, dir.path()));
    app.bootstrap().await.expect("bootstrap");
    assert!(
        !app.postgis_available(),
        "freshly wiped database must not report postgis as available"
    );
    let token = owner_token(&app).await;

    let (status, body) = request(&app, "POST", "/api/db/extensions/postgis", &token, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert!(app.postgis_available());

    let (status, body) = request(&app, "DELETE", "/api/db/extensions/postgis", &token, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert!(!app.postgis_available());
}

#[tokio::test]
async fn dropping_an_extension_with_dependents_needs_cascade() {
    let _guard = ONE_TEST_AT_A_TIME.lock().await;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping dropping_an_extension_with_dependents_needs_cascade: TEST_POSTGRES_URL not set"
        );
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

    let dir = tempfile::tempdir().unwrap();
    let app = App::new(node_config(&url, dir.path()));
    app.bootstrap().await.expect("bootstrap");
    let token = owner_token(&app).await;

    let (status, _) = request(&app, "POST", "/api/db/extensions/pg_trgm", &token, None).await;
    assert_eq!(status, StatusCode::OK);

    // A dependent object: an index using pg_trgm's operator class.
    app.db()
        .execute(r#"CREATE TABLE "trgm_dep" ("name" TEXT)"#, &[])
        .await
        .unwrap();
    app.db()
        .execute(
            r#"CREATE INDEX "trgm_dep_name_idx" ON "trgm_dep" USING gin ("name" gin_trgm_ops)"#,
            &[],
        )
        .await
        .unwrap();

    // Plain drop is refused by Postgres itself (dependent index).
    let (status, body) = request(&app, "DELETE", "/api/db/extensions/pg_trgm", &token, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");

    // `?cascade=true` succeeds and takes the dependent index with it.
    let (status, body) = request(
        &app,
        "DELETE",
        "/api/db/extensions/pg_trgm?cascade=true",
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
}
