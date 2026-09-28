//! End-to-end tests for `_notifications`: rule scoping (a recipient sees
//! only their own rows, and may only ever change `readAt` through the
//! generic records API), `POST /api/notifications/send` (superuser/
//! API-key only), `GET /api/notifications/unread-count` and
//! `POST /api/notifications/read-all`, and that the unread-count query is
//! index-backed on both SQLite and Postgres.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_db::Executor;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use serde_json::{json, Value};
use tower::ServiceExt;

const SUPERUSER_EMAIL: &str = "admin@example.com";
const SUPERUSER_PASSWORD: &str = "hunter2hunter2";

struct Harness {
    app: App,
    superuser_token: String,
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
        let superuser_token = app
            .mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token");
        Harness {
            app,
            superuser_token,
            _dir: dir,
        }
    }

    fn router(&self) -> axum::Router {
        cratebase_server::router(self.app.clone())
    }

    async fn send(&self, request: Request<Body>) -> Response {
        self.router().oneshot(request).await.expect("response")
    }

    async fn request(
        &self,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", token);
        }
        let resp = self
            .send(builder.body(Body::from(body.to_string())).unwrap())
            .await;
        split(resp).await
    }

    async fn post(&self, uri: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, token, body).await
    }

    async fn patch(&self, uri: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
        self.request("PATCH", uri, token, body).await
    }

    async fn delete(&self, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
        self.request("DELETE", uri, token, Value::Null).await
    }

    async fn get(&self, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
        let mut builder = Request::get(uri);
        if let Some(token) = token {
            builder = builder.header("authorization", token);
        }
        let resp = self.send(builder.body(Body::empty()).unwrap()).await;
        split(resp).await
    }

    /// A regular (non-superuser) `users` record with a valid auth token.
    /// Returns `(id, token)`.
    async fn create_user(&self, email: &str) -> (String, String) {
        let collection = self.app.db().collections.get("users").expect("users");
        let mut record = cratebase_core::Record::new(collection);
        record.set("email", Value::String(email.to_string()));
        record.set("password", Value::String("supersecret123".into()));
        record.set("verified", Value::Bool(true));
        cratebase_db::records::create(self.app.db(), &self.app.db().collections, &mut record)
            .await
            .expect("create user");
        let id = record.id().to_string();
        let token = self
            .app
            .mint_token("users", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token");
        (id, token)
    }

    /// `$notify.send`/`POST /api/notifications/send`'s underlying pipeline,
    /// called directly rather than through the superuser-gated HTTP route,
    /// so tests can seed notifications without re-proving that gate in
    /// every test. Returns the created `_notifications` row's own id
    /// (`SendOutcome::recipients` is the *recipient* ids, not the
    /// notification row ids, so this looks the row up separately).
    async fn seed_notification(&self, user_id: &str, title: &str) -> String {
        let outcome = cratebase_server::notify::send(
            &self.app,
            cratebase_server::notify::NotifySendInput {
                to: vec![user_id.to_string()],
                collection: Some("users".into()),
                kind: "test".into(),
                title: title.into(),
                body: "body".into(),
                data: Value::Null,
                link: None,
                channels: Some(vec!["inapp".into()]),
            },
        )
        .await
        .expect("seed notification");
        assert_eq!(outcome.sent, 1);
        let row = self
            .app
            .db()
            .query_one(
                r#"SELECT "id" FROM "_notifications" WHERE "recordRef" = $1 AND "title" = $2
                   ORDER BY "created" DESC LIMIT 1"#,
                &[
                    cratebase_db::engine::Sql::from(user_id.to_string()),
                    cratebase_db::engine::Sql::from(title.to_string()),
                ],
            )
            .await
            .expect("query notification id")
            .expect("notification row exists");
        row.get_str("id").expect("id column").to_string()
    }
}

async fn split(resp: Response) -> (StatusCode, Value) {
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let body: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

#[tokio::test]
async fn a_recipient_can_only_list_view_their_own_notifications() {
    let h = Harness::new().await;
    let (alice_id, alice_token) = h.create_user("alice@example.com").await;
    let (_bob_id, bob_token) = h.create_user("bob@example.com").await;
    let note_id = h.seed_notification(&alice_id, "hello alice").await;

    // Alice sees it.
    let (status, body) = h
        .get(
            "/api/collections/_notifications/records",
            Some(&alice_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    let (status, _) = h
        .get(
            &format!("/api/collections/_notifications/records/{note_id}"),
            Some(&alice_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Bob does not.
    let (status, body) = h
        .get("/api/collections/_notifications/records", Some(&bob_token))
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["items"].as_array().unwrap().len(), 0);
    let (status, _) = h
        .get(
            &format!("/api/collections/_notifications/records/{note_id}"),
            Some(&bob_token),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a denied view is a 404, not a 403"
    );

    // Anonymous sees nothing either.
    let (status, _) = h.get("/api/collections/_notifications/records", None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn only_the_recipient_may_mark_their_own_notification_read() {
    let h = Harness::new().await;
    let (alice_id, alice_token) = h.create_user("alice@example.com").await;
    let (_bob_id, bob_token) = h.create_user("bob@example.com").await;
    let note_id = h.seed_notification(&alice_id, "hello alice").await;

    // Bob cannot touch Alice's notification at all.
    let (status, _) = h
        .patch(
            &format!("/api/collections/_notifications/records/{note_id}"),
            Some(&bob_token),
            json!({ "readAt": "2024-01-01 00:00:00.000Z" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = h
        .delete(
            &format!("/api/collections/_notifications/records/{note_id}"),
            Some(&bob_token),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Alice may flip `readAt` on her own row...
    let (status, body) = h
        .patch(
            &format!("/api/collections/_notifications/records/{note_id}"),
            Some(&alice_token),
            json!({ "readAt": "2024-01-01 00:00:00.000Z" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(!body["readAt"].as_str().unwrap_or("").is_empty());

    // ...but nothing else, even though the row itself passes `updateRule`.
    let (status, body) = h
        .patch(
            &format!("/api/collections/_notifications/records/{note_id}"),
            Some(&alice_token),
            json!({ "title": "hacked" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");

    let (status, body) = h
        .patch(
            &format!("/api/collections/_notifications/records/{note_id}"),
            Some(&alice_token),
            json!({ "readAt": "2024-02-02 00:00:00.000Z", "body": "hacked" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "mixing readAt with another field must still be refused: {body:?}"
    );

    // A superuser is exempt from the readAt-only restriction.
    let (status, body) = h
        .patch(
            &format!("/api/collections/_notifications/records/{note_id}"),
            Some(&h.superuser_token),
            json!({ "title": "superuser can edit anything" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    // Alice may delete her own row.
    let (status, _) = h
        .delete(
            &format!("/api/collections/_notifications/records/{note_id}"),
            Some(&alice_token),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn notifications_send_is_superuser_or_api_key_only() {
    let h = Harness::new().await;
    let (alice_id, alice_token) = h.create_user("alice@example.com").await;

    let payload = json!({
        "to": alice_id,
        "type": "info",
        "title": "hi",
        "body": "there",
        "channels": ["inapp"],
    });

    // No token at all is 401 ("who are you"); an authenticated
    // non-superuser is 403 ("I know who you are, but no") — same split
    // `RequireSuperuser` gives every other superuser-only endpoint.
    let (status, _) = h
        .post("/api/notifications/send", None, payload.clone())
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = h
        .post(
            "/api/notifications/send",
            Some(&alice_token),
            payload.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A superuser may send, and it actually creates the row.
    let (status, body) = h
        .post(
            "/api/notifications/send",
            Some(&h.superuser_token),
            payload.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["sent"], 1);

    let (status, body) = h
        .get(
            "/api/collections/_notifications/records",
            Some(&alice_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1, "{body:?}");
}

#[tokio::test]
async fn unread_count_and_read_all_endpoints() {
    let h = Harness::new().await;
    let (alice_id, alice_token) = h.create_user("alice@example.com").await;
    let (_bob_id, bob_token) = h.create_user("bob@example.com").await;
    h.seed_notification(&alice_id, "one").await;
    let second = h.seed_notification(&alice_id, "two").await;

    // Unauthenticated has no recipient identity to count for.
    let (status, _) = h.get("/api/notifications/unread-count", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = h
        .get("/api/notifications/unread-count", Some(&alice_token))
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["count"], 2);

    // Bob's own count is unaffected by Alice's notifications.
    let (status, body) = h
        .get("/api/notifications/unread-count", Some(&bob_token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 0);

    // Mark one read directly, then read-all should only report the rest.
    h.patch(
        &format!("/api/collections/_notifications/records/{second}"),
        Some(&alice_token),
        json!({ "readAt": "2024-01-01 00:00:00.000Z" }),
    )
    .await;
    let (status, body) = h
        .get("/api/notifications/unread-count", Some(&alice_token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 1, "{body:?}");

    let (status, body) = h
        .post(
            "/api/notifications/read-all",
            Some(&alice_token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["updated"], 1);

    let (status, body) = h
        .get("/api/notifications/unread-count", Some(&alice_token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 0, "{body:?}");

    // read-all again is a no-op, not an error.
    let (status, body) = h
        .post(
            "/api/notifications/read-all",
            Some(&alice_token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["updated"], 0);
}

/// `GET /api/notifications/unread-count`'s query must be answered from
/// `idx_notifications_recipient` — a full table scan defeats the whole
/// point of a dedicated unread-count endpoint once `_notifications` has
/// any real volume in it.
#[tokio::test]
async fn unread_count_query_is_index_backed_on_sqlite() {
    let h = Harness::new().await;
    let sql = r#"EXPLAIN QUERY PLAN SELECT COUNT(*) AS "count" FROM "_notifications"
                 WHERE "collectionRef" = ? AND "recordRef" = ? AND ("readAt" = '' OR "readAt" IS NULL)"#;
    let rows = h
        .app
        .db()
        .query(
            sql,
            &[
                cratebase_db::engine::Sql::from("col".to_string()),
                cratebase_db::engine::Sql::from("rec".to_string()),
            ],
        )
        .await
        .expect("explain query plan");
    let plan: String = rows
        .iter()
        .filter_map(|r| r.get_str("detail"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains("idx_notifications_recipient"),
        "expected the composite index to be used, got:\n{plan}"
    );
    assert!(
        !plan.to_uppercase().contains("SCAN TABLE _NOTIFICATIONS")
            || plan.contains("idx_notifications_recipient"),
        "unexpected full table scan:\n{plan}"
    );
}

/// Same query, same assertion, against Postgres — skipped when
/// `TEST_POSTGRES_URL` is unset, matching every other Postgres-only test
/// in this workspace.
#[tokio::test]
async fn unread_count_query_is_index_backed_on_postgres() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping unread_count_query_is_index_backed_on_postgres: TEST_POSTGRES_URL not set"
        );
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    {
        let wipe = cratebase_db::Db::connect(&url, &dir.path().to_string_lossy())
            .await
            .expect("connect to wipe schema");
        wipe.execute("DROP SCHEMA public CASCADE", &[])
            .await
            .unwrap();
        wipe.execute("CREATE SCHEMA public", &[]).await.unwrap();
        wipe.close().await.unwrap();
    }

    let app = App::new(Config {
        database_url: url,
        log_requests: false,
        secret: "test-secret-0123456789".into(),
        ..Config::for_data_dir(dir.path().join("pb_data"))
    });
    app.bootstrap().await.expect("bootstrap");

    // An empty table is always cheapest to seq-scan, so give the planner
    // realistic volume (many recipients, mostly read) and fresh stats.
    app.db()
        .execute(
            r#"INSERT INTO "_notifications"
                 ("id", "collectionRef", "recordRef", "type", "title", "body", "readAt", "created", "updated")
               SELECT 'n' || g, '_pb_users_auth_', 'user' || (g % 500), 't', 't', 'b',
                      CASE WHEN g % 10 = 0 THEN '' ELSE '2024-01-01 00:00:00.000Z' END,
                      '2024-01-01 00:00:00.000Z', '2024-01-01 00:00:00.000Z'
               FROM generate_series(1, 20000) AS g"#,
            &[],
        )
        .await
        .expect("seed rows");
    app.db()
        .execute(r#"ANALYZE "_notifications""#, &[])
        .await
        .expect("analyze");

    let sql = r#"EXPLAIN SELECT COUNT(*) AS "count" FROM "_notifications"
                 WHERE "collectionRef" = $1 AND "recordRef" = $2 AND ("readAt" = '' OR "readAt" IS NULL)"#;
    let rows = app
        .db()
        .query(
            sql,
            &[
                cratebase_db::engine::Sql::from("col".to_string()),
                cratebase_db::engine::Sql::from("rec".to_string()),
            ],
        )
        .await
        .expect("explain");
    let plan: String = rows
        .iter()
        .filter_map(|r| r.get_str("QUERY PLAN"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains("idx_notifications_recipient"),
        "expected the planner to use the composite index, got:\n{plan}"
    );
    assert!(
        !plan.contains("Seq Scan"),
        "expected an index scan, not a sequential scan, got:\n{plan}"
    );
}
