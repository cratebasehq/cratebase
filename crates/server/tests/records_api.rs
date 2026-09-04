//! End-to-end tests for the collection, record, file and password-auth
//! routes, driven through `tower::ServiceExt::oneshot` against an
//! in-memory database — no listener, no ports.
//!
//! The conformance suite (`tests/conformance`) is the behavioural
//! specification and runs against the real PocketBase binary too; what
//! lives here is what a Rust-level regression would break first: the rule
//! outcomes, the serialization options, the transaction boundary and the
//! "one token resolution per request" performance invariant.

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

    async fn raw(&self, request: Request<Body>) -> Response {
        cratebase_server::router(self.app.clone())
            .oneshot(request)
            .await
            .expect("response")
    }

    /// A request as the superuser.
    async fn admin(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.as_user(method, uri, body, Some(&self.token)).await
    }

    /// A request as `token`, or anonymously when it is `None`.
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

    /// Create a collection through the HTTP API and return its id.
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

fn posts(name: &str) -> Value {
    json!({
        "name": name,
        "type": "base",
        "listRule": "",
        "viewRule": "",
        "createRule": "",
        "updateRule": "",
        "deleteRule": "",
        "fields": [
            {"name": "title", "type": "text", "required": true, "min": 3},
            {"name": "views", "type": "number", "onlyInt": true},
            {"name": "tags", "type": "select", "values": ["go", "rust"], "maxSelect": 2},
            {"name": "created", "type": "autodate", "onCreate": true},
        ],
    })
}

// ------------------------------------------------------------- collections

#[tokio::test]
async fn collections_crud_and_the_paginated_envelope() {
    let harness = Harness::new().await;
    let id = harness.collection(posts("posts")).await;
    assert!(id.starts_with("pbc_"), "{id}");

    // A bare array would break the SDK: the list is an envelope.
    let (status, page) = harness.admin("GET", "/api/collections", None).await;
    assert_eq!(status, 200);
    let mut keys: Vec<&str> = page
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        ["items", "page", "perPage", "totalItems", "totalPages"]
    );
    assert!(page["totalItems"].as_i64().unwrap() >= 6, "{page}");

    let (_, filtered) = harness
        .admin(
            "GET",
            "/api/collections?filter=name%20%3D%20%22posts%22",
            None,
        )
        .await;
    assert_eq!(filtered["totalItems"], 1);
    assert_eq!(filtered["items"][0]["name"], "posts");

    // By id and by name.
    let (status, by_name) = harness.admin("GET", "/api/collections/posts", None).await;
    assert_eq!(status, 200);
    assert_eq!(by_name["id"], id);
    assert_eq!(
        harness
            .admin("GET", &format!("/api/collections/{id}"), None)
            .await
            .1["name"],
        "posts"
    );
    assert_eq!(
        harness.admin("GET", "/api/collections/nope", None).await.0,
        404
    );

    // A `PATCH` is partial: rules change, fields survive.
    let (status, updated) = harness
        .admin(
            "PATCH",
            "/api/collections/posts",
            Some(json!({"listRule": "views > 0"})),
        )
        .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["listRule"], "views > 0");
    assert_eq!(updated["fields"].as_array().unwrap().len(), 5);

    // Superuser only.
    let (status, body) = harness.as_user("GET", "/api/collections", None, None).await;
    assert_eq!(status, 401);
    assert_eq!(body["data"], json!({}));
}

#[tokio::test]
async fn collection_validation_matches_pocketbase_shapes() {
    let harness = Harness::new().await;
    harness.collection(posts("posts")).await;

    // Duplicate name, case-insensitively.
    let (status, body) = harness
        .admin(
            "POST",
            "/api/collections",
            Some(json!({"name": "POSTS", "type": "base", "fields": []})),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["message"], "Failed to create collection.");
    assert_eq!(
        body["data"]["name"]["code"],
        "validation_collection_name_exists"
    );

    // Invalid identifier.
    let (_, body) = harness
        .admin(
            "POST",
            "/api/collections",
            Some(json!({"name": "bad name!", "type": "base", "fields": []})),
        )
        .await;
    assert_eq!(body["data"]["name"]["code"], "validation_match_invalid");

    // A rule that does not compile against the collection being saved.
    let (_, body) = harness
        .admin(
            "POST",
            "/api/collections",
            Some(json!({"name": "badrule", "type": "base", "fields": [], "listRule": "nope = 1"})),
        )
        .await;
    assert_eq!(body["data"]["listRule"]["code"], "validation_invalid_rule");

    // List-typed properties report by index, not by property name.
    let (_, body) = harness
        .admin(
            "POST",
            "/api/collections",
            Some(json!({"name": "badidx", "type": "base", "fields": [], "indexes": ["not an index"]})),
        )
        .await;
    assert_eq!(
        body["data"]["indexes"]["0"]["code"],
        "validation_invalid_index_expression"
    );

    // An unknown field type is a *body-format* error, not a field error.
    let (status, body) = harness
        .admin(
            "POST",
            "/api/collections",
            Some(json!({"name": "notype", "type": "base", "fields": [{"name": "a", "type": "bogus"}]})),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["message"],
        "Failed to load the submitted data due to invalid formatting."
    );
    assert_eq!(body["data"], json!({}));

    // `type` is immutable.
    let (_, body) = harness
        .admin(
            "PATCH",
            "/api/collections/posts",
            Some(json!({"type": "auth"})),
        )
        .await;
    assert_eq!(
        body["data"]["type"]["code"],
        "validation_collection_type_change"
    );

    // System collections are protected.
    let (status, body) = harness
        .admin("DELETE", "/api/collections/_superusers", None)
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["message"], "Failed to delete collection.");
}

// ----------------------------------------------------------------- records

#[tokio::test]
async fn record_crud_round_trip() {
    let harness = Harness::new().await;
    harness.collection(posts("posts")).await;

    let (status, created) = harness
        .admin(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "First", "views": 3, "tags": ["go"]})),
        )
        .await;
    assert_eq!(status, 200, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(id.len(), 15);
    assert_eq!(created["collectionName"], "posts");
    assert_eq!(created["views"], 3);
    assert_eq!(created["tags"], json!(["go"]));

    let (status, one) = harness
        .admin("GET", &format!("/api/collections/posts/records/{id}"), None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(one["title"], "First");

    let (status, updated) = harness
        .admin(
            "PATCH",
            &format!("/api/collections/posts/records/{id}"),
            Some(json!({"views": 9})),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(updated["views"], 9);
    assert_eq!(updated["title"], "First", "PATCH leaves other fields alone");

    // `+`/`-` work on numbers as well as on multi-valued fields.
    let (_, bumped) = harness
        .admin(
            "PATCH",
            &format!("/api/collections/posts/records/{id}"),
            Some(json!({"views+": 5, "tags+": ["rust"]})),
        )
        .await;
    assert_eq!(bumped["views"], 14);
    assert_eq!(bumped["tags"], json!(["go", "rust"]));

    let response = harness
        .raw(
            Request::delete(format!("/api/collections/posts/records/{id}"))
                .header("authorization", &harness.token)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        harness
            .admin("GET", &format!("/api/collections/posts/records/{id}"), None)
            .await
            .0,
        404
    );
}

#[tokio::test]
async fn list_pagination_projection_and_skip_total() {
    let harness = Harness::new().await;
    harness.collection(posts("posts")).await;
    for i in 1..=3 {
        harness
            .admin(
                "POST",
                "/api/collections/posts/records",
                Some(json!({"title": format!("Post {i}"), "views": i})),
            )
            .await;
    }

    let (status, page) = harness
        .admin(
            "GET",
            "/api/collections/posts/records?perPage=2&sort=views",
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(page["page"], 1);
    assert_eq!(page["perPage"], 2);
    assert_eq!(page["totalItems"], 3);
    assert_eq!(page["totalPages"], 2);
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    assert_eq!(page["items"][0]["views"], 1);

    // `perPage` is capped and `page` floors at 1.
    let (_, clamped) = harness
        .admin(
            "GET",
            "/api/collections/posts/records?page=0&perPage=5000",
            None,
        )
        .await;
    assert_eq!(clamped["page"], 1);
    assert_eq!(clamped["perPage"], 1000);

    // `skipTotal` skips the COUNT and reports -1 rather than 0.
    let (_, skipped) = harness
        .admin("GET", "/api/collections/posts/records?skipTotal=1", None)
        .await;
    assert_eq!(skipped["totalItems"], -1);
    assert_eq!(skipped["totalPages"], -1);

    // `?fields=` projects each item, not the envelope.
    let (_, projected) = harness
        .admin("GET", "/api/collections/posts/records?fields=id", None)
        .await;
    for item in projected["items"].as_array().unwrap() {
        assert_eq!(item.as_object().unwrap().keys().count(), 1);
        assert!(item["id"].is_string());
    }

    // An unusable sort is the generic 400 with no `data`.
    let (status, body) = harness
        .admin("GET", "/api/collections/posts/records?sort=nope", None)
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["message"],
        "Something went wrong while processing your request."
    );
    assert_eq!(body["data"], json!({}));
}

#[tokio::test]
async fn validation_errors_use_the_operation_wrapper() {
    let harness = Harness::new().await;
    harness.collection(posts("posts")).await;

    let (status, body) = harness
        .admin("POST", "/api/collections/posts/records", Some(json!({})))
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["message"], "Failed to create record.");
    assert_eq!(body["data"]["title"]["code"], "validation_required");

    let (_, body) = harness
        .admin(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "ab"})),
        )
        .await;
    assert_eq!(
        body["data"]["title"]["code"],
        "validation_min_text_constraint"
    );

    // A malformed body on a *records* endpoint is the generic message,
    // unlike the collections endpoints (KNOWN_DIVERGENCES §17).
    let (status, body) = harness
        .send(
            Request::post("/api/collections/posts/records")
                .header("authorization", &harness.token)
                .header("content-type", "application/json")
                .body(Body::from("{not json"))
                .unwrap(),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["message"],
        "Something went wrong while processing your request."
    );
    assert_eq!(body["data"], json!({}));
}

/// PocketBase answers a failed rule four different ways; collapsing them
/// into one would be an information leak in one direction and a broken
/// client in the other.
#[tokio::test]
async fn the_four_rule_outcomes_stay_distinct() {
    let harness = Harness::new().await;
    // Every rule `null` — superusers only.
    harness
        .collection(json!({
            "name": "locked",
            "type": "base",
            "fields": [{"name": "title", "type": "text"}],
        }))
        .await;
    // Rules that can never match for a guest.
    harness
        .collection(json!({
            "name": "owned",
            "type": "base",
            "fields": [{"name": "title", "type": "text"}],
            "listRule": "@request.auth.id != ''",
            "viewRule": "@request.auth.id != ''",
            "createRule": "@request.auth.id != ''",
            "updateRule": "@request.auth.id != ''",
            "deleteRule": "@request.auth.id != ''",
        }))
        .await;
    let (_, seeded) = harness
        .admin(
            "POST",
            "/api/collections/owned/records",
            Some(json!({"title": "hidden"})),
        )
        .await;
    let id = seeded["id"].as_str().unwrap().to_string();

    // 1. a `null` rule is a 403, decided before any query runs.
    let (status, body) = harness
        .as_user("GET", "/api/collections/locked/records", None, None)
        .await;
    assert_eq!(status, 403);
    assert_eq!(body["message"], "Only superusers can perform this action.");
    assert_eq!(body["data"], json!({}));

    // 2. a failing `listRule` is an empty page, not an error.
    let (status, page) = harness
        .as_user("GET", "/api/collections/owned/records", None, None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(page["totalItems"], 0);
    assert_eq!(page["items"], json!([]));

    // 3. a failing `viewRule`/`updateRule`/`deleteRule` is a 404, so the
    //    API cannot confirm that a hidden id exists.
    for (method, body) in [
        ("GET", None),
        ("PATCH", Some(json!({"title": "x"}))),
        ("DELETE", None),
    ] {
        let (status, _) = harness
            .as_user(
                method,
                &format!("/api/collections/owned/records/{id}"),
                body,
                None,
            )
            .await;
        assert_eq!(status, 404, "{method} must not leak the record's existence");
    }

    // 4. a failing `createRule` is a 400 with an empty `data`.
    let (status, body) = harness
        .as_user(
            "POST",
            "/api/collections/owned/records",
            Some(json!({"title": "x"})),
            None,
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["message"], "Failed to create record.");
    assert_eq!(body["data"], json!({}));
}

#[tokio::test]
async fn a_failed_write_rolls_back_and_fires_the_error_hook() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let harness = Harness::new().await;
    harness.collection(posts("posts")).await;

    let failures = Arc::new(AtomicUsize::new(0));
    let counter = failures.clone();
    harness
        .app
        .hooks()
        .on_record_create_execute
        .bind_func(move |_e| {
            let counter = counter.clone();
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err(cratebase_core::AppError::bad_request("hook says no"))
            })
        });

    let (status, body) = harness
        .admin(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "Never"})),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["message"], "hook says no");
    assert_eq!(failures.load(Ordering::SeqCst), 1);

    // The transaction rolled back, so nothing was written.
    let (_, page) = harness
        .admin("GET", "/api/collections/posts/records", None)
        .await;
    assert_eq!(page["totalItems"], 0);
}

/// A plain delete skips the explicit transaction (one statement, nothing
/// that can join or abort it). The moment a delete hook is bound that is
/// no longer true, and the row has to come back when the hook fails
/// *after* the `DELETE` has already run.
#[tokio::test]
async fn a_delete_hook_that_fails_after_the_row_is_gone_rolls_it_back() {
    let harness = Harness::new().await;
    harness.collection(posts("posts")).await;
    let (status, created) = harness
        .admin(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "Keep me"})),
        )
        .await;
    assert_eq!(status, 200, "{created}");
    let id = created["id"].as_str().expect("id").to_string();

    harness
        .app
        .hooks()
        .on_record_after_delete_success
        .bind_func(|_e| {
            Box::pin(async move { Err(cratebase_core::AppError::bad_request("hook says no")) })
        });

    let (status, body) = harness
        .admin(
            "DELETE",
            &format!("/api/collections/posts/records/{id}"),
            None,
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["message"], "hook says no");

    // The DELETE ran before the hook did, so only a transaction can put
    // the row back.
    let (status, back) = harness
        .admin("GET", &format!("/api/collections/posts/records/{id}"), None)
        .await;
    assert_eq!(
        status, 200,
        "the failed delete must have rolled back: {back}"
    );
    assert_eq!(back["title"], "Keep me");
}

/// The same delete without a hook: still one row gone, still a 204, and
/// the record is really gone rather than rolled back by accident.
#[tokio::test]
async fn a_plain_delete_commits_without_an_explicit_transaction() {
    let harness = Harness::new().await;
    harness.collection(posts("posts")).await;
    let (_, created) = harness
        .admin(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "Delete me"})),
        )
        .await;
    let id = created["id"].as_str().expect("id").to_string();

    let response = harness
        .raw(
            axum::http::Request::builder()
                .method("DELETE")
                .uri(format!("/api/collections/posts/records/{id}"))
                .header("authorization", &harness.token)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let (status, _) = harness
        .admin("GET", &format!("/api/collections/posts/records/{id}"), None)
        .await;
    assert_eq!(status, 404);
}

// -------------------------------------------------------------------- auth

/// Creates a `users` auth collection and one member, returning that
/// member's id and session token.
async fn member(harness: &Harness) -> (String, String) {
    harness
        .collection(json!({
            "name": "members",
            "type": "auth",
            "listRule": "",
            "viewRule": "",
            "createRule": "",
            "updateRule": "id = @request.auth.id",
            "fields": [],
        }))
        .await;
    let (status, created) = harness
        .admin(
            "POST",
            "/api/collections/members/records",
            Some(json!({
                "email": "member@example.com",
                "password": "hunter2hunter2",
                "passwordConfirm": "hunter2hunter2",
            })),
        )
        .await;
    assert_eq!(status, 200, "{created}");
    let id = created["id"].as_str().unwrap().to_string();

    let (status, auth) = harness
        .as_user(
            "POST",
            "/api/collections/members/auth-with-password",
            Some(json!({"identity": "member@example.com", "password": "hunter2hunter2"})),
            None,
        )
        .await;
    assert_eq!(status, 200, "{auth}");
    (id, auth["token"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn password_auth_refresh_and_methods() {
    let harness = Harness::new().await;
    let (id, token) = member(&harness).await;

    // The claims are PocketBase's exactly; the SDK reads `refreshable`.
    let payload = token.split('.').nth(1).expect("a jwt payload");
    let decoded = base64_url(payload);
    let claims: Value = serde_json::from_slice(&decoded).unwrap();
    let mut keys: Vec<&str> = claims
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(keys, ["collectionId", "exp", "id", "refreshable", "type"]);
    assert_eq!(claims["type"], "auth");
    assert_eq!(claims["refreshable"], true);
    assert_eq!(claims["id"], id);

    // Credentials never come back.
    let (_, auth) = harness
        .as_user(
            "POST",
            "/api/collections/members/auth-with-password",
            Some(json!({"identity": "member@example.com", "password": "hunter2hunter2"})),
            None,
        )
        .await;
    assert!(auth["record"].get("password").is_none());
    assert!(auth["record"].get("tokenKey").is_none());
    assert_eq!(auth["record"]["email"], "member@example.com");

    // A wrong password is a flat 400 with no `data` — no oracle.
    let (status, body) = harness
        .as_user(
            "POST",
            "/api/collections/members/auth-with-password",
            Some(json!({"identity": "member@example.com", "password": "wrong"})),
            None,
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body,
        json!({"status": 400, "message": "Failed to authenticate.", "data": {}})
    );

    // An empty payload is a field-level validation error instead.
    let (status, body) = harness
        .as_user(
            "POST",
            "/api/collections/members/auth-with-password",
            Some(json!({"identity": "", "password": ""})),
            None,
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["data"]["identity"]["code"], "validation_required");
    assert_eq!(body["data"]["password"]["code"], "validation_required");

    // Refresh: same record, new token; a token from another collection is
    // a 403 that names the *authenticated* record's collection.
    let (status, refreshed) = harness
        .as_user(
            "POST",
            "/api/collections/members/auth-refresh",
            None,
            Some(&token),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(refreshed["record"]["id"], id);
    let (status, body) = harness
        .as_user(
            "POST",
            "/api/collections/members/auth-refresh",
            None,
            Some(&harness.token),
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(
        body["message"],
        "The request requires auth record from _superusers collection."
    );
    assert_eq!(
        harness
            .as_user("POST", "/api/collections/members/auth-refresh", None, None)
            .await
            .0,
        401
    );

    // Auth methods: a disabled method reports a zero duration.
    let (status, methods) = harness
        .as_user("GET", "/api/collections/members/auth-methods", None, None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        methods["password"],
        json!({"enabled": true, "identityFields": ["email"]})
    );
    assert_eq!(
        methods["oauth2"],
        json!({"enabled": false, "providers": []})
    );
    assert_eq!(methods["mfa"], json!({"enabled": false, "duration": 0}));
    assert_eq!(methods["otp"], json!({"enabled": false, "duration": 0}));

    // An auth endpoint on a base collection, and `auth-methods` on a name
    // that is not a collection at all, are worded differently.
    harness.collection(posts("posts")).await;
    let (status, body) = harness
        .as_user(
            "POST",
            "/api/collections/posts/auth-with-password",
            Some(json!({"identity": "a", "password": "b"})),
            None,
        )
        .await;
    assert_eq!(status, 404);
    assert_eq!(
        body["message"],
        "Missing or invalid auth collection context."
    );
    let (status, body) = harness
        .as_user("GET", "/api/collections/nope/auth-methods", None, None)
        .await;
    assert_eq!(status, 404);
    assert_eq!(body["message"], "Missing or invalid collection context.");
}

#[tokio::test]
async fn disabled_password_auth_is_a_403_not_a_400() {
    let harness = Harness::new().await;
    harness
        .collection(json!({
            "name": "nopass",
            "type": "auth",
            "fields": [],
            "passwordAuth": {"enabled": false, "identityFields": ["email"]},
        }))
        .await;
    let (status, body) = harness
        .as_user(
            "POST",
            "/api/collections/nopass/auth-with-password",
            Some(json!({"identity": "a@b.co", "password": "whatever"})),
            None,
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(
        body["message"],
        "The collection is not configured to allow password authentication."
    );
}

#[tokio::test]
async fn a_password_change_invalidates_existing_sessions() {
    let harness = Harness::new().await;
    let (id, token) = member(&harness).await;

    // Self-service needs the old password.
    let (status, body) = harness
        .as_user(
            "PATCH",
            &format!("/api/collections/members/records/{id}"),
            Some(json!({"password": "newpassword1", "passwordConfirm": "newpassword1"})),
            Some(&token),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["message"], "Failed to update record.");
    assert_eq!(body["data"]["oldPassword"]["code"], "validation_required");

    let (status, _) = harness
        .as_user(
            "PATCH",
            &format!("/api/collections/members/records/{id}"),
            Some(json!({
                "oldPassword": "hunter2hunter2",
                "password": "newpassword1",
                "passwordConfirm": "newpassword1",
            })),
            Some(&token),
        )
        .await;
    assert_eq!(status, 200);

    // `tokenKey` rotated, so the token that made the change is now dead.
    let (status, _) = harness
        .as_user(
            "POST",
            "/api/collections/members/auth-refresh",
            None,
            Some(&token),
        )
        .await;
    assert_eq!(status, 401);
    let (status, _) = harness
        .as_user(
            "POST",
            "/api/collections/members/auth-with-password",
            Some(json!({"identity": "member@example.com", "password": "newpassword1"})),
            None,
        )
        .await;
    assert_eq!(status, 200);
}

/// Setting `verified` on yourself is a *values mismatch*, not a
/// permission error — clients branch on the code (KNOWN_DIVERGENCES §21).
#[tokio::test]
async fn self_service_cannot_set_verified_or_email() {
    let harness = Harness::new().await;
    let (id, token) = member(&harness).await;

    for (field, value) in [
        ("verified", json!(true)),
        ("email", json!("other@example.com")),
    ] {
        let (status, body) = harness
            .as_user(
                "PATCH",
                &format!("/api/collections/members/records/{id}"),
                Some(json!({ field: value })),
                Some(&token),
            )
            .await;
        assert_eq!(status, 400, "{field}");
        assert_eq!(
            body["data"][field],
            json!({"code": "validation_values_mismatch", "message": "Values don't match."})
        );
    }

    // `emailVisibility` is an ordinary field.
    let (status, updated) = harness
        .as_user(
            "PATCH",
            &format!("/api/collections/members/records/{id}"),
            Some(json!({"emailVisibility": true})),
            Some(&token),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(updated["emailVisibility"], true);
}

/// The Phase 1 regression this guards: the logging layer, the rate-limit
/// layer and the handler each resolving the caller independently. The
/// record path adds a third asker, so it is worth re-asserting here.
#[tokio::test]
async fn a_record_request_resolves_its_token_exactly_once() {
    let harness = Harness::new().await;
    let (_, token) = member(&harness).await;
    harness.collection(posts("posts")).await;

    let before = harness.app.auth_resolutions();
    let (status, _) = harness
        .as_user("GET", "/api/collections/posts/records", None, Some(&token))
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        harness.app.auth_resolutions() - before,
        1,
        "one authenticated record request must cost exactly one token resolution"
    );

    let before = harness.app.auth_resolutions();
    let (status, _) = harness
        .as_user(
            "POST",
            "/api/collections/posts/records",
            Some(json!({"title": "Once"})),
            Some(&token),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(harness.app.auth_resolutions() - before, 1);

    // An anonymous request resolves nothing at all.
    let before = harness.app.auth_resolutions();
    harness
        .as_user("GET", "/api/collections/posts/records", None, None)
        .await;
    assert_eq!(harness.app.auth_resolutions() - before, 0);
}

// ------------------------------------------------------------------- files

fn multipart(parts: &[(&str, Option<&str>, &str)]) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "----cratebasetestboundary";
    let mut body = Vec::new();
    for (name, filename, content) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        match filename {
            Some(filename) => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: text/plain\r\n\r\n"
                )
                .as_bytes(),
            ),
            None => body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
            ),
        }
        body.extend_from_slice(content.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

#[tokio::test]
async fn multipart_upload_download_and_cleanup() {
    let harness = Harness::new().await;
    let collection_id = harness
        .collection(json!({
            "name": "docs",
            "type": "base",
            "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": "",
            "fields": [
                {"name": "title", "type": "text"},
                {"name": "doc", "type": "file", "maxSelect": 1, "mimeTypes": ["text/plain"]},
            ],
        }))
        .await;

    let (content_type, body) = multipart(&[
        ("@jsonPayload", None, r#"{"title":"From JSON"}"#),
        ("doc", Some("notes.txt"), "hello world"),
    ]);
    let (status, created) = harness
        .send(
            Request::post("/api/collections/docs/records")
                .header("authorization", &harness.token)
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(status, 200, "{created}");
    // `@jsonPayload` is merged with the file parts.
    assert_eq!(created["title"], "From JSON");
    let id = created["id"].as_str().unwrap().to_string();
    let stored = created["doc"].as_str().unwrap().to_string();
    assert!(stored.starts_with("notes_"), "{stored}");
    assert!(stored.ends_with(".txt"));

    let url = format!("/api/files/{collection_id}/{id}/{stored}");
    let response = harness
        .raw(Request::get(&url).body(Body::empty()).unwrap())
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/plain");
    assert_eq!(
        response.headers()["content-disposition"],
        format!("inline; filename=\"{stored}\"")
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], b"hello world");

    // `?download=1` only changes the disposition.
    let response = harness
        .raw(
            Request::get(format!("{url}?download=1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert!(response.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .starts_with("attachment;"));

    // A name the record does not hold is a 404, whatever is in storage.
    let response = harness
        .raw(
            Request::get(format!("/api/files/{collection_id}/{id}/nope.txt"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), 404);

    // Deleting the record sweeps its files.
    let response = harness
        .raw(
            Request::delete(format!("/api/collections/docs/records/{id}"))
                .header("authorization", &harness.token)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = harness
        .raw(Request::get(&url).body(Body::empty()).unwrap())
        .await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn file_tokens_are_minted_for_any_authenticated_record() {
    let harness = Harness::new().await;
    let (id, token) = member(&harness).await;

    let (status, body) = harness
        .as_user("POST", "/api/files/token", None, Some(&token))
        .await;
    assert_eq!(status, 200);
    let minted = body["token"].as_str().expect("a token");
    let claims: Value =
        serde_json::from_slice(&base64_url(minted.split('.').nth(1).unwrap())).unwrap();
    assert_eq!(claims["type"], "file");
    assert_eq!(claims["id"], id);

    // A file token is single-purpose: it must not authenticate an
    // ordinary request.
    let (status, _) = harness
        .as_user("GET", "/api/settings", None, Some(minted))
        .await;
    assert_eq!(status, 401);

    // Minting requires auth.
    let (status, body) = harness
        .as_user("POST", "/api/files/token", None, None)
        .await;
    assert_eq!(status, 401);
    assert_eq!(
        body["message"],
        "The request requires valid record authorization token."
    );
}

/// `protected: true` does not mean "always needs a token": PocketBase
/// runs the collection's `viewRule` for the token owner, so a protected
/// file in a public collection is public (KNOWN_DIVERGENCES §29).
#[tokio::test]
async fn protected_files_follow_the_view_rule_not_the_flag() {
    let harness = Harness::new().await;
    let public_id = harness
        .collection(json!({
            "name": "publicdocs",
            "type": "base",
            "listRule": "", "viewRule": "", "createRule": "",
            "fields": [{"name": "secret", "type": "file", "maxSelect": 1, "protected": true}],
        }))
        .await;
    let private_id = harness
        .collection(json!({
            "name": "privatedocs",
            "type": "base",
            "fields": [{"name": "secret", "type": "file", "maxSelect": 1, "protected": true}],
        }))
        .await;

    let mut urls = Vec::new();
    for (collection, collection_id) in [("publicdocs", &public_id), ("privatedocs", &private_id)] {
        let (content_type, body) = multipart(&[("secret", Some("s.txt"), "classified")]);
        let (status, created) = harness
            .send(
                Request::post(format!("/api/collections/{collection}/records"))
                    .header("authorization", &harness.token)
                    .header("content-type", content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(status, 200, "{created}");
        urls.push(format!(
            "/api/files/{collection_id}/{}/{}",
            created["id"].as_str().unwrap(),
            created["secret"].as_str().unwrap()
        ));
    }

    // Public collection: served to anyone, protected flag notwithstanding.
    let response = harness
        .raw(Request::get(&urls[0]).body(Body::empty()).unwrap())
        .await;
    assert_eq!(response.status(), 200);

    // Private collection (`viewRule: null`): a guest gets a 404, not a
    // 403, and neither does a garbage token help.
    for suffix in ["", "?token=not.a.token"] {
        let response = harness
            .raw(
                Request::get(format!("{}{suffix}", urls[1]))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), 404, "{suffix}");
    }

    // A superuser's file token opens it.
    let (_, minted) = harness.admin("POST", "/api/files/token", None).await;
    let response = harness
        .raw(
            Request::get(format!(
                "{}?token={}",
                urls[1],
                minted["token"].as_str().unwrap()
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_eq!(response.status(), 200);
}

/// Minimal base64url decoder for JWT payload assertions.
fn base64_url(input: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        let Some(value) = ALPHABET.iter().position(|c| *c == byte) else {
            continue;
        };
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}
