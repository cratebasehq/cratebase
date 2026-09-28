//! Tests for the MCP runtime tools (`call_rpc`, `send_email`,
//! `search_nearby`) and developer tools (`get_schema`, `apply_schema`,
//! `test_rule`, `list_email_templates`/`upsert_email_template`,
//! `list_email_triggers`/`upsert_email_trigger`, `query_logs`,
//! `sql_read`) added on top of the existing per-collection CRUD tools in
//! `crates/server/src/mcp.rs`.
//!
//! Every runtime tool reuses the same server-side function its HTTP
//! equivalent calls (see `mcp.rs`'s own doc comments), so these tests
//! focus on the MCP-specific contract: developer tools are invisible and
//! uncallable for a non-superuser, a runtime tool's rule/`sendRule`
//! enforcement survives being called through MCP instead of HTTP
//! directly, and `apply_schema`'s `dryRun` really defaults to `true`.

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

async fn memory_app() -> App {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(Config {
        log_requests: false,
        secret: "test-secret-0123456789".into(),
        ..Config::for_data_dir(dir.path().join("pb_data"))
    });
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

/// A regular (non-superuser) `users` record with a valid session token,
/// returning `(id, token)`.
async fn create_user(app: &App, email: &str) -> (String, String) {
    let collection = app.db().collections.get("users").expect("users");
    let mut record = cratebase_core::Record::new(collection);
    record.set("email", Value::String(email.to_string()));
    record.set("password", Value::String("supersecret123".into()));
    record.set("verified", Value::Bool(true));
    cratebase_db::records::create(app.db(), &app.db().collections, &mut record)
        .await
        .expect("create user");
    let id = record.id().to_string();
    let token = app
        .mint_token("users", &id, cratebase_auth::TokenType::Auth, 3600)
        .await
        .expect("token");
    (id, token)
}

/// One `POST /api/mcp` JSON-RPC `tools/call`, returning the parsed
/// `result` (`{content, structuredContent?, isError}`) — callers assert
/// on `result["isError"]`/`result["structuredContent"]`. Panics (via the
/// `expect` below) on a protocol-level JSON-RPC `error`, since every test
/// here that expects one checks for it explicitly with [`rpc_error`].
async fn tools_call(app: &App, token: Option<&str>, name: &str, arguments: Value) -> Value {
    let (status, body) = request(
        app,
        "POST",
        "/api/mcp",
        token,
        Some(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert!(
        body.get("error").is_none(),
        "unexpected protocol error: {body:?}"
    );
    body["result"].clone()
}

/// Same as [`tools_call`] but for a call expected to be refused at the
/// protocol level (an unknown/hidden tool) — returns the `error` object.
async fn tools_call_expect_error(
    app: &App,
    token: Option<&str>,
    name: &str,
    arguments: Value,
) -> Value {
    let (status, body) = request(
        app,
        "POST",
        "/api/mcp",
        token,
        Some(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    body["error"].clone()
}

async fn tools_list(app: &App, token: Option<&str>) -> Vec<String> {
    let (status, body) = request(
        app,
        "POST",
        "/api/mcp",
        token,
        Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    body["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

const DEV_TOOLS: &[&str] = &[
    "get_schema",
    "apply_schema",
    "test_rule",
    "list_email_templates",
    "upsert_email_template",
    "list_email_triggers",
    "upsert_email_trigger",
    "query_logs",
    "sql_read",
];

// --------------------------------------------------------- auth enforcement

#[tokio::test]
async fn dev_tools_are_hidden_from_tools_list_for_non_superusers() {
    let app = memory_app().await;
    let (_, user_token) = create_user(&app, "alice@example.com").await;

    for token in [None, Some(user_token.as_str())] {
        let names = tools_list(&app, token).await;
        for dev_tool in DEV_TOOLS {
            assert!(
                !names.contains(&dev_tool.to_string()),
                "{dev_tool} should be hidden for {token:?}, got {names:?}"
            );
        }
        // The three runtime tools are visible to everyone.
        for runtime_tool in ["call_rpc", "send_email", "search_nearby"] {
            assert!(
                names.contains(&runtime_tool.to_string()),
                "{runtime_tool} missing for {token:?}"
            );
        }
    }
}

#[tokio::test]
async fn dev_tools_are_visible_to_a_superuser() {
    let app = memory_app().await;
    let token = owner_token(&app).await;
    let names = tools_list(&app, Some(&token)).await;
    for dev_tool in DEV_TOOLS {
        assert!(
            names.contains(&dev_tool.to_string()),
            "{dev_tool} missing: {names:?}"
        );
    }
}

#[tokio::test]
async fn calling_a_dev_tool_as_a_non_superuser_is_refused_like_an_unknown_tool() {
    let app = memory_app().await;
    let (_, user_token) = create_user(&app, "bob@example.com").await;

    let unknown = tools_call_expect_error(&app, None, "totally_made_up_tool", json!({})).await;
    let anon = tools_call_expect_error(&app, None, "get_schema", json!({})).await;
    let user = tools_call_expect_error(
        &app,
        Some(&user_token),
        "sql_read",
        json!({"sql": "SELECT 1"}),
    )
    .await;

    // Same JSON-RPC error code (`method not found`) either way — nothing
    // distinguishes "doesn't exist" from "exists but you can't see it".
    assert_eq!(anon["code"], unknown["code"]);
    assert_eq!(user["code"], unknown["code"]);
}

#[tokio::test]
async fn a_superuser_can_call_a_dev_tool() {
    let app = memory_app().await;
    let token = owner_token(&app).await;
    let result = tools_call(&app, Some(&token), "get_schema", json!({})).await;
    assert_eq!(result["isError"], false, "{result:?}");
    assert!(!result["structuredContent"]["collections"]
        .as_array()
        .unwrap()
        .is_empty());
}

// ----------------------------------------------------------------- call_rpc

#[tokio::test]
async fn call_rpc_tool_enforces_the_rpc_rule() {
    let app = memory_app().await;
    let token = owner_token(&app).await;

    let (status, body) = request(
        &app,
        "POST",
        "/api/collections/_rpc/records",
        Some(&token),
        Some(json!({ "name": "public_ping", "sql": "SELECT 1 AS ok", "rule": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let (status, body) = request(
        &app,
        "POST",
        "/api/collections/_rpc/records",
        Some(&token),
        Some(json!({ "name": "secret_ping", "sql": "SELECT 1 AS ok" })), // rule omitted -> null
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    // Public rule: callable anonymously through the MCP tool.
    let result = tools_call(&app, None, "call_rpc", json!({ "name": "public_ping" })).await;
    assert_eq!(result["isError"], false, "{result:?}");
    assert_eq!(result["structuredContent"]["items"][0]["ok"], 1);

    // Null rule: the same MCP tool call is denied for the same anonymous
    // caller — the rule, not the transport, decides.
    let result = tools_call(&app, None, "call_rpc", json!({ "name": "secret_ping" })).await;
    assert_eq!(result["isError"], true, "{result:?}");

    // ...but a superuser can still call it.
    let result = tools_call(
        &app,
        Some(&token),
        "call_rpc",
        json!({ "name": "secret_ping" }),
    )
    .await;
    assert_eq!(result["isError"], false, "{result:?}");
}

// --------------------------------------------------------------- send_email

#[tokio::test]
async fn send_email_tool_enforces_send_rule() {
    let app = memory_app().await;
    let token = owner_token(&app).await;

    let (status, list) = request(
        &app,
        "GET",
        "/api/collections/_emailTemplates/records?filter=key='welcome'",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list:?}");
    let template_id = list["items"][0]["id"]
        .as_str()
        .expect("welcome template")
        .to_string();

    // Default (`sendRule: null`): denied for an anonymous MCP caller.
    let result = tools_call(
        &app,
        None,
        "send_email",
        json!({ "template": "welcome", "to": "someone@example.com" }),
    )
    .await;
    assert_eq!(result["isError"], true, "{result:?}");

    // Open it up (`sendRule: ""`), and the same anonymous call succeeds.
    let (status, body) = request(
        &app,
        "PATCH",
        &format!("/api/collections/_emailTemplates/records/{template_id}"),
        Some(&token),
        Some(json!({ "sendRule": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let result = tools_call(
        &app,
        None,
        "send_email",
        json!({ "template": "welcome", "to": "someone@example.com" }),
    )
    .await;
    assert_eq!(result["isError"], false, "{result:?}");
    assert!(matches!(
        result["structuredContent"]["status"].as_str(),
        Some("sent") | Some("queued")
    ));
}

// ------------------------------------------------------------ search_nearby

async fn seed_places(app: &App, token: &str) {
    let (status, body) = request(
        app,
        "POST",
        "/api/collections",
        Some(token),
        Some(json!({
            "name": "places",
            "type": "base",
            "fields": [
                { "name": "label", "type": "text" },
                { "name": "loc", "type": "geoPoint" },
            ],
            "listRule": "",
            "viewRule": "",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    for (label, lon, lat) in [("near", 0.01, 0.01), ("far", 40.0, 40.0)] {
        let (status, body) = request(
            app,
            "POST",
            "/api/collections/places/records",
            Some(token),
            Some(json!({ "label": label, "loc": { "lon": lon, "lat": lat } })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:?}");
    }
}

#[tokio::test]
async fn search_nearby_tool_lists_within_radius_respecting_list_rule() {
    let app = memory_app().await;
    let token = owner_token(&app).await;
    seed_places(&app, &token).await;

    let result = tools_call(
        &app,
        None,
        "search_nearby",
        json!({ "collection": "places", "field": "loc", "lon": 0, "lat": 0, "radiusKm": 100 }),
    )
    .await;
    assert_eq!(result["isError"], false, "{result:?}");
    let items = result["structuredContent"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["label"], "near");
}

#[tokio::test]
async fn search_nearby_tool_rejects_a_system_collection() {
    let app = memory_app().await;
    let result = tools_call(
        &app,
        None,
        "search_nearby",
        json!({ "collection": "_superusers", "field": "loc", "lon": 0, "lat": 0, "radiusKm": 1 }),
    )
    .await;
    assert_eq!(result["isError"], true, "{result:?}");
}

// -------------------------------------------------------------- apply_schema

#[tokio::test]
async fn apply_schema_defaults_dry_run_to_true() {
    let app = memory_app().await;
    let token = owner_token(&app).await;

    let payload = json!({
        "collections": [
            { "name": "widgets", "type": "base", "fields": [{ "name": "title", "type": "text" }] },
        ],
    });

    // No `dryRun` key at all -> defaults true -> nothing written.
    let result = tools_call(&app, Some(&token), "apply_schema", payload.clone()).await;
    assert_eq!(result["isError"], false, "{result:?}");
    assert_eq!(result["structuredContent"]["applied"], false);
    let (status, _) = request(&app, "GET", "/api/collections/widgets", Some(&token), None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "widgets should not exist yet"
    );

    // Explicit `dryRun: false` actually writes it.
    let mut with_flag = payload;
    with_flag["dryRun"] = json!(false);
    let result = tools_call(&app, Some(&token), "apply_schema", with_flag).await;
    assert_eq!(result["isError"], false, "{result:?}");
    assert_eq!(result["structuredContent"]["applied"], true);
    let (status, body) = request(&app, "GET", "/api/collections/widgets", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
}

// ----------------------------------------------------------------- test_rule

#[tokio::test]
async fn test_rule_tool_evaluates_allow_and_deny() {
    let app = memory_app().await;
    let token = owner_token(&app).await;
    let (alice_id, _) = create_user(&app, "alice@example.com").await;
    let (bob_id, _) = create_user(&app, "bob@example.com").await;

    let (status, body) = request(
        &app,
        "POST",
        "/api/collections",
        Some(&token),
        Some(json!({
            "name": "notes",
            "type": "base",
            "fields": [{ "name": "owner", "type": "text" }],
            "listRule": "",
            "viewRule": "",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let result = tools_call(
        &app,
        Some(&token),
        "test_rule",
        json!({
            "rule": "owner = @request.auth.id",
            "collection": "notes",
            "authRecordId": alice_id,
            "record": { "owner": alice_id },
        }),
    )
    .await;
    assert_eq!(result["isError"], false, "{result:?}");
    assert_eq!(result["structuredContent"]["allowed"], true, "{result:?}");

    let result = tools_call(
        &app,
        Some(&token),
        "test_rule",
        json!({
            "rule": "owner = @request.auth.id",
            "collection": "notes",
            "authRecordId": bob_id,
            "record": { "owner": alice_id },
        }),
    )
    .await;
    assert_eq!(result["structuredContent"]["allowed"], false, "{result:?}");

    // Anonymous (no authRecordId): denied against a rule requiring auth.
    let result = tools_call(
        &app,
        Some(&token),
        "test_rule",
        json!({
            "rule": "owner = @request.auth.id",
            "collection": "notes",
            "record": { "owner": alice_id },
        }),
    )
    .await;
    assert_eq!(result["structuredContent"]["allowed"], false, "{result:?}");
}

// -------------------------------------------------- email template/trigger CRUD

#[tokio::test]
async fn upsert_and_list_email_template_tool() {
    let app = memory_app().await;
    let token = owner_token(&app).await;

    let result = tools_call(
        &app,
        Some(&token),
        "upsert_email_template",
        json!({ "key": "mcp-test", "name": "MCP test", "subject": "Hi", "html": "<p>hi</p>" }),
    )
    .await;
    assert_eq!(result["isError"], false, "{result:?}");
    let id = result["structuredContent"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let result = tools_call(
        &app,
        Some(&token),
        "list_email_templates",
        json!({ "filter": "key = 'mcp-test'" }),
    )
    .await;
    assert_eq!(result["isError"], false, "{result:?}");
    let items = result["structuredContent"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id);

    // Update via the same tool, passing `id`.
    let result = tools_call(
        &app,
        Some(&token),
        "upsert_email_template",
        json!({ "id": id, "subject": "Updated" }),
    )
    .await;
    assert_eq!(result["isError"], false, "{result:?}");
    assert_eq!(result["structuredContent"]["subject"], "Updated");
}

// --------------------------------------------------------------- sql_read

#[tokio::test]
async fn sql_read_tool_only_accepts_reads() {
    let app = memory_app().await;
    let token = owner_token(&app).await;

    let result = tools_call(
        &app,
        Some(&token),
        "sql_read",
        json!({ "sql": "DELETE FROM _collections" }),
    )
    .await;
    assert_eq!(result["isError"], true, "{result:?}");

    let result = tools_call(
        &app,
        Some(&token),
        "sql_read",
        json!({ "sql": "SELECT 1 AS n" }),
    )
    .await;
    assert_eq!(result["isError"], false, "{result:?}");
    assert_eq!(result["structuredContent"]["rows"][0]["n"], 1);
}

// ---------------------------------------------------------------- query_logs

#[tokio::test]
async fn query_logs_tool_is_callable_by_a_superuser() {
    let app = memory_app().await;
    let token = owner_token(&app).await;
    let result = tools_call(&app, Some(&token), "query_logs", json!({ "perPage": 5 })).await;
    assert_eq!(result["isError"], false, "{result:?}");
    assert!(result["structuredContent"]["items"].is_array());
}

#[tokio::test]
async fn a_geo_distance_sort_with_no_filter_does_not_500() {
    // Regression test for a real bug this pass found and fixed
    // (`crates/db/src/query.rs`'s `Query::tail_params` split): a
    // `sort=geoDistance(...)` bound its own literal params into the same
    // vec `count_sql()` executed against, even though `count_sql()` has
    // no `ORDER BY` — with no `filter=` present to "absorb" the mismatch
    // this was an outright param-count error the driver rejected, a
    // `500`, not a `400`. `search_nearby`'s happy-path test above always
    // sends both `filter` and `sort` together (which happened to hit the
    // very same bug in the other direction — more bound params than
    // placeholders), so this covers the sort-only shape directly, over
    // real HTTP, through the real router.
    let app = memory_app().await;
    let token = owner_token(&app).await;
    seed_places(&app, &token).await;

    let (status, body) = request(&app, "GET", "/api/collections/places/records", None, None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let (status, body) = request(
        &app,
        "GET",
        "/api/collections/places/records?filter=geoDistance(loc.lon,+loc.lat,+0,+0)+%3C+100",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    let (status, body) = request(
        &app,
        "GET",
        "/api/collections/places/records?sort=geoDistance(loc.lon,+loc.lat,+0,+0)",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["label"], "near"); // nearest first
    assert_eq!(items[1]["label"], "far");
}
