//! End-to-end tests for the `vector` field type: round-trip storage,
//! the dimension-mismatch validation error, `EchoProvider`-driven
//! auto-embedding on create, and `?nearestTo=` similarity ranking on the
//! records list endpoint.
//!
//! Driven through `tower::ServiceExt::oneshot` against an in-memory
//! database, same harness shape as `tests/records_api.rs`.

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

// ------------------------------------------------------- storage/validation

#[tokio::test]
async fn vector_field_round_trips_and_rejects_dimension_mismatch() {
    let harness = Harness::new().await;
    harness
        .collection(json!({
            "name": "points",
            "type": "base",
            "listRule": "",
            "viewRule": "",
            "createRule": "",
            "updateRule": "",
            "deleteRule": "",
            "fields": [
                {"name": "label", "type": "text"},
                {"name": "embedding", "type": "vector", "dimensions": 3},
            ],
        }))
        .await;

    // Create with a caller-supplied array — stored and returned verbatim.
    let (status, created) = harness
        .admin(
            "POST",
            "/api/collections/points/records",
            Some(json!({"label": "a", "embedding": [1.0, 2.0, 3.0]})),
        )
        .await;
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["embedding"], json!([1.0, 2.0, 3.0]));
    let id = created["id"].as_str().unwrap().to_string();

    // Read-back round-trips the same array.
    let (status, fetched) = harness
        .admin(
            "GET",
            &format!("/api/collections/points/records/{id}"),
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(fetched["embedding"], json!([1.0, 2.0, 3.0]));

    // Update with a new array of the correct dimension.
    let (status, updated) = harness
        .admin(
            "PATCH",
            &format!("/api/collections/points/records/{id}"),
            Some(json!({"embedding": [4.0, 5.0, 6.0]})),
        )
        .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["embedding"], json!([4.0, 5.0, 6.0]));

    // A mismatched-dimension array is a real validation error, not a
    // silent truncation/pad.
    let (status, body) = harness
        .admin(
            "POST",
            "/api/collections/points/records",
            Some(json!({"label": "bad", "embedding": [1.0, 2.0]})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["message"], "Failed to create record.");
    assert_eq!(
        body["data"]["embedding"]["code"],
        "validation_vector_dimension_mismatch"
    );

    // Same check applies on update.
    let (status, body) = harness
        .admin(
            "PATCH",
            &format!("/api/collections/points/records/{id}"),
            Some(json!({"embedding": [1.0, 2.0, 3.0, 4.0]})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["data"]["embedding"]["code"],
        "validation_vector_dimension_mismatch"
    );
}

// --------------------------------------------------------- auto-embedding

fn chunks_collection() -> Value {
    json!({
        "name": "chunks",
        "type": "base",
        "listRule": "",
        "viewRule": "",
        "createRule": "",
        "updateRule": "",
        "deleteRule": "",
        "fields": [
            {"name": "body", "type": "text"},
            {
                "name": "embedding",
                "type": "vector",
                "dimensions": 8,
                "embedding": {
                    "provider": "echo",
                    "model": "",
                    "sourceField": "body",
                },
            },
        ],
    })
}

#[tokio::test]
async fn echo_provider_auto_embeds_from_the_source_field_on_create() {
    let harness = Harness::new().await;
    harness.collection(chunks_collection()).await;

    // No `embedding` key in the body — the server computes it from `body`.
    let (status, created) = harness
        .admin(
            "POST",
            "/api/collections/chunks/records",
            Some(json!({"body": "the quick brown fox"})),
        )
        .await;
    assert_eq!(status, 200, "{created}");
    let vector = created["embedding"]
        .as_array()
        .expect("embedding was auto-populated as an array")
        .clone();
    assert_eq!(vector.len(), 8);
    assert!(
        vector.iter().any(|v| v.as_f64().unwrap() != 0.0),
        "echo embedding should not be all zeros: {vector:?}"
    );

    // Same source text embeds identically (EchoProvider is deterministic).
    let (_, created2) = harness
        .admin(
            "POST",
            "/api/collections/chunks/records",
            Some(json!({"body": "the quick brown fox"})),
        )
        .await;
    assert_eq!(created2["embedding"], Value::Array(vector.clone()));

    // Different source text embeds differently.
    let (_, created3) = harness
        .admin(
            "POST",
            "/api/collections/chunks/records",
            Some(json!({"body": "completely different text"})),
        )
        .await;
    assert_ne!(created3["embedding"], Value::Array(vector.clone()));

    // A caller-supplied vector is never overridden by auto-embedding.
    let explicit: Vec<f64> = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
    let (_, created4) = harness
        .admin(
            "POST",
            "/api/collections/chunks/records",
            Some(json!({"body": "ignored for embedding", "embedding": explicit})),
        )
        .await;
    assert_eq!(created4["embedding"], json!(explicit));

    // Updating a record's `body` re-embeds it.
    let id = created["id"].as_str().unwrap().to_string();
    let (status, updated) = harness
        .admin(
            "PATCH",
            &format!("/api/collections/chunks/records/{id}"),
            Some(json!({"body": "completely different text"})),
        )
        .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["embedding"], created3["embedding"]);

    // Updating an unrelated field leaves a previously computed vector
    // alone rather than recomputing it against stale text.
    let (_, untouched) = harness
        .admin(
            "PATCH",
            &format!("/api/collections/chunks/records/{id}"),
            Some(json!({})),
        )
        .await;
    assert_eq!(untouched["embedding"], updated["embedding"]);
}

// ------------------------------------------------------------- nearestTo

#[tokio::test]
async fn nearest_to_orders_by_descending_cosine_similarity() {
    let harness = Harness::new().await;
    harness
        .collection(json!({
            "name": "points",
            "type": "base",
            "listRule": "",
            "viewRule": "",
            "createRule": "",
            "updateRule": "",
            "deleteRule": "",
            "fields": [
                {"name": "label", "type": "text"},
                {"name": "embedding", "type": "vector", "dimensions": 2},
            ],
        }))
        .await;

    // Cosine similarity to the query vector [1, 0]:
    //   close    ~0.995
    //   medium   ~0.707
    //   far       0.0
    //   opposite -1.0
    let fixtures = [
        ("far", [0.0, 1.0]),
        ("opposite", [-1.0, 0.0]),
        ("close", [1.0, 0.1]),
        ("medium", [1.0, 1.0]),
    ];
    let mut ids = std::collections::HashMap::new();
    for (label, vector) in fixtures {
        let (status, created) = harness
            .admin(
                "POST",
                "/api/collections/points/records",
                Some(json!({"label": label, "embedding": vector})),
            )
            .await;
        assert_eq!(status, 200, "{created}");
        ids.insert(label, created["id"].as_str().unwrap().to_string());
    }

    let (status, page) = harness
        .admin(
            "GET",
            "/api/collections/points/records?nearestTo=embedding:1,0&nearestLimit=10",
            None,
        )
        .await;
    assert_eq!(status, 200, "{page}");
    let items = page["items"].as_array().expect("items array");
    assert_eq!(items.len(), 4);
    let order: Vec<&str> = items.iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert_eq!(order, vec!["close", "medium", "far", "opposite"]);

    // `nearestLimit` truncates.
    let (_, limited) = harness
        .admin(
            "GET",
            "/api/collections/points/records?nearestTo=embedding:1,0&nearestLimit=2",
            None,
        )
        .await;
    let limited_items = limited["items"].as_array().unwrap();
    assert_eq!(limited_items.len(), 2);
    assert_eq!(limited_items[0]["label"], "close");
    assert_eq!(limited_items[1]["label"], "medium");

    // The target may also be another record's id: nearest to "close"
    // itself should rank "close" first (similarity to itself is 1.0).
    let close_id = &ids["close"];
    let (status, by_id) = harness
        .admin(
            "GET",
            &format!("/api/collections/points/records?nearestTo=embedding:{close_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{by_id}");
    assert_eq!(by_id["items"][0]["label"], "close");

    // A non-vector field is a 400, not a silent empty page.
    let (status, body) = harness
        .admin(
            "GET",
            "/api/collections/points/records?nearestTo=label:1,0",
            None,
        )
        .await;
    assert_eq!(status, 400, "{body}");
}
