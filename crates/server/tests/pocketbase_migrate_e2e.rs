//! Live end-to-end proof that `cratebase migrate-from-pocketbase` leaves
//! the target instance with a fully working `_sessions`/`_bans`-backed
//! auth system (session listing + revocation, ban enforcement) for the
//! migrated collections — not just a schema/record copy.
//!
//! This builds a real PocketBase-shaped `data.db` (the same `_collections`
//! + per-collection-table shape `pocketbase_migrate::run` reads), runs the
//! actual migration against a fresh on-disk Cratebase data dir (a real
//! sqlite file, not `sqlite::memory:`, matching a real `--dir`), then
//! drives the production router with real HTTP requests through
//! `tower::ServiceExt::oneshot` (the same router `cratebase serve` binds)
//! to prove: (a) the migrated user can log in, (b) their login shows up
//! in `GET .../sessions`, (c) revoking that session invalidates the token
//! on the next authenticated request.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use rusqlite::Connection;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn send(app: &App, request: Request<Body>) -> (StatusCode, Value) {
    let response = cratebase_server::router(app.clone())
        .oneshot(request)
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// Build a real PocketBase `pb_data` fixture: `data.db` with one
/// non-system auth collection ("users", PocketBase's own default is
/// `system = 0`) holding a single migrated user with a bcrypt password
/// hash, exactly the shape `pocketbase_migrate::read_pb_collections` /
/// `read_pb_rows` expect.
fn build_pb_fixture(pb_dir: &std::path::Path, bcrypt_hash: &str) {
    std::fs::create_dir_all(pb_dir).unwrap();
    let conn = Connection::open(pb_dir.join("data.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE _collections (
            id TEXT PRIMARY KEY, name TEXT, type TEXT, system INTEGER,
            fields TEXT, listRule TEXT, viewRule TEXT, createRule TEXT,
            updateRule TEXT, deleteRule TEXT, options TEXT
        );
        CREATE TABLE users (
            id TEXT PRIMARY KEY, password TEXT, tokenKey TEXT, email TEXT,
            emailVisibility INTEGER, verified INTEGER, created TEXT,
            updated TEXT, name TEXT
        );",
    )
    .unwrap();

    let fields = json!([
        { "id": "text_id", "name": "id", "type": "text", "system": true },
        { "id": "password_x", "name": "password", "type": "password", "system": true },
        { "id": "tokenKey_x", "name": "tokenKey", "type": "text", "system": true },
        { "id": "email_x", "name": "email", "type": "email", "system": true },
        { "id": "emailVisibility_x", "name": "emailVisibility", "type": "bool", "system": true },
        { "id": "verified_x", "name": "verified", "type": "bool", "system": true },
        { "id": "name_x", "name": "name", "type": "text", "system": false },
        { "id": "created_x", "name": "created", "type": "autodate", "system": true },
        { "id": "updated_x", "name": "updated", "type": "autodate", "system": true },
    ]);
    let options = json!({
        "passwordAuth": { "enabled": true, "identityFields": ["email"] },
    });

    conn.execute(
        "INSERT INTO _collections (id, name, type, system, fields, listRule, viewRule, createRule, updateRule, deleteRule, options) \
         VALUES (?1, 'users', 'auth', 0, ?2, '', '', '', '', '', ?3)",
        rusqlite::params!["pbc_users_test", fields.to_string(), options.to_string()],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO users (id, password, tokenKey, email, emailVisibility, verified, created, updated, name) \
         VALUES (?1, ?2, ?3, ?4, 1, 1, ?5, ?5, ?6)",
        rusqlite::params![
            "user_migrated_1",
            bcrypt_hash,
            "pb-token-key-abc123",
            "migrated@example.com",
            "2024-01-01 00:00:00.000Z",
            "Migrated User",
        ],
    )
    .unwrap();
}

#[tokio::test]
async fn migrated_user_gets_full_session_lifecycle_and_ban_enforcement() {
    // 1. A real PocketBase-shaped pb_data dir, bcrypt password hash (the
    //    same hash format `pocketbase serve` writes).
    let pb_dir = tempfile::tempdir().unwrap();
    let bcrypt_hash = bcrypt::hash("pb-password", 4).unwrap();
    build_pb_fixture(pb_dir.path(), &bcrypt_hash);

    // 2. A fresh, real on-disk Cratebase data dir — `Config::for_data_dir`
    //    is the same constructor the CLI's `config_for` wraps, and unlike
    //    `Config::memory` it writes an actual sqlite file, matching a
    //    real `--dir` run.
    let cb_dir = tempfile::tempdir().unwrap();
    let app = App::new(Config::for_data_dir(cb_dir.path()));
    app.bootstrap().await.expect("bootstrap");

    // `_sessions`/`_bans` exist immediately after bootstrap, before the
    // pocketbase import ever runs.
    assert!(
        app.db().collections.get_by_name("_sessions").is_some(),
        "_sessions must exist right after App::bootstrap(), before migrate-from-pocketbase runs"
    );
    assert!(
        app.db().collections.get_by_name("_bans").is_some(),
        "_bans must exist right after App::bootstrap(), before migrate-from-pocketbase runs"
    );

    let report = cratebase_server::pocketbase_migrate::run(&app, pb_dir.path().to_str().unwrap())
        .await
        .expect("migration");
    assert_eq!(report.collections_updated, vec!["users".to_string()]);
    assert_eq!(report.records_migrated.get("users"), Some(&1));

    // `_sessions`/`_bans` are untouched by the import (PocketBase has no
    // equivalent tables to skip/import) and still present afterwards.
    assert!(app.db().collections.get_by_name("_sessions").is_some());
    assert!(app.db().collections.get_by_name("_bans").is_some());

    // (a) the migrated user logs in with their original PocketBase
    // password (bcrypt verified transparently by `verify_password`).
    let (status, body) = send(
        &app,
        Request::post("/api/collections/users/auth-with-password")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "identity": "migrated@example.com", "password": "pb-password" })
                    .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "login response: {body}");
    let token = body["token"].as_str().expect("token in response").to_string();
    assert_eq!(body["record"]["email"], json!("migrated@example.com"));

    // (b) that login shows up in `GET .../sessions`.
    let (status, body) = send(
        &app,
        Request::get("/api/collections/users/sessions")
            .header("authorization", &token)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "sessions list: {body}");
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "exactly one live session after one login: {body}");
    let session_id = items[0]["id"].as_str().expect("session id").to_string();
    assert_eq!(items[0]["current"], json!(true));

    // A protected route (listing own record) works before revocation.
    let (status, _) = send(
        &app,
        Request::get(format!("/api/collections/users/records/{}", "user_migrated_1"))
            .header("authorization", &token)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "authenticated read must work before revocation");

    // (c) revoking that session via the real revoke route invalidates the
    // token on the next authenticated request.
    let (status, _) = send(
        &app,
        Request::delete(format!("/api/collections/users/sessions/{session_id}"))
            .header("authorization", &token)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "revoke must succeed");

    let (status, body) = send(
        &app,
        Request::get(format!("/api/collections/users/records/{}", "user_migrated_1"))
            .header("authorization", &token)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "revoked session's token must be rejected on the next request: {body}"
    );

    // Bonus: ban enforcement also works end-to-end for a migrated user —
    // a fresh login is rejected once a superuser bans the record.
    let superuser_id = app
        .create_superuser("admin@example.com", "hunter2hunter2")
        .await
        .expect("superuser");
    let superuser_token = app
        .mint_token(
            "_superusers",
            &superuser_id,
            cratebase_auth::TokenType::Auth,
            3600,
        )
        .await
        .expect("superuser token");
    let (status, body) = send(
        &app,
        Request::post("/api/collections/users/ban/user_migrated_1")
            .header("authorization", &superuser_token)
            .header("content-type", "application/json")
            .body(Body::from(json!({ "reason": "test ban" }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "ban response: {body}");

    let (status, body) = send(
        &app,
        Request::post("/api/collections/users/auth-with-password")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "identity": "migrated@example.com", "password": "pb-password" })
                    .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "banned migrated user must not be able to log in: {body}"
    );
}
