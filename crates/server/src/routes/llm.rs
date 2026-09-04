//! `POST /api/llm/chat` — one chat completion against the provider
//! configured in `settings.llm` (see `crate::llm`).
//!
//! # Where the chunks actually go
//!
//! The HTTP response is the *final* result, returned once the provider's
//! stream ends — never the streamed body itself. The incremental chunks
//! go out over the caller's already-open `GET /api/realtime` SSE
//! connection instead (`crate::realtime::RealtimeService::send`), when
//! the request names that connection's `clientId`. This is the "existing
//! SSE realtime infrastructure" reuse the feature is built on: one
//! registry, one queue-per-client, one place that unregisters a dead
//! peer, shared with every other realtime event in this server rather
//! than a second bespoke SSE endpoint. A caller that omits `clientId`
//! still gets a normal, non-streamed response — it just isn't watching
//! anything meanwhile.
//!
//! Three realtime events, all under an `event:` name unique to this
//! feature so they never collide with a `<collectionKey>` record-event
//! subscription: `llm_chunk` (`{requestId, delta}`) per chunk,
//! `llm_done` (`{requestId, text}`) once, and `llm_error`
//! (`{requestId, message}`) instead of `llm_done` if the provider fails
//! mid-stream.
//!
//! # Persistence respects the target collection's own rules
//!
//! `collection` is optional. When set, the exchange is written there
//! *after* the stream completes, through the same `createRule` check
//! `POST /api/collections/{c}/records` runs (`precheck_create` below),
//! evaluated against this request's own auth context — a superuser
//! platform feature does not imply superuser-only write access to
//! whatever collection the caller names.
//!
//! The row itself is written with `crate::routes::records::write_record`
//! rather than going through the full `records::create_record` handler:
//! that skips only the outer `onRecordCreateRequest` hook (there is no
//! multipart/upload handling here to justify running it) while still
//! running `onRecordValidate`, `onRecordCreate(Execute)` and the
//! after-success/after-error hooks, so a bound JS hook on the target
//! collection still sees the write.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use cratebase_core::{ids, record_id, Collection, RecordAction, SerializeOptions};
use cratebase_db::context::CollectionResolver;
use cratebase_db::{records, rules};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::app::App;
use crate::extract::RequestInfo;
use crate::http_error::{rule_errors, ApiError, ApiResult};
use crate::llm::{self, ChatMessage, LlmError};
use crate::realtime;
use crate::routes::common;
use crate::routes::records::{read_error, write_record, Write};

pub fn router() -> Router<App> {
    Router::new().route("/llm/chat", post(chat))
}

/// A random-enough id for one chat call to tag its own realtime frames
/// with; a caller juggling more than one in-flight request tells them
/// apart by this, not by event name.
const REQUEST_ID_LEN: usize = 20;
const REQUEST_ID_ALPHABET: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

#[derive(Debug, Deserialize)]
struct WireMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatBody {
    messages: Vec<WireMessage>,
    #[serde(default)]
    collection: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatResponse {
    reply: String,
    prompt_tokens: i64,
    completion_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    record: Option<Value>,
}

async fn chat(
    State(app): State<App>,
    info: RequestInfo,
    Json(body): Json<ChatBody>,
) -> ApiResult<Json<ChatResponse>> {
    if body.messages.is_empty() {
        return Err(ApiError::bad_request("messages must not be empty."));
    }

    // Rule-check the target collection *before* spending a single token
    // on the provider: a denied write should fail immediately, not after
    // a full (for a real provider, costly) generation.
    let target = match &body.collection {
        Some(name) => Some(precheck_create(&app, name, &info).await?),
        None => None,
    };

    let messages: Vec<ChatMessage> = body
        .messages
        .iter()
        .map(|m| ChatMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        })
        .collect();

    let request_id = ids::random_string(REQUEST_ID_LEN, REQUEST_ID_ALPHABET);
    let provider = llm::provider_from_settings(&app.settings().llm);

    let mut stream = provider.chat_stream(messages.clone()).await.map_err(|e| {
        notify_error(&app, body.client_id.as_deref(), &request_id, &e);
        provider_error(e)
    })?;

    let mut reply = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                notify_error(&app, body.client_id.as_deref(), &request_id, &e);
                return Err(provider_error(e));
            }
        };
        if let Some(client_id) = &body.client_id {
            app.realtime().send(
                client_id,
                "llm_chunk",
                serde_json::json!({ "requestId": request_id, "delta": chunk }),
            );
        }
        reply.push_str(&chunk);
    }
    if let Some(client_id) = &body.client_id {
        app.realtime().send(
            client_id,
            "llm_done",
            serde_json::json!({ "requestId": request_id, "text": reply }),
        );
    }

    let prompt_text = messages
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let prompt_tokens = llm::count_tokens(&prompt_text);
    let completion_tokens = llm::count_tokens(&reply);

    let record = match (target, &body.collection) {
        (Some(collection), Some(_)) => {
            Some(persist_exchange(&app, collection, &prompt_text, &reply).await?)
        }
        _ => None,
    };

    let model = app.settings().llm.model.clone();
    llm::log_usage(
        &app,
        info.auth.as_ref(),
        &model,
        prompt_tokens,
        completion_tokens,
    )
    .await;

    Ok(Json(ChatResponse {
        reply,
        prompt_tokens,
        completion_tokens,
        record,
    }))
}

fn provider_error(e: LlmError) -> ApiError {
    match e {
        LlmError::Provider { status, body } => {
            ApiError::bad_request(format!("llm provider error ({status}): {body}"))
        }
        LlmError::Http(e) => ApiError::bad_request(format!("llm provider request failed: {e}")),
    }
}

fn notify_error(app: &App, client_id: Option<&str>, request_id: &str, e: &LlmError) {
    if let Some(client_id) = client_id {
        app.realtime().send(
            client_id,
            "llm_error",
            serde_json::json!({ "requestId": request_id, "message": e.to_string() }),
        );
    }
}

/// The `createRule` half of `POST /api/collections/{c}/records`
/// (`crate::routes::records::create_record`), reused as-is: a `null`
/// rule is superusers-only, otherwise the rule is evaluated against this
/// request's own auth context. Denying here is exactly as final as
/// denying there — this is not a second, weaker gate.
async fn precheck_create(app: &App, name: &str, info: &RequestInfo) -> ApiResult<Arc<Collection>> {
    let collection = common::collection_of(app, name)?;
    if collection.is_view() {
        return Err(ApiError::bad_request("Unsupported collection type."));
    }
    let ctx = info.to_context();
    if rules::is_superuser_only(&collection.create_rule, &ctx) {
        return Err(ApiError(rule_errors::superusers_only()));
    }
    let resolver = CollectionResolver::new(
        collection.clone(),
        &app.db().collections,
        &ctx,
        app.db().dialect(),
    );
    if !rules::check_create_rule(app.db(), &resolver, &collection.create_rule)
        .await
        .map_err(read_error)?
    {
        return Err(ApiError(rule_errors::create_denied()));
    }
    Ok(collection)
}

/// Write `{prompt, response, model}` into `collection`, honouring
/// whatever fields it actually declares (`records::from_body` silently
/// drops anything the schema doesn't recognize — a collection with no
/// `model` field just doesn't get one). Publishes the usual realtime
/// record-create event afterwards, same as a normal
/// `POST .../records` create.
async fn persist_exchange(
    app: &App,
    collection: Arc<Collection>,
    prompt: &str,
    reply: &str,
) -> ApiResult<Value> {
    let mut input = Map::new();
    input.insert("prompt".into(), Value::String(prompt.to_string()));
    input.insert("response".into(), Value::String(reply.to_string()));
    input.insert(
        "model".into(),
        Value::String(app.settings().llm.model.clone()),
    );

    let mut record = records::from_body(collection.clone(), &input);
    if record.id().is_empty() {
        record.set_id(record_id());
    }

    let saved = app
        .run_scoped(true, {
            let collection = collection.clone();
            move |tx| write_record(tx, collection, record, None, Write::Create, Vec::new())
        })
        .await
        .map_err(ApiError)?;

    realtime::publish(app, &collection, RecordAction::Create, &saved);
    Ok(saved.to_json(SerializeOptions::default()))
}
