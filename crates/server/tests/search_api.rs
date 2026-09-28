//! End-to-end tests for `GET /api/collections/{c}/records?search=` —
//! the HTTP-level wiring on top of `cratebase_db::records::list`'s
//! `ListParams::search` (already covered in depth at the db layer by
//! `crates/db/tests/records.rs`). What's specific to this layer: the
//! query param actually reaches the db call, composes with `?filter=`,
//! is still gated by the list rule, and a collection with no searchable
//! fields fails cleanly (400, not 500) instead of leaking a Rust panic
//! or an opaque SQL error through the API.

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

    async fn send(&self, request: Request<Body>) -> (StatusCode, Value) {
        let response = cratebase_server::router(self.app.clone())
            .oneshot(request)
            .await
            .expect("response");
        split(response).await
    }

    async fn admin(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.as_user(method, uri, body, Some(&self.token)).await
    }

    async fn as_user(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
        token: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header("authorization", token);
        }
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        self.send(request).await
    }

    async fn collection(&self, body: Value) -> String {
        let (status, value) = self.admin("POST", "/api/collections", Some(body)).await;
        assert_eq!(status, 200, "{value}");
        value["id"].as_str().expect("an id").to_string()
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

fn searchable_posts(name: &str, list_rule: &str) -> Value {
    json!({
        "name": name,
        "type": "base",
        "listRule": list_rule, "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": "",
        "fields": [
            {"name": "title", "type": "text", "searchable": true},
            {"name": "body", "type": "editor", "searchable": true},
            {"name": "author", "type": "text"},
        ],
    })
}

#[tokio::test]
async fn search_query_param_matches_and_ranks_by_relevance() {
    let harness = Harness::new().await;
    harness.collection(searchable_posts("posts", "")).await;
    harness
        .admin(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "treasure map", "body": "an old map", "author": "a"})),
        )
        .await;
    harness
        .admin(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "treasure treasure treasure", "body": "gold", "author": "b"})),
        )
        .await;
    harness
        .admin(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "unrelated", "body": "nothing here", "author": "c"})),
        )
        .await;

    let (status, page) = harness
        .as_user("GET", "/api/collections/posts/records?search=treasure", None, None)
        .await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(page["totalItems"], 2);
    // No explicit `sort`: most-relevant-first, so the record whose title
    // repeats "treasure" three times outranks the one that mentions it
    // once.
    assert_eq!(page["items"][0]["author"], "b");
    assert_eq!(page["items"][1]["author"], "a");
}

#[tokio::test]
async fn search_composes_with_filter_and_the_list_rule() {
    let harness = Harness::new().await;
    harness
        .collection(searchable_posts("posts", "author = 'a'"))
        .await;
    harness
        .admin(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "treasure map", "body": "x", "author": "a"})),
        )
        .await;
    harness
        .admin(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "treasure chest", "body": "x", "author": "b"})),
        )
        .await;

    // The list rule restricts to author = 'a' regardless of who's
    // asking (no auth check in the rule itself here, just a data
    // filter) — the search still only ever sees that row.
    let (status, page) = harness
        .as_user("GET", "/api/collections/posts/records?search=treasure", None, None)
        .await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(page["totalItems"], 1);
    assert_eq!(page["items"][0]["author"], "a");

    // AND-ed with an explicit `?filter=` too.
    let (status, page) = harness
        .as_user(
            "GET",
            "/api/collections/posts/records?search=treasure&filter=author='a'",
            None,
            None,
        )
        .await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(page["totalItems"], 1);

    let (status, page) = harness
        .as_user(
            "GET",
            "/api/collections/posts/records?search=treasure&filter=author='nobody'",
            None,
            None,
        )
        .await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(page["totalItems"], 0);
}

#[tokio::test]
async fn explicit_sort_overrides_the_relevance_default() {
    let harness = Harness::new().await;
    harness.collection(searchable_posts("posts", "")).await;
    harness
        .admin(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "treasure map", "body": "x", "author": "a"})),
        )
        .await;
    harness
        .admin(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "treasure treasure", "body": "x", "author": "b"})),
        )
        .await;

    let (status, page) = harness
        .as_user(
            "GET",
            "/api/collections/posts/records?search=treasure&sort=author",
            None,
            None,
        )
        .await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(page["items"][0]["author"], "a");
    assert_eq!(page["items"][1]["author"], "b");
}

#[tokio::test]
async fn search_on_a_collection_with_no_searchable_fields_is_a_clean_400() {
    let harness = Harness::new().await;
    harness
        .collection(json!({
            "name": "plain",
            "type": "base",
            "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": "",
            "fields": [{"name": "title", "type": "text"}],
        }))
        .await;
    let (status, body) = harness
        .as_user("GET", "/api/collections/plain/records?search=anything", None, None)
        .await;
    assert_eq!(status, 400, "{body}");
}
