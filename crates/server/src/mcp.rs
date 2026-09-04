//! `GET /api/mcp` + `POST /api/mcp` — collections exposed as [Model
//! Context Protocol](https://modelcontextprotocol.io) tools.
//!
//! # Why hand-rolled JSON-RPC instead of an MCP crate
//!
//! Nothing in `crates/server/Cargo.toml` pulls in a protocol crate for
//! anything shaped like this (the closest precedents — `jsonwebtoken`,
//! `croner`, `qrcode` — are all narrow, single-purpose codecs, not
//! frameworks that own a transport loop). The official `rmcp` crate
//! exists on crates.io, but it wants to own the whole server (its own
//! router, its own session/state model) rather than slot one route into
//! an existing `axum::Router<App>` the way every other endpoint here
//! does, and pulling in a whole SDK to emit a JSON-RPC envelope with
//! three methods (`initialize`, `tools/list`, `tools/call`) is a lot of
//! surface area to audit for not much saved code. The envelope itself —
//! `{jsonrpc, id, method, params}` in, `{jsonrpc, id, result|error}` out
//! — is a couple of structs and a `match`, so hand-rolling it here is
//! both simpler and easier to verify against the spec line by line. If
//! MCP support grows well beyond collection CRUD (resources, prompts,
//! bidirectional server-initiated requests), that calculus should be
//! revisited.
//!
//! # Transport
//!
//! The spec's 2024-11-05 "HTTP+SSE" transport pairs a `GET` (the SSE
//! stream the server pushes messages over) with a `POST` (JSON-RPC
//! requests in). This server takes a deliberately simplified reading of
//! that pairing for its first pass: every `tools/call` is answered
//! **synchronously** in the `POST` response body, not funnelled back
//! through the SSE stream — there is no per-request work here slow or
//! asynchronous enough to need a push channel, and a synchronous
//! response is what every plain HTTP client (including the one this
//! module's own tests use) expects without needing to speak SSE at all.
//! `GET /api/mcp` still opens a real SSE stream and emits the spec's
//! `endpoint` handshake event, so a client that *does* expect the
//! two-leg dance finds a working stream to open — it just never carries
//! an independent reply, because none of the `POST` handling here
//! blocks on it.
//!
//! # Authorization
//!
//! An MCP tool call is authorized exactly like the HTTP record routes it
//! is a thin restatement of: the caller's bearer token resolves through
//! [`crate::extract::Auth`] like any other request, and every tool
//! handler below calls straight into `crate::routes::records`'s
//! `list`/`view`/`create_record`/`update_record`/`delete_record` — the
//! same functions `routes::records::router()` wires to the HTTP verbs —
//! so a `listRule`/`viewRule`/`createRule`/`updateRule`/`deleteRule` that
//! would reject an HTTP caller rejects the equivalent tool call
//! identically, through the identical code path. There is no separate
//! permission system to keep in sync.
//!
//! Only non-system collections (name not starting with `_`) are exposed
//! as tools; `_collections`, `_superusers` and friends stay reachable
//! only through the superuser-gated admin API, never through MCP.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{FromRequest, Path, Request, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use cratebase_core::Collection;
use futures::stream::{self, Stream};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::extract::{Auth, MaybeAuth, RequestInfo};
use crate::http_error::ApiError;
use crate::routes::records::{create_record, delete_record, list, update_record, view};
use crate::routes::records::{ListQuery, SingleQuery};

pub fn router() -> Router<App> {
    Router::new().route("/mcp", get(sse_handshake).post(rpc))
}

// ------------------------------------------------------------------ SSE

/// The spec's `endpoint` handshake event, then idle keep-alive comments.
/// See the module doc's "Transport" section for why nothing else is ever
/// pushed down this stream.
async fn sse_handshake() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let handshake = stream::once(async { Ok(Event::default().event("endpoint").data("/api/mcp")) });
    Sse::new(handshake).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(25))
            .text("keep-alive"),
    )
}

// -------------------------------------------------------------- JSON-RPC

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

/// A JSON-RPC protocol-level error (bad method, bad params, malformed
/// tool name) — distinct from a *tool* failing, which is reported inside
/// a normal `tools/call` result with `isError: true` (see
/// [`call_tool`]), matching MCP's convention that an authorization
/// denial or a validation failure is a tool outcome, not a transport
/// fault.
struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn invalid_params(message: impl Into<String>) -> Self {
        RpcError {
            code: -32602,
            message: message.into(),
        }
    }
    fn method_not_found(message: impl Into<String>) -> Self {
        RpcError {
            code: -32601,
            message: message.into(),
        }
    }
}

async fn rpc(State(app): State<App>, MaybeAuth(auth): MaybeAuth, request: Request) -> Response {
    let bytes = match axum::body::Bytes::from_request(request, &app).await {
        Ok(b) => b,
        Err(_) => return rpc_error_response(None, -32700, "Parse error"),
    };
    let req: RpcRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(_) => return rpc_error_response(None, -32700, "Parse error"),
    };

    // A request with no `id` is a JSON-RPC *notification* — e.g. the
    // `notifications/initialized` the spec has a client send after
    // `initialize`. Notifications get no reply; `202 Accepted` with an
    // empty body just acknowledges receipt over an HTTP transport that
    // requires *some* response.
    let Some(id) = req.id.clone() else {
        return StatusCode::ACCEPTED.into_response();
    };

    match dispatch(&app, auth, &req.method, req.params).await {
        Ok(result) => Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response(),
        Err(err) => rpc_error_response(Some(id), err.code, &err.message),
    }
}

fn rpc_error_response(id: Option<Value>, code: i64, message: &str) -> Response {
    Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    }))
    .into_response()
}

async fn dispatch(
    app: &App,
    auth: Option<Auth>,
    method: &str,
    params: Value,
) -> Result<Value, RpcError> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "cratebase", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": build_tools(app) })),
        "tools/call" => call_tool(app, auth, params).await,
        other => Err(RpcError::method_not_found(format!(
            "Unknown method: {other}"
        ))),
    }
}

// ------------------------------------------------------------------ tools

/// The five operations a non-view collection is exposed as; a view
/// collection (no writes, see `routes::records::UNSUPPORTED_TYPE`) only
/// gets the first two.
#[derive(Clone, Copy)]
enum Op {
    List,
    Get,
    Create,
    Update,
    Delete,
}

const WRITABLE_OPS: [Op; 5] = [Op::List, Op::Get, Op::Create, Op::Update, Op::Delete];
const READONLY_OPS: [Op; 2] = [Op::List, Op::Get];

impl Op {
    fn prefix(self) -> &'static str {
        match self {
            Op::List => "list",
            Op::Get => "get",
            Op::Create => "create",
            Op::Update => "update",
            Op::Delete => "delete",
        }
    }
}

fn tool_name(op: Op, collection: &str) -> String {
    format!("{}_{collection}", op.prefix())
}

/// Non-system collections, in the same order `_collections` lists them.
/// Recomputed per request (never cached) so a schema change or a new
/// collection is reflected on the very next `tools/list`/`tools/call`.
fn exposed_collections(app: &App) -> Vec<Arc<Collection>> {
    app.db()
        .collections
        .all()
        .all
        .iter()
        .filter(|c| !c.name.starts_with('_'))
        .cloned()
        .collect()
}

/// One tool's `{name, description, inputSchema}` entry.
fn tool_entry(op: Op, collection: &Collection) -> Value {
    let schema = collection.to_json_schema();
    let create_params = schema["parameters"].clone();
    let input_schema = match op {
        Op::List => json!({
            "type": "object",
            "properties": {
                "page": { "type": "integer", "description": "1-based page number." },
                "perPage": { "type": "integer", "description": "Items per page." },
                "filter": { "type": "string", "description": "Filter expression, e.g. \"status = 'open'\"." },
                "sort": { "type": "string", "description": "Sort expression, e.g. '-created'." },
                "expand": { "type": "string", "description": "Comma-separated relation expand paths." },
            },
            "required": [],
        }),
        Op::Get => json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Record id." },
                "expand": { "type": "string", "description": "Comma-separated relation expand paths." },
            },
            "required": ["id"],
        }),
        Op::Create => create_params,
        Op::Update => {
            let mut params = create_params;
            if let Value::Object(props) = &mut params["properties"] {
                props.insert(
                    "id".into(),
                    json!({ "type": "string", "description": "Record id." }),
                );
            }
            params["required"] = json!(["id"]);
            params
        }
        Op::Delete => json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "Record id." } },
            "required": ["id"],
        }),
    };
    let description = match op {
        Op::List => format!(
            "List/search records in the '{}' collection.",
            collection.name
        ),
        Op::Get => format!(
            "Fetch one record by id from the '{}' collection.",
            collection.name
        ),
        Op::Create => schema["description"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        Op::Update => format!("Update one record in the '{}' collection.", collection.name),
        Op::Delete => format!(
            "Delete one record from the '{}' collection.",
            collection.name
        ),
    };
    json!({
        "name": tool_name(op, &collection.name),
        "description": description,
        "inputSchema": input_schema,
    })
}

fn build_tools(app: &App) -> Vec<Value> {
    let mut tools = Vec::new();
    for collection in exposed_collections(app) {
        let ops: &[Op] = if collection.is_view() {
            &READONLY_OPS
        } else {
            &WRITABLE_OPS
        };
        for &op in ops {
            tools.push(tool_entry(op, &collection));
        }
    }
    tools
}

/// Match a `tools/call` name back to the `(op, collection)` it names.
/// Resolved by regenerating the same names [`build_tools`] would, rather
/// than parsing the tool name apart — collection names may themselves
/// contain underscores, so `list_a_b` is ambiguous to split but not to
/// look up.
fn resolve_tool(app: &App, name: &str) -> Option<(Op, Arc<Collection>)> {
    for collection in exposed_collections(app) {
        let ops: &[Op] = if collection.is_view() {
            &READONLY_OPS
        } else {
            &WRITABLE_OPS
        };
        for &op in ops {
            if tool_name(op, &collection.name) == name {
                return Some((op, collection));
            }
        }
    }
    None
}

// --------------------------------------------------------------- tools/call

async fn call_tool(app: &App, auth: Option<Auth>, params: Value) -> Result<Value, RpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params("Missing required 'name' argument."))?;
    let arguments = match params.get("arguments") {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };

    let (op, collection) = resolve_tool(app, name)
        .ok_or_else(|| RpcError::method_not_found(format!("Unknown tool: {name}")))?;

    let outcome = match op {
        Op::List => call_list(app, auth, &collection.name, &arguments).await,
        Op::Get => call_get(app, auth, &collection.name, &arguments).await,
        Op::Create => call_create(app, auth, &collection.name, arguments).await,
        Op::Update => call_update(app, auth, &collection.name, arguments).await,
        Op::Delete => call_delete(app, auth, &collection.name, &arguments).await,
    };

    Ok(match outcome {
        Ok(value) => json!({
            "content": [{ "type": "text", "text": serde_json::to_string(&value).unwrap_or_default() }],
            "structuredContent": value,
            "isError": false,
        }),
        Err(err) => {
            let body = err.error.body();
            json!({
                "content": [{ "type": "text", "text": body.message }],
                "isError": true,
            })
        }
    })
}

/// `@request.*` for a tool call: the same caller [`RequestInfo`] any
/// HTTP request carries, tagged with the `mcp` context (mirrors
/// `routes::batch`'s `"batch"` and `routes::files`'s `"protectedFile"`)
/// so a rule can distinguish an MCP-originated write if it needs to.
fn mcp_request_info(auth: Option<Auth>, method: &str, path: String) -> RequestInfo {
    RequestInfo {
        method: method.to_string(),
        path,
        query: Map::new(),
        headers: Map::new(),
        body: Map::new(),
        context: "mcp".to_string(),
        auth,
    }
}

fn json_request(method: axum::http::Method, uri: String, body: &Value) -> Request {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap_or_default()))
        .expect("well-formed synthetic MCP request")
}

fn get_arg<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

async fn call_list(
    app: &App,
    auth: Option<Auth>,
    collection: &str,
    args: &Map<String, Value>,
) -> Result<Value, ApiError> {
    let query = ListQuery {
        page: args.get("page").and_then(Value::as_i64),
        per_page: args.get("perPage").and_then(Value::as_i64),
        sort: get_arg(args, "sort").map(String::from),
        filter: get_arg(args, "filter").map(String::from),
        expand: get_arg(args, "expand").map(String::from),
        fields: None,
        skip_total: None,
        nearest_to: None,
        nearest_limit: None,
    };
    let info = mcp_request_info(
        auth,
        "GET",
        format!("/api/collections/{collection}/records"),
    );
    let Json(page) = list(
        State(app.clone()),
        Path(collection.to_string()),
        crate::http_error::ApiQuery(query),
        info,
    )
    .await?;
    serde_json::to_value(page)
        .map_err(|e| ApiError(cratebase_core::AppError::internal(e.to_string())))
}

async fn call_get(
    app: &App,
    auth: Option<Auth>,
    collection: &str,
    args: &Map<String, Value>,
) -> Result<Value, ApiError> {
    let id = get_arg(args, "id")
        .ok_or_else(|| {
            ApiError(cratebase_core::AppError::bad_request(
                "Missing required 'id' argument.",
            ))
        })?
        .to_string();
    let query = SingleQuery {
        expand: get_arg(args, "expand").map(String::from),
        fields: None,
    };
    let info = mcp_request_info(
        auth,
        "GET",
        format!("/api/collections/{collection}/records/{id}"),
    );
    let Json(value) = view(
        State(app.clone()),
        Path((collection.to_string(), id)),
        crate::http_error::ApiQuery(query),
        info,
    )
    .await?;
    Ok(value)
}

async fn call_create(
    app: &App,
    auth: Option<Auth>,
    collection: &str,
    args: Map<String, Value>,
) -> Result<Value, ApiError> {
    let path = format!("/api/collections/{collection}/records");
    let info = mcp_request_info(auth, "POST", path.clone());
    let request = json_request(axum::http::Method::POST, path, &Value::Object(args));
    let Json(value) = create_record(
        State(app.clone()),
        Path(collection.to_string()),
        crate::http_error::ApiQuery(SingleQuery::default()),
        info,
        request,
    )
    .await?;
    Ok(value)
}

async fn call_update(
    app: &App,
    auth: Option<Auth>,
    collection: &str,
    mut args: Map<String, Value>,
) -> Result<Value, ApiError> {
    let id = args
        .remove("id")
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| {
            ApiError(cratebase_core::AppError::bad_request(
                "Missing required 'id' argument.",
            ))
        })?;
    let path = format!("/api/collections/{collection}/records/{id}");
    let info = mcp_request_info(auth, "PATCH", path.clone());
    let request = json_request(axum::http::Method::PATCH, path, &Value::Object(args));
    let Json(value) = update_record(
        State(app.clone()),
        Path((collection.to_string(), id)),
        crate::http_error::ApiQuery(SingleQuery::default()),
        info,
        request,
    )
    .await?;
    Ok(value)
}

async fn call_delete(
    app: &App,
    auth: Option<Auth>,
    collection: &str,
    args: &Map<String, Value>,
) -> Result<Value, ApiError> {
    let id = get_arg(args, "id")
        .ok_or_else(|| {
            ApiError(cratebase_core::AppError::bad_request(
                "Missing required 'id' argument.",
            ))
        })?
        .to_string();
    let info = mcp_request_info(
        auth,
        "DELETE",
        format!("/api/collections/{collection}/records/{id}"),
    );
    delete_record(
        State(app.clone()),
        Path((collection.to_string(), id.clone())),
        info,
    )
    .await?;
    Ok(json!({ "deleted": true, "id": id }))
}
