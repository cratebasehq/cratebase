//! End-to-end HTTP tests: drives the real axum `Router` in-process (no TCP
//! socket) against an isolated sqlite DB and temp-dir local storage per
//! test, so these exercise the exact request/response contract clients see.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cratebase_mailer::MailerConfig;
use cratebase_server::config::Config;
use cratebase_server::state::AppState;
use cratebase_server::{build_app, build_state};
use cratebase_storage::StorageConfig;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn test_state() -> AppState {
    test_state_with_rate_limit(false).await
}

async fn test_state_with_rate_limit(auth_rate_limit_enabled: bool) -> AppState {
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
        // `tower::ServiceExt::oneshot` (used throughout this file) never
        // populates `ConnectInfo<SocketAddr>`, so the rate limiter's IP
        // key extractor would reject every login request with no peer
        // address to key on unless the request sets `x-forwarded-for`
        // itself (`SmartIpKeyExtractor` checks that header first). Real
        // requests through `axum::serve` carry a real peer address — see
        // `main.rs`'s `into_make_service_with_connect_info`.
        auth_rate_limit_enabled,
        mailer: MailerConfig::Log,
        mail_from_address: "no-reply@test.local".to_string(),
        mail_from_name: "Cratebase Test".to_string(),
        public_app_url: "http://localhost:8090".to_string(),
        verification_token_ttl_seconds: 86_400,
        password_reset_token_ttl_seconds: 3_600,
        email_change_token_ttl_seconds: 3_600,
        file_token_ttl_seconds: 5,
        otp_token_ttl_seconds: 300,
        oauth_providers: Vec::new(),
    };
    build_state(config).await.unwrap()
}

async fn test_state_with_otp_ttl(otp_token_ttl_seconds: i64) -> AppState {
    let dir = std::env::temp_dir().join(format!(
        "cratebase-api-test-otp-{}",
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
        auth_rate_limit_enabled: false,
        mailer: MailerConfig::Log,
        mail_from_address: "no-reply@test.local".to_string(),
        mail_from_name: "Cratebase Test".to_string(),
        public_app_url: "http://localhost:8090".to_string(),
        verification_token_ttl_seconds: 86_400,
        password_reset_token_ttl_seconds: 3_600,
        email_change_token_ttl_seconds: 3_600,
        file_token_ttl_seconds: 5,
        otp_token_ttl_seconds,
        oauth_providers: Vec::new(),
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

/// Recovers the plaintext OTP code the server just minted for
/// `(collection_id, record_id)`, for tests to drive `auth-with-otp`/
/// `mfa/confirm` without a real mailer to read the email from. The row
/// only stores `hash_otp(code)` (never the plaintext — same as
/// production), so this reads the stored hash straight out of
/// `_otp_codes` and brute-forces the 10^6-code space to find the
/// matching plaintext, exactly what makes a 6-digit OTP "safe": not the
/// hash, the short TTL and single-use consumption around it.
async fn find_otp_code(state: &AppState, collection_id: &str, record_id: &str) -> String {
    let row: (String,) = sqlx::query_as(
        "SELECT code_hash FROM _otp_codes WHERE collection_id = $1 AND record_id = $2",
    )
    .bind(collection_id)
    .bind(record_id)
    .fetch_one(&state.db.pool)
    .await
    .unwrap();
    let hash = row.0;
    for candidate in 0u32..1_000_000 {
        let code = format!("{candidate:06}");
        if cratebase_auth::hash_otp(&code) == hash {
            return code;
        }
    }
    panic!("no otp code in 0..1_000_000 hashes to the stored code_hash");
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
    let app = build_app(test_state().await, &cratebase_server::plugins::registry());
    let res = app.oneshot(get_request("/api/health", None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(json_body(res).await["status"], "ok");
}

#[tokio::test]
async fn plugin_stats_route_reports_record_counts() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "widgets", "type": "base",
                "schema": [{"id": "f1", "name": "name", "type": "text"}],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/widgets/records",
            None,
            json!({"name": "gizmo"}),
        ))
        .await
        .unwrap();

    let stats = app
        .oneshot(get_request("/api/plugins/example/stats", None))
        .await
        .unwrap();
    assert_eq!(stats.status(), StatusCode::OK);
    assert_eq!(json_body(stats).await["recordCounts"]["widgets"], 1);
}

#[tokio::test]
async fn collection_management_requires_admin() {
    let app = build_app(test_state().await, &cratebase_server::plugins::registry());
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
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
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
async fn view_collection_lists_filtered_rows_and_rejects_writes() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    app.clone()
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

    for (title, published) in [
        ("Alpha", true),
        ("Bravo", false),
        ("Charlie", true),
        ("Delta", true),
    ] {
        let res = app
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/collections/posts/records",
                None,
                json!({"title": title, "published": published}),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    let create_view = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "published_posts",
                "type": "view",
                "schema": [{"id": "vf1", "name": "title", "type": "text"}],
                "viewQuery": "SELECT id, title, created, updated FROM cb_posts WHERE published = 1",
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        create_view.status(),
        StatusCode::OK,
        "{:?}",
        json_body(create_view).await
    );

    // Unfiltered list only ever surfaces the three published rows — the
    // `WHERE published = 1` baked into the view query, not anything the
    // client asked for.
    let listed = app
        .clone()
        .oneshot(get_request(
            "/api/collections/published_posts/records",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let body = json_body(listed).await;
    assert_eq!(body["totalItems"], 3);
    let titles: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap())
        .collect();
    assert!(!titles.contains(&"Bravo"), "unpublished row must be excluded");

    // `?filter=` composes with the view exactly like it does for a table.
    let filtered = app
        .clone()
        .oneshot(get_request(
            "/api/collections/published_posts/records?filter=title~%22Charlie%22",
            None,
        ))
        .await
        .unwrap();
    let filtered_body = json_body(filtered).await;
    assert_eq!(filtered_body["totalItems"], 1);
    assert_eq!(filtered_body["items"][0]["title"], "Charlie");

    // `?sort=` and pagination also work unmodified against the view.
    let sorted = app
        .clone()
        .oneshot(get_request(
            "/api/collections/published_posts/records?sort=-title&perPage=2&page=1",
            None,
        ))
        .await
        .unwrap();
    let sorted_body = json_body(sorted).await;
    assert_eq!(sorted_body["totalItems"], 3);
    assert_eq!(sorted_body["totalPages"], 2);
    assert_eq!(sorted_body["items"].as_array().unwrap().len(), 2);
    assert_eq!(sorted_body["items"][0]["title"], "Delta");
    assert_eq!(sorted_body["items"][1]["title"], "Charlie");

    // Writes against a view collection are rejected outright.
    let create_record = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/published_posts/records",
            None,
            json!({"title": "Echo"}),
        ))
        .await
        .unwrap();
    assert_eq!(create_record.status(), StatusCode::BAD_REQUEST);

    let first_id = body["items"][0]["id"].as_str().unwrap().to_string();
    let update_record = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/collections/published_posts/records/{first_id}"),
            None,
            json!({"title": "Nope"}),
        ))
        .await
        .unwrap();
    assert_eq!(update_record.status(), StatusCode::BAD_REQUEST);

    let delete_record = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/collections/published_posts/records/{first_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete_record.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn email_field_name_only_reserved_on_auth_collections() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
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
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
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
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "PATCH",
            "/api/collections/users",
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
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
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
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
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

#[tokio::test]
async fn file_upload_rejects_disallowed_mime_and_oversized_file() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "photos", "type": "base",
                "schema": [{
                    "id": "f1", "name": "image", "type": "file",
                    "options": {"mimeTypes": ["image/png"], "maxSize": 5}
                }],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();

    // Wrong mime type: field only accepts image/png, upload sends text/plain.
    let boundary = "----cratebase-test-boundary";
    let body = format!(
        "--{b}\r\ncontent-disposition: form-data; name=\"image\"; filename=\"note.txt\"\r\ncontent-type: text/plain\r\n\r\nhello\r\n--{b}--\r\n",
        b = boundary
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/collections/photos/records")
        .header("content-type", format!("multipart/form-data; boundary={boundary}"))
        .body(Body::from(body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = json_body(res).await;
    assert_eq!(body["data"]["image"]["code"], "invalid_mime_type");

    // Right mime type, but bytes exceed the 5-byte maxSize.
    let body = format!(
        "--{b}\r\ncontent-disposition: form-data; name=\"image\"; filename=\"pic.png\"\r\ncontent-type: image/png\r\n\r\ntoo many bytes\r\n--{b}--\r\n",
        b = boundary
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/collections/photos/records")
        .header("content-type", format!("multipart/form-data; boundary={boundary}"))
        .body(Body::from(body))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = json_body(res).await;
    assert_eq!(body["data"]["image"]["code"], "file_too_large");
}

#[tokio::test]
async fn realtime_publish_enforces_list_rule_per_subscriber() {
    use cratebase_core::field::FieldType;
    use cratebase_core::{now, Collection, CollectionType, Field, FieldOptions};
    use cratebase_db::resolver::AuthContext;
    use cratebase_server::realtime::RealtimeHub;

    let state = test_state().await;

    let collection = Collection {
        id: "c1".into(),
        name: "posts".into(),
        collection_type: CollectionType::Base,
        schema: vec![Field {
            id: "f1".into(),
            name: "owner".into(),
            field_type: FieldType::Text,
            required: false,
            unique: false,
            options: FieldOptions::default(),
        }],
        list_rule: Some("owner = @request.auth.id".into()),
        view_rule: Some("owner = @request.auth.id".into()),
        create_rule: Some(String::new()),
        update_rule: Some(String::new()),
        delete_rule: Some(String::new()),
        auth_options: Default::default(),
        view_query: None,
        created: now(),
        updated: now(),
    };

    let hub = RealtimeHub::default();
    let owner_auth = AuthContext {
        id: "user-1".into(),
        collection_id: "users".into(),
        is_superuser: false,
        record: Default::default(),
    };
    let other_auth = AuthContext {
        id: "user-2".into(),
        collection_id: "users".into(),
        is_superuser: false,
        record: Default::default(),
    };

    let (owner_id, mut owner_rx) = hub.connect(Some(owner_auth)).await;
    let (other_id, mut other_rx) = hub.connect(Some(other_auth)).await;
    hub.subscribe(&owner_id, vec!["posts".into()], None).await;
    hub.subscribe(&other_id, vec!["posts".into()], None).await;

    let record = json!({"id": "r1", "owner": "user-1", "created": "x", "updated": "x"});
    hub.publish(&state.db, &collection, "create", &record).await;

    assert!(
        owner_rx.try_recv().is_ok(),
        "owner's listRule admits this record, they should receive the event"
    );
    assert!(
        other_rx.try_recv().is_err(),
        "other subscriber's listRule denies this record — they must NOT receive it"
    );
}

#[tokio::test]
async fn login_endpoint_rate_limits_after_burst() {
    let state = test_state_with_rate_limit(true).await;
    let app = build_app(state, &cratebase_server::plugins::registry());

    fn login_request(forwarded_for: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/admins/auth-with-password")
            .header("content-type", "application/json")
            .header("x-forwarded-for", forwarded_for)
            .body(Body::from(
                json!({"email": "nobody@test.local", "password": "wrong"}).to_string(),
            ))
            .unwrap()
    }

    // Burst size is 8 (see routes/auth.rs): all 8 should reach the
    // handler (401, wrong credentials) rather than being rate-limited.
    for i in 0..8 {
        let res = app.clone().oneshot(login_request("203.0.113.9")).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "request {i} within burst should reach the handler, not be rate-limited"
        );
    }
    // The 9th immediate request from the same IP exceeds the burst.
    let res = app.clone().oneshot(login_request("203.0.113.9")).await.unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);

    // A different client IP has its own, unaffected bucket.
    let res = app.oneshot(login_request("203.0.113.42")).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "a different IP must not share the throttled IP's bucket"
    );
}

#[tokio::test]
async fn email_verification_flow_gates_login_until_confirmed() {
    use cratebase_auth::{issue_action_token, TokenKind};

    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "PATCH",
            "/api/collections/users",
            Some(&token),
            json!({
                "name": "users", "type": "auth",
                "schema": [],
                "authOptions": {"requireEmailVerification": true},
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": null, "deleteRule": null
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
            json!({"email": "bob@example.com", "password": "secret123", "passwordConfirm": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(register.status(), StatusCode::OK);
    let register_body = json_body(register).await;
    let user_id = register_body["id"].as_str().unwrap().to_string();
    let collection_id = register_body["collectionId"].as_str().unwrap().to_string();
    assert_eq!(register_body["verified"], false, "new accounts start unverified");

    let blocked_login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-password",
            None,
            json!({"identity": "bob@example.com", "password": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(
        blocked_login.status(),
        StatusCode::FORBIDDEN,
        "login must be blocked until the email is verified"
    );

    let request = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/request-verification",
            None,
            json!({"email": "bob@example.com"}),
        ))
        .await
        .unwrap();
    assert_eq!(request.status(), StatusCode::NO_CONTENT);

    // Simulates the token a real user would receive by email — issued
    // with the same parameters `request-verification` uses internally.
    let verify_token = issue_action_token(&user_id, TokenKind::VerifyEmail, &collection_id, None, "test-secret", 3600).unwrap();
    let confirm = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/confirm-verification",
            None,
            json!({"token": verify_token}),
        ))
        .await
        .unwrap();
    assert_eq!(confirm.status(), StatusCode::NO_CONTENT);

    let login = app
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-password",
            None,
            json!({"identity": "bob@example.com", "password": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK, "login must succeed once verified");
}

#[tokio::test]
async fn password_reset_flow_replaces_password() {
    use cratebase_auth::{issue_action_token, TokenKind};

    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());

    let register = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/records",
            None,
            json!({"email": "carol@example.com", "password": "oldpassword1", "passwordConfirm": "oldpassword1"}),
        ))
        .await
        .unwrap();
    assert_eq!(register.status(), StatusCode::OK);
    let register_body = json_body(register).await;
    let user_id = register_body["id"].as_str().unwrap().to_string();
    let collection_id = register_body["collectionId"].as_str().unwrap().to_string();

    let request = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/request-password-reset",
            None,
            json!({"email": "carol@example.com"}),
        ))
        .await
        .unwrap();
    assert_eq!(request.status(), StatusCode::NO_CONTENT);

    let reset_token = issue_action_token(&user_id, TokenKind::ResetPassword, &collection_id, None, "test-secret", 3600).unwrap();
    let confirm = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/confirm-password-reset",
            None,
            json!({"token": reset_token, "password": "newpassword1", "passwordConfirm": "newpassword1"}),
        ))
        .await
        .unwrap();
    assert_eq!(confirm.status(), StatusCode::NO_CONTENT);

    let old_login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-password",
            None,
            json!({"identity": "carol@example.com", "password": "oldpassword1"}),
        ))
        .await
        .unwrap();
    assert_eq!(old_login.status(), StatusCode::UNAUTHORIZED, "old password must stop working");

    let new_login = app
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-password",
            None,
            json!({"identity": "carol@example.com", "password": "newpassword1"}),
        ))
        .await
        .unwrap();
    assert_eq!(new_login.status(), StatusCode::OK, "new password must work");
}

#[tokio::test]
async fn email_change_flow_updates_identity_after_confirmation() {
    use cratebase_auth::{issue_action_token, TokenKind};

    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());

    let register = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/records",
            None,
            json!({"email": "dave@example.com", "password": "secret123", "passwordConfirm": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(register.status(), StatusCode::OK);

    let login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-password",
            None,
            json!({"identity": "dave@example.com", "password": "secret123"}),
        ))
        .await
        .unwrap();
    let login_body = json_body(login).await;
    let user_token = login_body["token"].as_str().unwrap().to_string();
    let user_id = login_body["record"]["id"].as_str().unwrap().to_string();
    let collection_id = login_body["record"]["collectionId"].as_str().unwrap().to_string();

    let request = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/request-email-change",
            Some(&user_token),
            json!({"newEmail": "dave-new@example.com"}),
        ))
        .await
        .unwrap();
    assert_eq!(request.status(), StatusCode::NO_CONTENT);

    let change_token = issue_action_token(
        &user_id,
        TokenKind::ChangeEmail,
        &collection_id,
        Some("dave-new@example.com"),
        "test-secret",
        3600,
    )
    .unwrap();
    let confirm = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/confirm-email-change",
            None,
            json!({"token": change_token}),
        ))
        .await
        .unwrap();
    assert_eq!(confirm.status(), StatusCode::NO_CONTENT);

    let old_login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-password",
            None,
            json!({"identity": "dave@example.com", "password": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(old_login.status(), StatusCode::UNAUTHORIZED, "old email must stop working");

    let new_login = app
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-password",
            None,
            json!({"identity": "dave-new@example.com", "password": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(new_login.status(), StatusCode::OK, "new email must work");
}

#[tokio::test]
async fn oauth2_auth_methods_lists_configured_providers_with_redirect_baked_in() {
    use cratebase_server::oauth2::ProviderConfig;

    let dir = std::env::temp_dir().join(format!(
        "cratebase-api-test-oauth2-{}",
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
        auth_rate_limit_enabled: false,
        mailer: MailerConfig::Log,
        mail_from_address: "no-reply@test.local".to_string(),
        mail_from_name: "Cratebase Test".to_string(),
        public_app_url: "http://localhost:8090".to_string(),
        verification_token_ttl_seconds: 86_400,
        password_reset_token_ttl_seconds: 3_600,
        email_change_token_ttl_seconds: 3_600,
        file_token_ttl_seconds: 120,
        otp_token_ttl_seconds: 300,
        oauth_providers: vec![ProviderConfig {
            name: "google",
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
            token_url: "https://oauth2.googleapis.com/token",
            userinfo_url: "https://www.googleapis.com/oauth2/v3/userinfo",
            scope: "openid email profile",
        }],
    };
    let state = build_state(config).await.unwrap();
    let app = build_app(state, &cratebase_server::plugins::registry());

    let res = app
        .oneshot(get_request(
            "/api/collections/users/auth-methods?redirectUri=myapp%3A%2F%2Fcallback",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_body(res).await;
    assert_eq!(body["password"], true);
    assert_eq!(body["oauth2"]["enabled"], true);
    let providers = body["oauth2"]["providers"].as_array().unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0]["name"], "google");
    let auth_url = providers[0]["authUrl"].as_str().unwrap();
    assert!(auth_url.contains("client_id=test-client-id"));
    assert!(auth_url.contains("redirect_uri=myapp%3A%2F%2Fcallback"));
}

#[tokio::test]
async fn oauth2_login_rejects_unconfigured_provider() {
    let state = test_state().await;
    let app = build_app(state, &cratebase_server::plugins::registry());

    let res = app
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-oauth2",
            None,
            json!({"provider": "google", "code": "fake-code", "redirectUri": "https://example.com/callback"}),
        ))
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::BAD_REQUEST,
        "no provider configured in test harness, must be rejected before any network call"
    );
}

#[tokio::test]
async fn file_thumbnail_generates_resized_image_and_caches_derivative() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "photos2", "type": "base",
                "schema": [{"id": "f1", "name": "image", "type": "file"}],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();

    // A solid-color 64x64 PNG built in-process rather than checked-in test
    // fixture bytes.
    let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        64,
        64,
        image::Rgb([220, 20, 60]),
    ));
    let mut png_bytes = Vec::new();
    source
        .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .unwrap();

    let boundary = "----cratebase-thumb-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\ncontent-disposition: form-data; name=\"image\"; filename=\"pic.png\"\r\ncontent-type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&png_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let req = Request::builder()
        .method("POST")
        .uri("/api/collections/photos2/records")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let created = app.clone().oneshot(req).await.unwrap();
    assert_eq!(created.status(), StatusCode::OK, "{:?}", json_body(created).await);
    let record = json_body(created).await;
    let filename = record["image"].as_str().unwrap().to_string();
    let id = record["id"].as_str().unwrap().to_string();

    let thumb_res = app
        .clone()
        .oneshot(get_request(
            &format!("/api/files/photos2/{id}/{filename}?thumb=16x16"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(thumb_res.status(), StatusCode::OK);
    assert_eq!(
        thumb_res.headers().get("content-type").unwrap(),
        "image/png"
    );
    let thumb_bytes = axum::body::to_bytes(thumb_res.into_body(), usize::MAX)
        .await
        .unwrap();
    let decoded = image::load_from_memory(&thumb_bytes).unwrap();
    assert_eq!(decoded.width(), 16);
    assert_eq!(decoded.height(), 16);
    assert!(
        thumb_bytes.len() < png_bytes.len(),
        "a 16x16 thumbnail must be smaller than the 64x64 original"
    );

    // The derivative must now be cached in the storage backend alongside
    // the original, so a second identical request is a plain storage read
    // rather than another decode/resize/encode round trip.
    let collection = cratebase_db::collections::get_collection_by_name(&state.db, "photos2")
        .await
        .unwrap();
    let cache_key = format!("{}/{}/thumbs/{}_16x16", collection.id, id, filename);
    assert!(
        state.storage.exists(&cache_key).await.unwrap(),
        "resized thumbnail must be cached to storage under a derived key"
    );

    let second = app
        .oneshot(get_request(
            &format!("/api/files/photos2/{id}/{filename}?thumb=16x16"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let second_bytes = axum::body::to_bytes(second.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        &second_bytes[..],
        &thumb_bytes[..],
        "cache hit must return byte-identical cached derivative"
    );
}

#[tokio::test]
async fn file_thumbnail_falls_back_to_original_for_non_image_mime() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "docs2", "type": "base",
                "schema": [{"id": "f1", "name": "attachment", "type": "file"}],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();

    let boundary = "----cratebase-thumb-fallback-boundary";
    let body = format!(
        "--{b}\r\ncontent-disposition: form-data; name=\"attachment\"; filename=\"note.txt\"\r\ncontent-type: text/plain\r\n\r\nhello file\r\n--{b}--\r\n",
        b = boundary
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/collections/docs2/records")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let created = app.clone().oneshot(req).await.unwrap();
    let record = json_body(created).await;
    let filename = record["attachment"].as_str().unwrap().to_string();
    let id = record["id"].as_str().unwrap().to_string();

    let res = app
        .oneshot(get_request(
            &format!("/api/files/docs2/{id}/{filename}?thumb=16x16"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], b"hello file", "non-raster mime must ignore ?thumb= and serve the original");
}

#[tokio::test]
async fn protected_file_requires_auth_and_accepts_file_token() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let admin = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&admin),
            json!({
                "name": "secure_docs", "type": "base",
                "schema": [{"id": "f1", "name": "attachment", "type": "file"}],
                "listRule": "@request.auth.id != \"\"", "viewRule": "@request.auth.id != \"\"",
                "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&admin),
            json!({
                "name": "sd_users", "type": "auth",
                "schema": [],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();

    let boundary = "----cratebase-protected-boundary";
    let body = format!(
        "--{b}\r\ncontent-disposition: form-data; name=\"attachment\"; filename=\"secret.txt\"\r\ncontent-type: text/plain\r\n\r\ntop secret\r\n--{b}--\r\n",
        b = boundary
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/collections/secure_docs/records")
        .header("authorization", format!("Bearer {admin}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let created = app.clone().oneshot(req).await.unwrap();
    assert_eq!(created.status(), StatusCode::OK, "{:?}", json_body(created).await);
    let record = json_body(created).await;
    let filename = record["attachment"].as_str().unwrap().to_string();
    let id = record["id"].as_str().unwrap().to_string();

    // No auth at all: a non-empty viewRule that fails to match compiles
    // to a `WHERE` filter with no matching rows — same anti-enumeration
    // behavior as every other record read (see `records.rs`'s `forbidden()`
    // vs plain not-found), so this surfaces as 404, not 403.
    let denied = app
        .clone()
        .oneshot(get_request(
            &format!("/api/files/secure_docs/{id}/{filename}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::NOT_FOUND);

    // Register and log in an ordinary (non-superuser) auth record, since a
    // file token should work for any authenticated caller, not just admins.
    let signup = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/sd_users/records",
            None,
            json!({"email": "viewer@test.local", "password": "secret123", "passwordConfirm": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(signup.status(), StatusCode::OK, "{:?}", json_body(signup).await);

    let login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/sd_users/auth-with-password",
            None,
            json!({"identity": "viewer@test.local", "password": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let user_token = json_body(login).await["token"].as_str().unwrap().to_string();

    // Mint a short-lived file token for the logged-in viewer.
    let mint = app
        .clone()
        .oneshot(Request::builder()
            .method("POST")
            .uri("/api/files/token")
            .header("authorization", format!("Bearer {user_token}"))
            .body(Body::empty())
            .unwrap())
        .await
        .unwrap();
    assert_eq!(mint.status(), StatusCode::OK, "{:?}", json_body(mint).await);
    let file_token = json_body(mint).await["token"].as_str().unwrap().to_string();

    // No Authorization header, only `?token=`: the viewRule now passes.
    let via_token = app
        .oneshot(get_request(
            &format!("/api/files/secure_docs/{id}/{filename}?token={file_token}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(via_token.status(), StatusCode::OK, "{:?}", json_body(via_token).await);
    let bytes = axum::body::to_bytes(via_token.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"top secret");
}

#[tokio::test]
async fn batch_create_commits_every_record_in_order() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "notes",
                "type": "base",
                "schema": [{"id": "f1", "name": "title", "type": "text", "required": true}],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();

    let batch = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/batch",
            None,
            json!({
                "requests": [
                    {"method": "POST", "url": "/api/collections/notes/records", "body": {"title": "one"}},
                    {"method": "POST", "url": "/api/collections/notes/records", "body": {"title": "two"}},
                    {"method": "POST", "url": "/api/collections/notes/records", "body": {"title": "three"}}
                ]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(batch.status(), StatusCode::OK);
    let body = json_body(batch).await;
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0]["status"], 200);
    assert_eq!(results[0]["body"]["title"], "one");
    assert_eq!(results[1]["body"]["title"], "two");
    assert_eq!(results[2]["body"]["title"], "three");

    let listed = app
        .oneshot(get_request("/api/collections/notes/records", None))
        .await
        .unwrap();
    assert_eq!(json_body(listed).await["totalItems"], 3);
}

#[tokio::test]
async fn batch_rolls_back_every_write_when_one_sub_request_fails() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "tasks",
                "type": "base",
                "schema": [{"id": "f1", "name": "title", "type": "text", "required": true}],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": ""
            }),
        ))
        .await
        .unwrap();

    let batch = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/batch",
            None,
            json!({
                "requests": [
                    {"method": "POST", "url": "/api/collections/tasks/records", "body": {"title": "first"}},
                    // Violates the required `title` field on the second sub-request.
                    {"method": "POST", "url": "/api/collections/tasks/records", "body": {}},
                    {"method": "POST", "url": "/api/collections/tasks/records", "body": {"title": "third"}}
                ]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(batch.status(), StatusCode::BAD_REQUEST);
    let body = json_body(batch).await;
    assert_eq!(body["failedIndex"], 1);
    assert_eq!(body["error"]["data"]["title"]["code"], "value_required");

    let listed = app
        .oneshot(get_request("/api/collections/tasks/records", None))
        .await
        .unwrap();
    assert_eq!(
        json_body(listed).await["totalItems"],
        0,
        "the first and third creates must be rolled back along with the failing second one"
    );
}

#[tokio::test]
async fn otp_login_succeeds_with_correct_code() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());

    let register = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/records",
            None,
            json!({"email": "otp-user@example.com", "password": "secret123", "passwordConfirm": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(register.status(), StatusCode::OK);
    let register_body = json_body(register).await;
    let user_id = register_body["id"].as_str().unwrap().to_string();
    let collection_id = register_body["collectionId"].as_str().unwrap().to_string();

    let requested = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/request-otp",
            None,
            json!({"email": "otp-user@example.com"}),
        ))
        .await
        .unwrap();
    assert_eq!(requested.status(), StatusCode::NO_CONTENT);

    let code = find_otp_code(&state, &collection_id, &user_id).await;

    let login = app
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-otp",
            None,
            json!({"email": "otp-user@example.com", "otp": code}),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let login_body = json_body(login).await;
    assert!(login_body["token"].as_str().is_some_and(|t| !t.is_empty()));
    assert_eq!(login_body["record"]["id"], user_id);
}

#[tokio::test]
async fn otp_login_fails_with_incorrect_code() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());

    let register = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/records",
            None,
            json!({"email": "otp-wrong@example.com", "password": "secret123", "passwordConfirm": "secret123"}),
        ))
        .await
        .unwrap();
    let register_body = json_body(register).await;
    let user_id = register_body["id"].as_str().unwrap().to_string();
    let collection_id = register_body["collectionId"].as_str().unwrap().to_string();

    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/request-otp",
            None,
            json!({"email": "otp-wrong@example.com"}),
        ))
        .await
        .unwrap();

    let real_code = find_otp_code(&state, &collection_id, &user_id).await;
    let wrong_code = if real_code == "000000" { "111111" } else { "000000" };

    let login = app
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-otp",
            None,
            json!({"email": "otp-wrong@example.com", "otp": wrong_code}),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn otp_login_fails_with_expired_code() {
    let state = test_state_with_otp_ttl(1).await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());

    let register = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/records",
            None,
            json!({"email": "otp-expired@example.com", "password": "secret123", "passwordConfirm": "secret123"}),
        ))
        .await
        .unwrap();
    let register_body = json_body(register).await;
    let user_id = register_body["id"].as_str().unwrap().to_string();
    let collection_id = register_body["collectionId"].as_str().unwrap().to_string();

    app.clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/request-otp",
            None,
            json!({"email": "otp-expired@example.com"}),
        ))
        .await
        .unwrap();

    let code = find_otp_code(&state, &collection_id, &user_id).await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let login = app
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-otp",
            None,
            json!({"email": "otp-expired@example.com", "otp": code}),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mfa_required_blocks_password_login_until_otp_confirmed() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(json_request(
            "PATCH",
            "/api/collections/users",
            Some(&token),
            json!({
                "name": "users", "type": "auth",
                "schema": [],
                "authOptions": {"mfaRequired": true},
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": null, "deleteRule": null
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
            json!({"email": "mfa-user@example.com", "password": "secret123", "passwordConfirm": "secret123"}),
        ))
        .await
        .unwrap();
    let register_body = json_body(register).await;
    let user_id = register_body["id"].as_str().unwrap().to_string();
    let collection_id = register_body["collectionId"].as_str().unwrap().to_string();

    let login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections/users/auth-with-password",
            None,
            json!({"identity": "mfa-user@example.com", "password": "secret123"}),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let login_body = json_body(login).await;
    assert!(
        login_body["token"].is_null(),
        "a correct password login on an mfaRequired collection must not return a usable session token directly"
    );
    let mfa_id = login_body["mfaId"].as_str().unwrap().to_string();
    assert_eq!(login_body["mfaRequired"], true);

    let code = find_otp_code(&state, &collection_id, &user_id).await;

    let confirm = app
        .oneshot(json_request(
            "POST",
            "/api/collections/users/mfa/confirm",
            None,
            json!({"mfaId": mfa_id, "otp": code}),
        ))
        .await
        .unwrap();
    assert_eq!(confirm.status(), StatusCode::OK);
    let confirm_body = json_body(confirm).await;
    assert!(confirm_body["token"].as_str().is_some_and(|t| !t.is_empty()));
    assert_eq!(confirm_body["record"]["id"], user_id);
}

#[tokio::test]
async fn request_log_middleware_captures_requests() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    app.clone()
        .oneshot(get_request("/api/health", None))
        .await
        .unwrap();

    // The middleware persists on a spawned task rather than inline (so
    // logging never adds latency to the response it's describing) — give
    // it a moment to land before asserting on it.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let logs = app
        .oneshot(get_request("/api/logs", Some(&token)))
        .await
        .unwrap();
    assert_eq!(logs.status(), StatusCode::OK);
    let body = json_body(logs).await;
    let items = body["items"].as_array().unwrap();
    assert!(
        items.iter().any(|e| e["path"] == "/api/health" && e["status"] == 200),
        "expected the /api/health request to show up in the request log: {items:?}"
    );
}

#[tokio::test]
async fn backups_round_trip_on_sqlite() {
    // `VACUUM INTO` needs a real on-disk database to copy from — `:memory:`
    // (what `test_state()` normally uses) has no backing file for SQLite
    // to snapshot from in the first place, so this test spins up its own
    // file-backed instance instead.
    let dir = std::env::temp_dir().join(format!(
        "cratebase-api-test-backup-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("cratebase.db");

    let config = Config {
        database_url: format!("sqlite://{}?mode=rwc", db_path.to_string_lossy()),
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
        auth_rate_limit_enabled: false,
        mailer: MailerConfig::Log,
        mail_from_address: "no-reply@test.local".to_string(),
        mail_from_name: "Cratebase Test".to_string(),
        public_app_url: "http://localhost:8090".to_string(),
        verification_token_ttl_seconds: 86_400,
        password_reset_token_ttl_seconds: 3_600,
        email_change_token_ttl_seconds: 3_600,
        file_token_ttl_seconds: 5,
        otp_token_ttl_seconds: 300,
        oauth_providers: Vec::new(),
    };
    let state = build_state(config).await.unwrap();
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    let create = app
        .clone()
        .oneshot(json_request("POST", "/api/backups", Some(&token), json!({})))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let create_body = json_body(create).await;
    let name = create_body["name"].as_str().unwrap().to_string();
    assert!(create_body["size"].as_u64().unwrap() > 0, "a real sqlite snapshot must have nonzero size");

    let list = app
        .clone()
        .oneshot(get_request("/api/backups", Some(&token)))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let list_body = json_body(list).await;
    assert!(list_body.as_array().unwrap().iter().any(|b| b["name"] == name));

    let download = app
        .clone()
        .oneshot(get_request(&format!("/api/backups/{name}/download"), Some(&token)))
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(download.into_body(), usize::MAX).await.unwrap();
    assert!(!bytes.is_empty(), "downloaded backup must contain the sqlite snapshot bytes");

    let delete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/backups/{name}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let list_after = app
        .oneshot(get_request("/api/backups", Some(&token)))
        .await
        .unwrap();
    let list_after_body = json_body(list_after).await;
    assert!(
        !list_after_body.as_array().unwrap().iter().any(|b| b["name"] == name),
        "deleted backup must no longer be listed"
    );
}

/// Proves the same server rejects a backup attempt outright on Postgres
/// rather than silently doing nothing or corrupting a shared cluster.
/// Skips (rather than fails) when `TEST_POSTGRES_URL` isn't set, matching
/// `crates/db/tests/postgres.rs`'s convention.
#[tokio::test]
async fn backups_are_rejected_on_postgres() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!("skipping backups_are_rejected_on_postgres: TEST_POSTGRES_URL not set");
        return;
    };

    let dir = std::env::temp_dir().join(format!(
        "cratebase-api-test-pg-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let config = Config {
        database_url: url,
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
        auth_rate_limit_enabled: false,
        mailer: MailerConfig::Log,
        mail_from_address: "no-reply@test.local".to_string(),
        mail_from_name: "Cratebase Test".to_string(),
        public_app_url: "http://localhost:8090".to_string(),
        verification_token_ttl_seconds: 86_400,
        password_reset_token_ttl_seconds: 3_600,
        email_change_token_ttl_seconds: 3_600,
        file_token_ttl_seconds: 5,
        otp_token_ttl_seconds: 300,
        oauth_providers: Vec::new(),
    };
    let state = build_state(config).await.unwrap();
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());

    // Doesn't reuse `admin_token`'s fixed `admin@test.local` address: a
    // shared Postgres instance can carry admin rows across test runs
    // (there's no per-test schema reset here, unlike
    // `crates/db/tests/postgres.rs`), and a unique id-suffixed email
    // sidesteps that instead of requiring one.
    let email = format!("backups-pg-test-{}@test.local", cratebase_core::new_id());
    let hash = cratebase_auth::hash_password("admin12345").unwrap();
    cratebase_db::admins::create_admin(&state.db, &email, &hash).await.unwrap();
    let login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admins/auth-with-password",
            None,
            json!({"email": email, "password": "admin12345"}),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let token = json_body(login).await["token"].as_str().unwrap().to_string();

    let create = app
        .oneshot(json_request("POST", "/api/backups", Some(&token), json!({})))
        .await
        .unwrap();
    assert_eq!(
        create.status(),
        StatusCode::BAD_REQUEST,
        "backups must be refused on a Postgres-backed server"
    );
}

#[tokio::test]
async fn collection_save_rejects_unparseable_and_unknown_field_rules() {
    let state = test_state().await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());
    let token = admin_token(&state, &app).await;

    // Syntax error: dangling operator.
    let bad_syntax = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "rule_syntax_error",
                "type": "base",
                "schema": [{"id": "f1", "name": "title", "type": "text"}],
                "listRule": "title =",
                "viewRule": "", "createRule": "", "updateRule": null, "deleteRule": null
            }),
        ))
        .await
        .unwrap();
    assert_eq!(bad_syntax.status(), StatusCode::BAD_REQUEST);
    let body = json_body(bad_syntax).await;
    assert!(
        body["message"].as_str().unwrap().contains("listRule"),
        "error should name which rule failed: {body:?}"
    );

    // Unknown field: typo'd column name.
    let bad_field = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/collections",
            Some(&token),
            json!({
                "name": "rule_unknown_field",
                "type": "base",
                "schema": [{"id": "f1", "name": "title", "type": "text"}],
                "listRule": "", "viewRule": "", "createRule": "", "updateRule": "titel = \"x\"", "deleteRule": null
            }),
        ))
        .await
        .unwrap();
    assert_eq!(bad_field.status(), StatusCode::BAD_REQUEST);
    let body = json_body(bad_field).await;
    assert!(
        body["message"].as_str().unwrap().contains("updateRule"),
        "error should name which rule failed: {body:?}"
    );

    // Neither invalid collection was persisted.
    let list = app
        .oneshot(get_request("/api/collections", Some(&token)))
        .await
        .unwrap();
    let names: Vec<String> = json_body(list)
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();
    assert!(!names.contains(&"rule_syntax_error".to_string()));
    assert!(!names.contains(&"rule_unknown_field".to_string()));
}

#[tokio::test]
async fn request_otp_rate_limits_after_burst() {
    let state = test_state_with_rate_limit(true).await;
    let app = build_app(state.clone(), &cratebase_server::plugins::registry());

    fn otp_request(forwarded_for: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/collections/users/request-otp")
            .header("content-type", "application/json")
            .header("x-forwarded-for", forwarded_for)
            .body(Body::from(json!({"email": "nobody@test.local"}).to_string()))
            .unwrap()
    }

    // Same burst=8 governor config as routes::auth::router. request-otp
    // always answers 204 regardless of match (no-enumeration), so every
    // in-burst request should be 204, not 429.
    for i in 0..8 {
        let res = app.clone().oneshot(otp_request("203.0.114.9")).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::NO_CONTENT,
            "request {i} within burst should reach the handler, not be rate-limited"
        );
    }
    let res = app.oneshot(otp_request("203.0.114.9")).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the 9th immediate request from the same IP must exceed the burst — request-otp is a \
         mail-bombing vector and must be throttled the same as request-verification/etc"
    );
}
