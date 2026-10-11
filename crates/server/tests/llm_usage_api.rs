//! End-to-end tests for the `_llm_usage` ledger endpoints:
//! `POST /api/llm/usage` (cost from the settings price table) and
//! `GET /api/llm/usage/summary`. Both are superuser-only and independent of
//! `settings.llm.enabled`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use serde_json::{json, Value};
use tower::ServiceExt;

struct Harness {
    app: App,
    superuser_token: String,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path().join("cb_data")));
        app.bootstrap().await.expect("bootstrap");
        let id = app
            .create_superuser("admin@example.com", "hunter2hunter2")
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
        let body = if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
        let resp = cratebase_server::router(self.app.clone())
            .oneshot(builder.body(body).unwrap())
            .await
            .expect("response");
        split(resp).await
    }

    async fn create_user(&self, email: &str) -> String {
        let collection = self.app.db().collections.get("users").expect("users");
        let mut record = cratebase_core::Record::new(collection);
        record.set("email", Value::String(email.to_string()));
        record.set("password", Value::String("supersecret123".into()));
        record.set("verified", Value::Bool(true));
        cratebase_db::records::create(self.app.db(), &self.app.db().collections, &mut record)
            .await
            .expect("create user");
        let id = record.id().to_string();
        self.app
            .mint_token("users", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token")
    }

    async fn set_prices(&self) {
        let (status, body) = self
            .request(
                "PATCH",
                "/api/settings",
                Some(&self.superuser_token),
                json!({"llm": {
                    "prices": {"m1": {"input": 1.0, "output": 2.0, "cachedInput": 0.1}},
                    "priceVersion": "v1"
                }}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}

async fn split(resp: Response) -> (StatusCode, Value) {
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

#[tokio::test]
async fn posts_usage_with_cost_and_summarizes_it() {
    let h = Harness::new().await;
    // The gateway is off: the ledger routes must not depend on it.
    assert!(!h.app.settings().llm.enabled);
    h.set_prices().await;
    let su = h.superuser_token.clone();

    let (status, body) = h
        .request(
            "POST",
            "/api/llm/usage",
            Some(&su),
            json!({"model": "m1", "promptTokens": 1000, "completionTokens": 500, "tokensCached": 200}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["costUsd"], json!(0.00182));
    assert!(body["id"].as_str().is_some_and(|s| !s.is_empty()));

    let (status, body) = h
        .request(
            "POST",
            "/api/llm/usage",
            Some(&su),
            json!({"model": "m1", "promptTokens": 1000, "completionTokens": 500, "tokensCached": 200,
                   "surface": "chat", "ok": true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = h
        .request(
            "GET",
            "/api/llm/usage/summary?groupBy=model",
            Some(&su),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["key"], "m1");
    assert_eq!(items[0]["calls"], 2);
    assert_eq!(items[0]["promptTokens"], 2000);
    assert_eq!(items[0]["completionTokens"], 1000);
    let cost = items[0]["costUsd"].as_f64().unwrap();
    assert!((cost - 0.00364).abs() < 1e-9, "{cost}");

    // Default groupBy is model; other groupings work; bad ones are 400.
    for group in ["surface", "purpose", "actor", "day"] {
        let (status, body) = h
            .request(
                "GET",
                &format!("/api/llm/usage/summary?groupBy={group}"),
                Some(&su),
                Value::Null,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{group}: {body}");
    }
    let (status, _) = h
        .request("GET", "/api/llm/usage/summary", Some(&su), Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = h
        .request(
            "GET",
            "/api/llm/usage/summary?groupBy=model%22%3BDROP",
            Some(&su),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Date range filter: nothing was created in the far future.
    let (_, body) = h
        .request(
            "GET",
            "/api/llm/usage/summary?from=2999-01-01",
            Some(&su),
            Value::Null,
        )
        .await;
    assert!(body["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn unpriced_model_has_null_cost() {
    let h = Harness::new().await;
    h.set_prices().await;
    let (status, body) = h
        .request(
            "POST",
            "/api/llm/usage",
            Some(&h.superuser_token),
            json!({"model": "other", "promptTokens": 10, "completionTokens": 5}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["costUsd"].is_null());
}

#[tokio::test]
async fn usage_endpoints_reject_non_superusers() {
    let h = Harness::new().await;
    let user = h.create_user("u@example.com").await;
    let payload = json!({"model": "m1", "promptTokens": 1, "completionTokens": 1});
    let (status, _) = h
        .request("POST", "/api/llm/usage", Some(&user), payload.clone())
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = h
        .request("GET", "/api/llm/usage/summary", Some(&user), Value::Null)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = h.request("POST", "/api/llm/usage", None, payload).await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "{status}"
    );
}

#[tokio::test]
async fn usage_post_validates_body() {
    let h = Harness::new().await;
    let su = h.superuser_token.clone();
    for (body, field) in [
        (json!({"promptTokens": 1, "completionTokens": 1}), "model"),
        (
            json!({"model": "  ", "promptTokens": 1, "completionTokens": 1}),
            "model",
        ),
        (
            json!({"model": "m", "promptTokens": -1, "completionTokens": 1}),
            "promptTokens",
        ),
        (json!({"model": "m", "promptTokens": 1}), "completionTokens"),
        (
            json!({"model": "m", "promptTokens": 1, "completionTokens": 1, "ok": "yes"}),
            "ok",
        ),
    ] {
        let (status, resp) = h.request("POST", "/api/llm/usage", Some(&su), body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{resp}");
        assert!(
            resp["message"].as_str().unwrap().contains(field),
            "{field}: {resp}"
        );
    }
}

#[tokio::test]
async fn deleting_a_price_makes_the_model_unpriced() {
    let h = Harness::new().await;
    h.set_prices().await;
    let su = h.superuser_token.clone();
    let payload = json!({"model": "m1", "promptTokens": 1000, "completionTokens": 500});

    let (_, body) = h
        .request("POST", "/api/llm/usage", Some(&su), payload.clone())
        .await;
    assert_eq!(body["costUsd"], json!(0.002));

    let (status, body) = h
        .request(
            "PATCH",
            "/api/settings",
            Some(&su),
            json!({"llm": {"prices": {"m1": null}}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = h
        .request("POST", "/api/llm/usage", Some(&su), payload)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["costUsd"].is_null(), "{body}");
}
