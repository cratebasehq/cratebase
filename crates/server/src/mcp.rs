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
use cratebase_db::engine::Executor;
use cratebase_db::{records, rules, AuthContext, CollectionResolver, RequestContext};
use futures::stream::{self, Stream};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::extract::{Auth, MaybeAuth, RequestInfo, RequireSuperuser};
use crate::http_error::{ApiError, ApiQuery};
use crate::routes::collections::redact_oauth2_secrets;
use crate::routes::records::{create_record, delete_record, list, update_record, view};
use crate::routes::records::{ListQuery, SingleQuery};
use crate::routes::{logs, mails, schema, sql_console};

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
        "tools/list" => Ok(json!({ "tools": build_tools(app, auth.as_ref()) })),
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

/// The `list_<collection>`/`list_email_templates`/`list_email_triggers`
/// input shape — one schema, reused by every "list" tool rather than
/// three copies of the same five properties drifting apart.
fn list_query_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "page": { "type": "integer", "description": "1-based page number." },
            "perPage": { "type": "integer", "description": "Items per page." },
            "filter": { "type": "string", "description": "Filter expression, e.g. \"status = 'open'\"." },
            "sort": { "type": "string", "description": "Sort expression, e.g. '-created'." },
            "expand": { "type": "string", "description": "Comma-separated relation expand paths." },
        },
        "required": [],
    })
}

/// `collection`'s own create schema, plus an optional `id`: the input
/// shape for an "upsert" tool (`upsert_email_template`/
/// `upsert_email_trigger`) that creates when `id` is omitted and updates
/// when it's given — see [`call_upsert`].
fn upsert_input_schema(collection: &Collection) -> Value {
    let schema = collection.to_json_schema();
    let mut params = schema["parameters"].clone();
    if let Value::Object(props) = &mut params["properties"] {
        props.insert(
            "id".into(),
            json!({
                "type": "string",
                "description": "Existing record id to update. Omit to create a new row.",
            }),
        );
    }
    params
}

/// One tool's `{name, description, inputSchema}` entry.
fn tool_entry(op: Op, collection: &Collection) -> Value {
    let schema = collection.to_json_schema();
    let create_params = schema["parameters"].clone();
    let input_schema = match op {
        Op::List => list_query_schema(),
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

fn build_tools(app: &App, auth: Option<&Auth>) -> Vec<Value> {
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
    tools.extend(named_tool_entries(
        app,
        auth.is_some_and(|a| a.is_superuser),
    ));
    tools
}

// ------------------------------------------------------ fixed/named tools

/// The tools that aren't a per-collection `<op>_<collection>` restatement
/// of `routes::records`: three runtime tools any caller can see (each
/// still enforced by the same rule/`sendRule` machinery its HTTP
/// equivalent uses), and nine developer tools visible only to a
/// superuser caller — see the module doc for the auth model this
/// mirrors.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NamedTool {
    CallRpc,
    SendEmail,
    SearchNearby,
    GetSchema,
    ApplySchema,
    TestRule,
    ListEmailTemplates,
    UpsertEmailTemplate,
    ListEmailTriggers,
    UpsertEmailTrigger,
    QueryLogs,
    SqlRead,
}

impl NamedTool {
    const ALL: [NamedTool; 12] = [
        NamedTool::CallRpc,
        NamedTool::SendEmail,
        NamedTool::SearchNearby,
        NamedTool::GetSchema,
        NamedTool::ApplySchema,
        NamedTool::TestRule,
        NamedTool::ListEmailTemplates,
        NamedTool::UpsertEmailTemplate,
        NamedTool::ListEmailTriggers,
        NamedTool::UpsertEmailTrigger,
        NamedTool::QueryLogs,
        NamedTool::SqlRead,
    ];

    fn name(self) -> &'static str {
        match self {
            NamedTool::CallRpc => "call_rpc",
            NamedTool::SendEmail => "send_email",
            NamedTool::SearchNearby => "search_nearby",
            NamedTool::GetSchema => "get_schema",
            NamedTool::ApplySchema => "apply_schema",
            NamedTool::TestRule => "test_rule",
            NamedTool::ListEmailTemplates => "list_email_templates",
            NamedTool::UpsertEmailTemplate => "upsert_email_template",
            NamedTool::ListEmailTriggers => "list_email_triggers",
            NamedTool::UpsertEmailTrigger => "upsert_email_trigger",
            NamedTool::QueryLogs => "query_logs",
            NamedTool::SqlRead => "sql_read",
        }
    }

    /// A developer tool reuses superuser-only server machinery directly
    /// (schema apply, raw SQL, rule testing, log queries, system
    /// collection writes) — it is not merely refused for a non-superuser
    /// caller, it is absent from `tools/list` *and* unresolvable by name
    /// in `tools/call`, so nothing here confirms these tools even exist
    /// to a caller who can't use them.
    fn is_dev_only(self) -> bool {
        !matches!(
            self,
            NamedTool::CallRpc | NamedTool::SendEmail | NamedTool::SearchNearby
        )
    }
}

/// Reverse of [`NamedTool::name`], for `tools/call`.
fn resolve_named_tool(name: &str) -> Option<NamedTool> {
    NamedTool::ALL.into_iter().find(|t| t.name() == name)
}

fn named_tool_entries(app: &App, is_superuser: bool) -> Vec<Value> {
    NamedTool::ALL
        .into_iter()
        .filter(|t| is_superuser || !t.is_dev_only())
        .map(|t| named_tool_entry(t, app))
        .collect()
}

fn named_tool_entry(tool: NamedTool, app: &App) -> Value {
    let (description, input_schema): (String, Value) = match tool {
        NamedTool::CallRpc => (
            "Call a saved custom SQL RPC (the `_rpc` collection) by name. Enforced by that \
             RPC's own `rule`, exactly like POST /api/rpc/{name} — a caller the rule rejects \
             gets the same denial an HTTP call would."
                .into(),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "The _rpc definition's name." },
                    "params": {
                        "type": "object",
                        "description": "Named parameters bound to the RPC's SQL.",
                        "additionalProperties": true,
                    },
                },
                "required": ["name"],
            }),
        ),
        NamedTool::SendEmail => (
            "Send an email through a named _emailTemplates template. Enforced through the \
             same sendRule logic as POST /api/mails/send: a superuser/API-key caller may send \
             anything the template allows; any other caller may only reach a template whose \
             sendRule allows the given `to` address, capped at 5 recipients."
                .into(),
            json!({
                "type": "object",
                "properties": {
                    "template": { "type": "string", "description": "An _emailTemplates.key." },
                    "to": {
                        "description": "A recipient address, {address, name}, or an array of either.",
                    },
                    "data": {
                        "type": "object",
                        "description": "Template data, merged into the standard {{var}} context.",
                        "additionalProperties": true,
                    },
                    "locale": { "type": "string" },
                },
                "required": ["template", "to"],
            }),
        ),
        NamedTool::SearchNearby => (
            "List records in `collection` within `radiusKm` of (lon, lat), nearest first — \
             the geoDistance filter/sort path (PostGIS-accelerated when available). Enforced \
             by that collection's own listRule, exactly like list_<collection>."
                .into(),
            json!({
                "type": "object",
                "properties": {
                    "collection": { "type": "string" },
                    "field": { "type": "string", "description": "The geoPoint field to measure from." },
                    "lon": { "type": "number" },
                    "lat": { "type": "number" },
                    "radiusKm": { "type": "number" },
                    "limit": { "type": "integer", "description": "Max rows (perPage). Defaults to 20." },
                },
                "required": ["collection", "field", "lon", "lat", "radiusKm"],
            }),
        ),
        NamedTool::GetSchema => (
            "Superuser only. Return every collection's schema (fields, rules, indexes); \
             OAuth2 client secrets and Apple private keys are redacted."
                .into(),
            json!({ "type": "object", "properties": {}, "required": [] }),
        ),
        NamedTool::ApplySchema => (
            "Superuser only. Diff `collections` against the live schema and, unless `dryRun` \
             is false, apply it — added/changed fields write unconditionally, a removed field \
             is only dropped when `force` is true. `dryRun` defaults to true, so a call never \
             writes unless explicitly told to."
                .into(),
            json!({
                "type": "object",
                "properties": {
                    "collections": {
                        "description": "An array of collection objects, or {collections:[...]}/{items:[...]} — the same shape GET /api/collections returns.",
                    },
                    "dryRun": {
                        "type": "boolean",
                        "description": "Compute and return the plan without writing. Defaults to true.",
                    },
                    "force": {
                        "type": "boolean",
                        "description": "Actually drop fields the payload omits. Defaults to false.",
                    },
                },
                "required": ["collections"],
            }),
        ),
        NamedTool::TestRule => (
            "Superuser only. Evaluate a filter-rule expression as `collection`'s own rule \
             would be, for the given auth record (or anonymous when omitted) and an optional \
             row, returning allow/deny and the compiled SQL when the rule compiles without a \
             database round trip."
                .into(),
            json!({
                "type": "object",
                "properties": {
                    "rule": {
                        "type": "string",
                        "description": "A filter-rule expression, e.g. \"owner = @request.auth.id\".",
                    },
                    "collection": {
                        "type": "string",
                        "description": "The collection whose schema the rule is resolved against.",
                    },
                    "authCollection": { "type": "string", "description": "Defaults to 'users'." },
                    "authRecordId": {
                        "type": "string",
                        "description": "Omit to test as an anonymous caller.",
                    },
                    "record": {
                        "type": "object",
                        "description": "The row to test the rule against. Omit to test it as a createRule against an empty body.",
                        "additionalProperties": true,
                    },
                },
                "required": ["rule", "collection"],
            }),
        ),
        NamedTool::ListEmailTemplates => (
            "Superuser only. List/search rows in the _emailTemplates system collection.".into(),
            list_query_schema(),
        ),
        NamedTool::UpsertEmailTemplate => (
            "Superuser only. Create or update an _emailTemplates row: pass `id` to update, \
             omit it to create."
                .into(),
            email_collection_schema(app, "_emailTemplates"),
        ),
        NamedTool::ListEmailTriggers => (
            "Superuser only. List/search rows in the _emailTriggers system collection.".into(),
            list_query_schema(),
        ),
        NamedTool::UpsertEmailTrigger => (
            "Superuser only. Create or update an _emailTriggers row: pass `id` to update, \
             omit it to create."
                .into(),
            email_collection_schema(app, "_emailTriggers"),
        ),
        NamedTool::QueryLogs => (
            "Superuser only. Query request/application logs (_logs) — the same filter \
             grammar and pagination as GET /api/logs."
                .into(),
            json!({
                "type": "object",
                "properties": {
                    "page": { "type": "integer" },
                    "perPage": { "type": "integer" },
                    "filter": {
                        "type": "string",
                        "description": "e.g. \"level >= 8 && data.method = 'GET'\".",
                    },
                    "sort": { "type": "string" },
                },
                "required": [],
            }),
        ),
        NamedTool::SqlRead => (
            format!(
                "Superuser only. Run a read-only SQL statement (must start with SELECT or \
                 WITH) via the SQL console's read path — capped at {} rows, {}s timeout.",
                sql_console::ROW_CAP,
                sql_console::QUERY_TIMEOUT.as_secs(),
            ),
            json!({
                "type": "object",
                "properties": { "sql": { "type": "string" } },
                "required": ["sql"],
            }),
        ),
    };
    json!({
        "name": tool.name(),
        "description": description,
        "inputSchema": input_schema,
    })
}

/// [`upsert_input_schema`] for a system collection that may not exist
/// yet (a database bootstrapped before this migration landed) — falls
/// back to an empty-but-valid schema rather than panicking.
fn email_collection_schema(app: &App, name: &str) -> Value {
    app.db()
        .collections
        .get_by_name(name)
        .map(|c| upsert_input_schema(&c))
        .unwrap_or_else(|| json!({ "type": "object", "properties": {}, "required": [] }))
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

    if let Some(tool) = resolve_named_tool(name) {
        let is_superuser = auth.as_ref().is_some_and(|a| a.is_superuser);
        if tool.is_dev_only() && !is_superuser {
            // Hidden from tools/list for this caller — a direct call by
            // name is refused identically to a tool that doesn't exist,
            // so nothing here confirms it does.
            return Err(RpcError::method_not_found(format!("Unknown tool: {name}")));
        }
        return Ok(tool_result(
            call_named_tool(app, auth, tool, arguments).await,
        ));
    }

    let (op, collection) = resolve_tool(app, name)
        .ok_or_else(|| RpcError::method_not_found(format!("Unknown tool: {name}")))?;

    let outcome = match op {
        Op::List => call_list(app, auth, &collection.name, &arguments).await,
        Op::Get => call_get(app, auth, &collection.name, &arguments).await,
        Op::Create => call_create(app, auth, &collection.name, arguments).await,
        Op::Update => call_update(app, auth, &collection.name, arguments).await,
        Op::Delete => call_delete(app, auth, &collection.name, &arguments).await,
    };

    Ok(tool_result(outcome))
}

/// The shared `tools/call` success/`isError` envelope, for both a
/// per-collection tool and a [`NamedTool`].
fn tool_result(outcome: Result<Value, ApiError>) -> Value {
    match outcome {
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
    }
}

/// Dispatches one [`NamedTool`] call. Every runtime tool
/// (`call_rpc`/`send_email`/`search_nearby`) reuses the exact server-side
/// function its HTTP equivalent calls — see each helper's own doc.
async fn call_named_tool(
    app: &App,
    auth: Option<Auth>,
    tool: NamedTool,
    args: Map<String, Value>,
) -> Result<Value, ApiError> {
    match tool {
        NamedTool::CallRpc => call_rpc_tool(app, auth, &args).await,
        NamedTool::SendEmail => call_send_email(app, auth, &args).await,
        NamedTool::SearchNearby => call_search_nearby(app, auth, &args).await,
        NamedTool::GetSchema => dev_get_schema(app).await,
        NamedTool::ApplySchema => dev_apply_schema(app, auth, args).await,
        NamedTool::TestRule => dev_test_rule(app, &args).await,
        NamedTool::ListEmailTemplates => call_list(app, auth, "_emailTemplates", &args).await,
        NamedTool::UpsertEmailTemplate => call_upsert(app, auth, "_emailTemplates", args).await,
        NamedTool::ListEmailTriggers => call_list(app, auth, "_emailTriggers", &args).await,
        NamedTool::UpsertEmailTrigger => call_upsert(app, auth, "_emailTriggers", args).await,
        NamedTool::QueryLogs => dev_query_logs(app, auth, &args).await,
        NamedTool::SqlRead => dev_sql_read(app, &args).await,
    }
}

/// `id` present and non-empty → update; otherwise → create. Shared by
/// `upsert_email_template`/`upsert_email_trigger`, reusing [`call_update`]
/// / [`call_create`] verbatim — the actual record write is identical to
/// a plain `create_<collection>`/`update_<collection>` tool call, just
/// under one name that picks the operation from the arguments.
async fn call_upsert(
    app: &App,
    auth: Option<Auth>,
    collection: &str,
    args: Map<String, Value>,
) -> Result<Value, ApiError> {
    let has_id = args
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    if has_id {
        call_update(app, auth, collection, args).await
    } else {
        call_create(app, auth, collection, args).await
    }
}

/// `call_rpc`: reuses [`crate::rpc::call_rpc`] verbatim (the same
/// `_rpc.rule` enforcement `POST /api/rpc/{name}` gets), by building the
/// same synthetic [`RequestInfo`] + JSON body every other tool here
/// builds for the record routes it restates.
async fn call_rpc_tool(
    app: &App,
    auth: Option<Auth>,
    args: &Map<String, Value>,
) -> Result<Value, ApiError> {
    let name = get_arg(args, "name")
        .ok_or_else(|| ApiError::bad_request("Missing required 'name' argument."))?
        .to_string();
    let params = match args.get("params") {
        Some(Value::Object(m)) => Value::Object(m.clone()),
        _ => Value::Object(Map::new()),
    };
    let path = format!("/api/rpc/{name}");
    let info = mcp_request_info(app, auth, "POST", path);
    let body = axum::body::Bytes::from(serde_json::to_vec(&params).unwrap_or_default());
    let Json(value) = crate::rpc::call_rpc(State(app.clone()), Path(name), info, body).await?;
    Ok(value)
}

/// `send_email`: reuses [`mails::send_mail`] verbatim — the same
/// `sendRule` gate `POST /api/mails/send` enforces for a non-superuser
/// caller, never duplicated here.
async fn call_send_email(
    app: &App,
    auth: Option<Auth>,
    args: &Map<String, Value>,
) -> Result<Value, ApiError> {
    let template = get_arg(args, "template")
        .ok_or_else(|| ApiError::bad_request("Missing required 'template' argument."))?
        .to_string();
    let body = mails::MailBody {
        to: args.get("to").cloned().unwrap_or_default(),
        template: Some(template),
        locale: get_arg(args, "locale").map(String::from),
        data: args.get("data").cloned().unwrap_or_default(),
        ..Default::default()
    };
    let outcome = mails::send_mail(app, auth.as_ref(), body).await?;
    serde_json::to_value(outcome).map_err(|e| ApiError::internal(e.to_string()))
}

/// `search_nearby`: no new query machinery — it's [`call_list`] (so
/// `collection`'s own `listRule` applies exactly as it does for
/// `list_<collection>`) with a `geoDistance(...)` filter/sort built from
/// the tool's arguments, the same string a client would pass directly.
async fn call_search_nearby(
    app: &App,
    auth: Option<Auth>,
    args: &Map<String, Value>,
) -> Result<Value, ApiError> {
    let collection_name = get_arg(args, "collection")
        .ok_or_else(|| ApiError::bad_request("Missing required 'collection' argument."))?;
    let exposed = app
        .db()
        .collections
        .get_by_name(collection_name)
        .map(|c| !c.name.starts_with('_'))
        .unwrap_or(false);
    if !exposed {
        return Err(ApiError::bad_request(format!(
            "Unknown collection '{collection_name}'."
        )));
    }
    let field = get_arg(args, "field")
        .ok_or_else(|| ApiError::bad_request("Missing required 'field' argument."))?;
    let lon = args
        .get("lon")
        .and_then(Value::as_f64)
        .ok_or_else(|| ApiError::bad_request("Missing required numeric 'lon' argument."))?;
    let lat = args
        .get("lat")
        .and_then(Value::as_f64)
        .ok_or_else(|| ApiError::bad_request("Missing required numeric 'lat' argument."))?;
    let radius_km = args
        .get("radiusKm")
        .and_then(Value::as_f64)
        .ok_or_else(|| ApiError::bad_request("Missing required numeric 'radiusKm' argument."))?;

    let mut list_args = Map::new();
    list_args.insert(
        "filter".into(),
        Value::String(format!(
            "geoDistance({field}.lon, {field}.lat, {lon}, {lat}) < {radius_km}"
        )),
    );
    list_args.insert(
        "sort".into(),
        Value::String(format!(
            "geoDistance({field}.lon, {field}.lat, {lon}, {lat})"
        )),
    );
    if let Some(limit) = args.get("limit").and_then(Value::as_i64) {
        list_args.insert("perPage".into(), Value::from(limit));
    }
    call_list(app, auth, collection_name, &list_args).await
}

/// `get_schema`: every collection's schema, redacted the same way
/// `GET /api/collections` redacts OAuth2 secrets — reuses
/// [`redact_oauth2_secrets`] rather than a second copy of that logic.
async fn dev_get_schema(app: &App) -> Result<Value, ApiError> {
    let snapshot = app.db().collections.all();
    let collections: Vec<Value> = snapshot
        .all
        .iter()
        .map(|c| redact_oauth2_secrets(c.to_json()))
        .collect();
    Ok(json!({ "collections": collections }))
}

/// `apply_schema`: reuses [`schema::plan_and_apply`] verbatim — the same
/// diff-first, `dryRun`-by-default apply `POST /api/schema/apply` runs,
/// just fed from tool arguments instead of the HTTP body/query string.
async fn dev_apply_schema(
    app: &App,
    auth: Option<Auth>,
    mut args: Map<String, Value>,
) -> Result<Value, ApiError> {
    let dry_run = args
        .remove("dryRun")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let force = args
        .remove("force")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let body = args
        .remove("collections")
        .ok_or_else(|| ApiError::bad_request("Missing required 'collections' argument."))?;
    let info = mcp_request_info(app, auth.clone(), "POST", "/api/schema/apply".to_string());
    let diff = schema::plan_and_apply(app, &body, dry_run, force, &info, auth).await?;
    serde_json::to_value(diff).map_err(|e| ApiError::internal(e.to_string()))
}

/// `test_rule`: builds the same [`RequestContext`]/[`CollectionResolver`]
/// pair every rule check in this codebase evaluates against (see
/// `crate::mail_templates::eval_send_rule` for the same recipe), then
/// asks [`rules::check_rule_against_row`] for the real allow/deny and
/// [`cratebase_filter::parse_and_compile`] for the compiled SQL when that
/// compiles without needing a database round trip.
async fn dev_test_rule(app: &App, args: &Map<String, Value>) -> Result<Value, ApiError> {
    let rule = get_arg(args, "rule")
        .ok_or_else(|| ApiError::bad_request("Missing required 'rule' argument."))?
        .to_string();
    let collection_name = get_arg(args, "collection")
        .ok_or_else(|| ApiError::bad_request("Missing required 'collection' argument."))?;
    let collection = app
        .db()
        .collections
        .get_by_name(collection_name)
        .ok_or_else(|| ApiError::bad_request(format!("Unknown collection '{collection_name}'.")))?;

    let auth_ctx = match get_arg(args, "authRecordId") {
        Some(id) => {
            let auth_collection_name = get_arg(args, "authCollection").unwrap_or("users");
            let auth_collection = app
                .db()
                .collections
                .get_by_name(auth_collection_name)
                .ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "Unknown auth collection '{auth_collection_name}'."
                    ))
                })?;
            let record = records::find_by_id_raw(app.db(), &auth_collection, id)
                .await
                .map_err(|_| {
                    ApiError::bad_request(format!(
                        "Auth record '{id}' not found in '{auth_collection_name}'."
                    ))
                })?;
            Some(AuthContext::new(record))
        }
        None => None,
    };

    let row: Map<String, Value> = match args.get("record") {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };

    let ctx = RequestContext {
        auth: auth_ctx,
        body: row.clone(),
        ..RequestContext::default()
    };
    let resolver =
        CollectionResolver::new(collection, &app.db().collections, &ctx, app.db().dialect());
    let rule_opt = Some(rule.clone());
    let allowed = rules::check_rule_against_row(app.db(), &resolver, &rule_opt, &row).await?;
    let compiled_sql = cratebase_filter::parse_and_compile(&rule, &resolver, 0)
        .ok()
        .map(|c| c.sql);

    Ok(json!({ "allowed": allowed, "compiledSql": compiled_sql }))
}

/// `query_logs`: reuses [`logs::list`] verbatim — the same filter
/// compilation and pagination `GET /api/logs` uses.
async fn dev_query_logs(
    app: &App,
    auth: Option<Auth>,
    args: &Map<String, Value>,
) -> Result<Value, ApiError> {
    let auth =
        auth.ok_or_else(|| ApiError::forbidden("Only superusers can perform this action."))?;
    let query = logs::ListQuery {
        page: args.get("page").and_then(Value::as_i64),
        per_page: args.get("perPage").and_then(Value::as_i64),
        filter: get_arg(args, "filter").map(String::from),
        sort: get_arg(args, "sort").map(String::from),
    };
    let Json(page) =
        logs::list(State(app.clone()), RequireSuperuser(auth), ApiQuery(query)).await?;
    serde_json::to_value(page).map_err(|e| ApiError::internal(e.to_string()))
}

/// `sql_read`: the SQL console's read path only — [`sql_console::is_read_statement`]
/// gates it to `SELECT`/`WITH`, [`sql_console::cappable_select`] enforces
/// the row cap at the source, and the query itself runs through the same
/// `query_interruptible` (`BEGIN READ ONLY` + a real statement timeout on
/// Postgres) `POST /api/sql` uses for a read. There is no write path
/// here at all — this tool cannot be asked to run one.
async fn dev_sql_read(app: &App, args: &Map<String, Value>) -> Result<Value, ApiError> {
    let sql = get_arg(args, "sql")
        .ok_or_else(|| ApiError::bad_request("Missing required 'sql' argument."))?;
    if sql.trim().is_empty() {
        return Err(ApiError::bad_request("sql must not be empty."));
    }
    if !sql_console::is_read_statement(sql) {
        return Err(ApiError::bad_request(
            "sql_read only accepts a statement starting with SELECT or WITH.",
        ));
    }
    let query_sql =
        sql_console::cappable_select(sql, sql_console::ROW_CAP).unwrap_or_else(|| sql.to_string());
    let rows = app
        .db()
        .query_interruptible(&query_sql, &[], sql_console::QUERY_TIMEOUT)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let columns: Vec<String> = rows
        .first()
        .map(|r| r.columns.iter().cloned().collect())
        .unwrap_or_default();
    let truncated = rows.len() > sql_console::ROW_CAP;
    let out_rows: Vec<Value> = rows
        .into_iter()
        .take(sql_console::ROW_CAP)
        .map(|r| Value::Object(sql_console::row_to_map(r)))
        .collect();
    Ok(json!({ "columns": columns, "rows": out_rows, "truncated": truncated }))
}

/// `@request.*` for a tool call: the same caller [`RequestInfo`] any
/// HTTP request carries, tagged with the `mcp` context (mirrors
/// `routes::batch`'s `"batch"` and `routes::files`'s `"protectedFile"`)
/// so a rule can distinguish an MCP-originated write if it needs to.
fn mcp_request_info(app: &App, auth: Option<Auth>, method: &str, path: String) -> RequestInfo {
    RequestInfo {
        method: method.to_string(),
        path,
        query: Map::new(),
        headers: Map::new(),
        body: Map::new(),
        context: "mcp".to_string(),
        auth,
        postgis_available: app.postgis_available(),
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
        app,
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
        app,
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
    let info = mcp_request_info(app, auth, "POST", path.clone());
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
    let info = mcp_request_info(app, auth, "PATCH", path.clone());
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
        app,
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
