//! `_rpc` custom SQL RPC (`crates/server/src/rpc.rs`): save-time
//! validation on `_rpc` itself, and `POST /api/rpc/{name}`.
//!
//! Most of this file runs on SQLite (no `TEST_POSTGRES_URL` needed); the
//! Postgres-only tests at the bottom cover what SQLite structurally
//! cannot (a data-modifying CTE disguised as a read, a real
//! `pg_sleep`-based timeout) and skip when `TEST_POSTGRES_URL` is unset,
//! matching every other Postgres-gated test in this crate.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use serde_json::{json, Value};
use tower::ServiceExt;

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
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let router = cratebase_server::router(app.clone());
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", token);
    }
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

/// A real file-backed SQLite database, not `sqlite::memory:` — that
/// matters specifically for `read_only_blocks_a_write_statement`: an
/// in-memory engine has no reader pool at all (`reader_count == 0`), so
/// every read silently falls back to the writer connection with none of
/// `PRAGMA query_only`'s enforcement a real deployment gets. A real file
/// exercises the actual reader pool, the same way production does.
async fn memory_app() -> App {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(Config {
        log_requests: false,
        secret: "test-secret-0123456789".into(),
        ..Config::for_data_dir(dir.path())
    });
    // Leak the tempdir on purpose: the test only needs `app` (and the
    // database file underneath it) to outlive the request.
    std::mem::forget(dir);
    app.bootstrap().await.expect("bootstrap");
    app
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

/// Create a regular (non-auth) `things` collection with public list/view
/// rules and one `label` text field, seeded with two rows — enough for an
/// RPC to have something real to `SELECT` from.
async fn seed_things(app: &App, token: &str) {
    let schema = json!({
        "name": "things",
        "type": "base",
        "fields": [
            {"name": "label", "type": "text"}
        ],
        "listRule": "",
        "viewRule": "",
    });
    let (status, body) = request(app, "POST", "/api/collections", Some(token), Some(schema)).await;
    assert_eq!(status, StatusCode::OK, "create things collection: {body:?}");

    for label in ["alpha", "beta"] {
        let (status, body) = request(
            app,
            "POST",
            "/api/collections/things/records",
            Some(token),
            Some(json!({ "label": label })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "seed {label}: {body:?}");
    }
}

async fn create_rpc(app: &App, token: &str, def: Value) -> (StatusCode, Value) {
    request(
        app,
        "POST",
        "/api/collections/_rpc/records",
        Some(token),
        Some(def),
    )
    .await
}

#[tokio::test]
async fn creating_an_rpc_requires_a_superuser() {
    let app = memory_app().await;
    let (status, body) =
        create_rpc(&app, "", json!({ "name": "no_auth", "sql": "SELECT 1" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body:?}");
}

#[tokio::test]
async fn save_time_validation_rejects_bad_definitions() {
    let app = memory_app().await;
    let token = owner_token(&app).await;

    // Invalid name (not an identifier).
    let (status, body) = create_rpc(
        &app,
        &token,
        json!({ "name": "not-a-slug", "sql": "SELECT 1" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");

    // Multiple statements.
    let (status, body) = create_rpc(
        &app,
        &token,
        json!({ "name": "multi", "sql": "SELECT 1; SELECT 2" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");

    // References an undeclared parameter.
    let (status, body) = create_rpc(
        &app,
        &token,
        json!({ "name": "undeclared", "sql": "SELECT :x" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");

    // Doesn't parse against the real driver.
    let (status, body) = create_rpc(
        &app,
        &token,
        json!({ "name": "broken", "sql": "SELECT * FROM this_table_does_not_exist" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");

    // A well-formed definition is accepted, and defaults are applied.
    let (status, body) = create_rpc(&app, &token, json!({ "name": "ok", "sql": "SELECT 1" })).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["readOnly"], true);
    assert_eq!(body["timeoutMs"], 5000);
    assert_eq!(body["maxRows"], 1000);
}

#[tokio::test]
async fn unknown_rpc_is_404() {
    let app = memory_app().await;
    let (status, body) = request(&app, "POST", "/api/rpc/nope", None, Some(json!({}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body:?}");
}

#[tokio::test]
async fn a_public_rule_is_callable_by_anyone() {
    let app = memory_app().await;
    let token = owner_token(&app).await;
    seed_things(&app, &token).await;

    let (status, body) = create_rpc(
        &app,
        &token,
        json!({
            "name": "list_things",
            "sql": "SELECT label FROM things ORDER BY label",
            "rule": "",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    // No auth at all.
    let (status, body) = request(&app, "POST", "/api/rpc/list_things", None, Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["label"], "alpha");
    assert_eq!(items[1]["label"], "beta");
}

#[tokio::test]
async fn a_null_rule_is_superuser_only() {
    let app = memory_app().await;
    let token = owner_token(&app).await;
    seed_things(&app, &token).await;

    // `rule` omitted entirely -> null -> superuser only.
    let (status, body) = create_rpc(
        &app,
        &token,
        json!({ "name": "su_only", "sql": "SELECT COUNT(*) AS n FROM things" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["rule"], Value::Null);

    let (status, body) = request(&app, "POST", "/api/rpc/su_only", None, Some(json!({}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body:?}");

    let (status, body) = request(
        &app,
        "POST",
        "/api/rpc/su_only",
        Some(&token),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["items"][0]["n"], 2);
}

#[tokio::test]
async fn a_conditional_rule_evaluates_request_body_and_auth() {
    let app = memory_app().await;
    let token = owner_token(&app).await;
    seed_things(&app, &token).await;

    let (status, body) = create_rpc(
        &app,
        &token,
        json!({
            "name": "gated",
            "sql": "SELECT :label AS echoed",
            "params": [{ "name": "label", "type": "text", "required": true }],
            "rule": "@request.body.label != ''",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    // Fails the rule: empty label.
    let (status, body) = request(
        &app,
        "POST",
        "/api/rpc/gated",
        None,
        Some(json!({ "label": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body:?}");

    // Passes the rule.
    let (status, body) = request(
        &app,
        "POST",
        "/api/rpc/gated",
        None,
        Some(json!({ "label": "hi" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["items"][0]["echoed"], "hi");
}

#[tokio::test]
async fn missing_required_param_is_a_400_and_defaults_apply() {
    let app = memory_app().await;
    let token = owner_token(&app).await;

    let (status, _) = create_rpc(
        &app,
        &token,
        json!({
            "name": "greet",
            "sql": "SELECT :name AS greeting",
            "params": [
                { "name": "name", "type": "text", "required": true },
                { "name": "loud", "type": "bool", "default": false },
            ],
            "rule": "",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = request(&app, "POST", "/api/rpc/greet", None, Some(json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");

    let (status, body) = request(
        &app,
        "POST",
        "/api/rpc/greet",
        None,
        Some(json!({ "name": "world" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["items"][0]["greeting"], "world");
}

/// An injection attempt through a bound parameter stays a literal value —
/// it can never terminate the statement or run a second one, because it
/// is never concatenated into `sql` at all.
#[tokio::test]
async fn a_parameter_value_cannot_inject_sql() {
    let app = memory_app().await;
    let token = owner_token(&app).await;
    seed_things(&app, &token).await;

    let (status, _) = create_rpc(
        &app,
        &token,
        json!({
            "name": "find_thing",
            "sql": "SELECT label FROM things WHERE label = :label",
            "params": [{ "name": "label", "type": "text", "required": true }],
            "rule": "",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let injection = "x' OR '1'='1";
    let (status, body) = request(
        &app,
        "POST",
        "/api/rpc/find_thing",
        None,
        Some(json!({ "label": injection })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    // Matches nothing — the attempted injection is just a literal string,
    // not a `WHERE label = 'x' OR '1'='1'` clause.
    assert_eq!(body["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn read_only_blocks_a_write_statement() {
    let app = memory_app().await;
    let token = owner_token(&app).await;
    seed_things(&app, &token).await;

    // readOnly defaults to true.
    let (status, _) = create_rpc(
        &app,
        &token,
        json!({
            "name": "sneaky_delete",
            "sql": "DELETE FROM things",
            "rule": "",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = request(
        &app,
        "POST",
        "/api/rpc/sneaky_delete",
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");

    // Nothing was actually deleted.
    let (status, body) = request(
        &app,
        "GET",
        "/api/collections/things/records",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["totalItems"], 2);
}

#[tokio::test]
async fn read_only_false_allows_a_write() {
    let app = memory_app().await;
    let token = owner_token(&app).await;
    seed_things(&app, &token).await;

    let (status, body) = create_rpc(
        &app,
        &token,
        json!({
            "name": "add_thing",
            "sql": "INSERT INTO things (id, label) VALUES (:id, :label)",
            "params": [
                { "name": "id", "type": "text", "required": true },
                { "name": "label", "type": "text", "required": true },
            ],
            "readOnly": false,
            "rule": "",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    let (status, body) = request(
        &app,
        "POST",
        "/api/rpc/add_thing",
        None,
        Some(json!({ "id": "manualid1234567890123", "label": "gamma" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    let (status, body) = request(
        &app,
        "GET",
        "/api/collections/things/records",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["totalItems"], 3);
}

#[tokio::test]
async fn max_rows_truncates_the_response() {
    let app = memory_app().await;
    let token = owner_token(&app).await;
    seed_things(&app, &token).await;

    let (status, _) = create_rpc(
        &app,
        &token,
        json!({
            "name": "capped",
            "sql": "SELECT label FROM things ORDER BY label",
            "maxRows": 1,
            "rule": "",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = request(&app, "POST", "/api/rpc/capped", None, Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
}

// --------------------------------------------------------------- Postgres

fn node_config(url: &str, dir: &std::path::Path) -> Config {
    Config {
        database_url: url.to_string(),
        log_requests: false,
        secret: "test-secret-0123456789".into(),
        ..Config::for_data_dir(dir)
    }
}

async fn wipe_and_connect(url: &str) {
    use cratebase_db::{Db, Executor};
    let dir = tempfile::tempdir().unwrap();
    let wipe = Db::connect(url, &dir.path().to_string_lossy())
        .await
        .expect("connect to wipe schema");
    wipe.execute("DROP SCHEMA public CASCADE", &[])
        .await
        .unwrap();
    wipe.execute("CREATE SCHEMA public", &[]).await.unwrap();
    wipe.close().await.unwrap();
}

static ONE_TEST_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The same "write disguised as a read" gap `crates/server/tests/
/// sql_console.rs` proves closed for `/api/sql`: a data-modifying CTE
/// that starts and ends like a `SELECT` must still be rejected by a
/// `readOnly: true` RPC on Postgres, because [`Executor::query_interruptible`]
/// runs it inside `BEGIN READ ONLY` regardless of what the statement's
/// own syntax looks like.
#[tokio::test]
async fn read_only_blocks_a_data_modifying_cte_on_postgres() {
    let _guard = ONE_TEST_AT_A_TIME.lock().await;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping read_only_blocks_a_data_modifying_cte_on_postgres: TEST_POSTGRES_URL not set"
        );
        return;
    };
    wipe_and_connect(&url).await;

    let dir = tempfile::tempdir().unwrap();
    let app = App::new(node_config(&url, dir.path()));
    app.bootstrap().await.expect("bootstrap");
    let token = owner_token(&app).await;
    seed_things(&app, &token).await;

    let cte_sql = "WITH x AS (DELETE FROM things RETURNING 1) SELECT * FROM x";
    let (status, body) = create_rpc(
        &app,
        &token,
        json!({ "name": "cte_delete", "sql": cte_sql, "rule": "" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    let (status, body) = request(&app, "POST", "/api/rpc/cte_delete", None, Some(json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");

    let (status, body) = request(
        &app,
        "GET",
        "/api/collections/things/records",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["totalItems"], 2);
}

/// `timeoutMs` is enforced for real on Postgres (`SET LOCAL
/// statement_timeout`, see `PostgresEngine::query_interruptible`), not
/// just as a client-side wait.
#[tokio::test]
async fn timeout_is_enforced_on_postgres() {
    let _guard = ONE_TEST_AT_A_TIME.lock().await;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!("skipping timeout_is_enforced_on_postgres: TEST_POSTGRES_URL not set");
        return;
    };
    wipe_and_connect(&url).await;

    let dir = tempfile::tempdir().unwrap();
    let app = App::new(node_config(&url, dir.path()));
    app.bootstrap().await.expect("bootstrap");
    let token = owner_token(&app).await;

    let (status, body) = create_rpc(
        &app,
        &token,
        json!({
            "name": "slow",
            "sql": "SELECT pg_sleep(2)",
            "timeoutMs": 200,
            "rule": "",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    let start = std::time::Instant::now();
    let (status, body) = request(&app, "POST", "/api/rpc/slow", None, Some(json!({}))).await;
    let elapsed = start.elapsed();
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "must be cancelled well before the statement's own 2s sleep: {elapsed:?}"
    );
}
