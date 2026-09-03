//! End-to-end HTTP tests: drives the real axum `Router` in-process (no TCP
//! socket) against an isolated sqlite DB and temp-dir local storage per
//! test, so these exercise the exact request/response contract clients see.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cratebase_server::config::Config;
use cratebase_server::state::AppState;
use cratebase_server::{build_app, build_state};
use cratebase_storage::StorageConfig;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn test_state() -> AppState {
    let dir = std::env::temp_dir().join(format!(
        "cratebase-api-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let config = Config {
        database_url: "sqlite::memory:".to_string(),
        storage: StorageConfig::Local {
            base_dir: dir.join("storage").to_string_lossy().to_string(),
        },
        auth_secret: "test-secret".to_string(),
        host: "127.0.0.1".to_string(),
        port: 0,
        admin_token_ttl_seconds: 3600,
        auth_token_ttl_seconds: 3600,
        cors_allow_origins: vec!["*".to_string()],
        data_dir: dir.to_string_lossy().to_string(),
    };
    build_state(config).await.unwrap()
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn json_request(method: &str, uri: &str, token: Option<&str>, body: Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

fn get_request(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    builder.body(Body::empty()).unwrap()
}

/// Provisions a superuser directly against the shared `Db` (there is no
/// public "create first admin" HTTP endpoint by design — schema management
/// is superuser-only, same as `cratebase superuser create`) and logs in
/// over HTTP like a real client to get a bearer token.
async fn admin_token(state: &AppState, app: &axum::Router) -> String {
    let hash = cratebase_auth::hash_password("admin12345").unwrap();
    cratebase_db::admins::create_admin(&state.db, "admin@test.local", &hash)
        .await
        .unwrap();

    let login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admins/auth-with-password",
            None,
            json!({"email": "admin@test.local", "password": "admin12345"}),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    json_body(login).await["token"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn health_check() {
    let app = build_app(test_state().await);
    let res = app.oneshot(get_request("/api/health", None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(json_body(res).await["status"], "ok");
}

#[tokio::test]
async fn collection_management_requires_admin() {
    let app = build_app(test_state().await);
    let res = app
        .oneshot(json_request(
            "POST",
            "/api/collections",
            None,
            json!({"name": "posts", "type": "base", "schema": []}),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn full_record_lifecycle_with_public_rules() {
    let state = test_state().await;
    let app = build_app(state.clone());
    let token = admin_token(&state, &app).await;

    let create_collection = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "posts",
                "type": "base",
                "schema": [
                    {"id": "f1", "name": "title", "type": "text", "required": true},
                    {"id": "f2", "name": "published", "type": "bool"}
                ],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();
    assert_eq!(create_collection.status(), StatusCode::OK);

    let created = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/posts/records",
            None,
            json!({"title": "Hello", "published": true}),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let record = json_body(created).await;
    assert_eq!(record["title"], "Hello");
    let id = record["id"].as_str().unwrap().to_string();

    let listed = app
        .clone()
        .oneshot(get_request(
            "/api/collections/posts/records?filter=published%20%3D%20true",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(json_body(listed).await["totalItems"], 1);

    let updated = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/collections/posts/records/{id}"),
            None,
            json!({"title": "Updated"}),
        ))
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    assert_eq!(json_body(updated).await["title"], "Updated");

    let deleted = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/collections/posts/records/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let missing = app
        .oneshot(get_request(
            &format!("/api/collections/posts/records/{id}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn email_field_name_only_reserved_on_auth_collections() {
    let state = test_state().await;
    let app = build_app(state.clone());
    let token = admin_token(&state, &app).await;

    // A base collection storing contacts should be able to name a field
    // "email" — only auth collections reserve it for their own column.
    let base = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "contacts", "type": "base",
                "schema": [{"id": "f1", "name": "email", "type": "email"}],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();
    assert_eq!(base.status(), StatusCode::OK, "{:?}", json_body(base).await);

    let auth = app
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "members", "type": "auth",
                "schema": [{"id": "f1", "name": "email", "type": "text"}],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();
    assert_eq!(auth.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn locked_create_rule_rejects_anonymous_writes() {
    let state = test_state().await;
    let app = build_app(state.clone());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "secrets", "type": "base",
                "schema": [{"id": "f1", "name": "value", "type": "text"}],
                "listRule": "", "viewRule": "", "createRule": null, "updateRule": null, "deleteRule": null
            }),
        ))
        .await
        .unwrap();

    let res = app
        .oneshot(json_request(
            "POST",
            "/api/collections/secrets/records",
            None,
            json!({"value": "x"}),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_collection_register_login_and_self_update() {
    let state = test_state().await;
    let app = build_app(state.clone());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "users", "type": "auth",
                "schema": [{"id": "f1", "name": "displayName", "type": "text"}],
                "listRule": "@request.auth.id != \"\"", "viewRule": "", "createRule": "",
                "updateRule": "id = @request.auth.id", "deleteRule": null
            }),
        ))
        .await
        .unwrap();

    let register = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/records",
            None,
            json!({"email": "alice@example.com", "password": "secret123", "passwordConfirm": "secret123", "displayName": "Alice"}),
        ))
        .await
        .unwrap();
    assert_eq!(register.status(), StatusCode::OK);

    let bad_login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-password",
            None,
            json!({"identity": "alice@example.com", "password": "wrong"}),
        ))
        .await
        .unwrap();
    assert_eq!(bad_login.status(), StatusCode::UNAUTHORIZED);

    let login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-password",
            None,
            json!({"identity": "alice@example.com", "password": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let login_body = json_body(login).await;
    let user_token = login_body["token"].as_str().unwrap().to_string();
    let user_id = login_body["record"]["id"].as_str().unwrap().to_string();
    assert!(
        login_body["record"].get("password_hash").is_none(),
        "password hash must never be exposed"
    );

    let anon_list = app
        .clone()
        .oneshot(get_request("/api/collections/users/records", None))
        .await
        .unwrap();
    assert_eq!(
        json_body(anon_list).await["totalItems"],
        0,
        "anonymous must not see auth records"
    );

    let auth_list = app
        .clone()
        .oneshot(get_request(
            "/api/collections/users/records",
            Some(&user_token),
        ))
        .await
        .unwrap();
    assert_eq!(json_body(auth_list).await["totalItems"], 1);

    let self_update = app
        .oneshot(json_request(
            "PATCH",
            &format!("/api/collections/users/records/{user_id}"),
            Some(&user_token),
            json!({"displayName": "Alice Updated"}),
        ))
        .await
        .unwrap();
    assert_eq!(self_update.status(), StatusCode::OK);
    assert_eq!(json_body(self_update).await["displayName"], "Alice Updated");
}

#[tokio::test]
async fn auth_collection_with_username_identity_field() {
    let state = test_state().await;
    let app = build_app(state.clone());
    let token = admin_token(&state, &app).await;

    // An auth collection can be configured to log in with something other
    // than an email address, and admin-created records don't need
    // `passwordConfirm` at all.
    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "players", "type": "auth",
                "authOptions": {"identityField": "username"},
                "schema": [],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();

    let created = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/players/records",
            Some(&token),
            json!({"username": "neo", "password": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(
        created.status(),
        StatusCode::OK,
        "{:?}",
        json_body(created).await
    );
    let record = json_body(created).await;
    assert_eq!(record["username"], "neo");
    assert!(
        record.get("email").is_none(),
        "no email column on a username-identity collection"
    );

    let login = app
        .oneshot(json_request(
            "POST",
            "/api/collections/players/auth-with-password",
            None,
            json!({"identity": "neo", "password": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
}

#[tokio::test]
async fn file_upload_and_download_round_trip() {
    let state = test_state().await;
    let app = build_app(state.clone());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "docs", "type": "base",
                "schema": [{"id": "f1", "name": "title", "type": "text"}, {"id": "f2", "name": "attachment", "type": "file"}],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();

    let boundary = "----cratebase-test-boundary";
    let body = format!(
        "--{b}\r\ncontent-disposition: form-data; name=\"title\"\r\n\r\nReport\r\n\
         --{b}\r\ncontent-disposition: form-data; name=\"attachment\"; filename=\"note.txt\"\r\ncontent-type: text/plain\r\n\r\nhello file\r\n--{b}--\r\n",
        b = boundary
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/collections/docs/records")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let created = app.clone().oneshot(req).await.unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let record = json_body(created).await;
    let filename = record["attachment"].as_str().unwrap().to_string();
    let id = record["id"].as_str().unwrap().to_string();

    let download = app
        .oneshot(get_request(
            &format!("/api/files/docs/{id}/{filename}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(download.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"hello file");
}
