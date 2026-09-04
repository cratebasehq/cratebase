//! An LLM provider trait ([`LlmProvider`]) behind [`POST /api/llm/chat`](crate::routes::llm),
//! two implementations ([`EchoProvider`], [`OpenAiCompatProvider`]), and
//! [`provider_from_settings`] — provider selection mirroring
//! `cratebase_mailer::Mailer::from_settings`'s exact shape: `enabled`
//! picks between a real, network-calling backend and a zero-config
//! fallback that always works. Where the mailer's fallback logs instead
//! of delivering, the LLM fallback ([`EchoProvider`]) *is* a real,
//! deterministic provider — it just never leaves the process, which is
//! also why it is what this module's own tests and the assignment's own
//! live verification run against instead of a paid API key.
//!
//! [`log_usage`] is the per-request token ledger: a best-effort insert
//! into the `_llm_usage` system collection (see
//! `cratebase_core::Collection::default_system_collections`), written
//! after a chat completes. "Best-effort" is load-bearing — a logging
//! failure must never fail the chat response the caller is waiting on,
//! so every error from this path is logged and swallowed, never
//! propagated.

use std::collections::VecDeque;
use std::pin::Pin;

use async_trait::async_trait;
use futures::stream::{self, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use cratebase_core::settings::Llm;

use crate::app::App;
use crate::extract::Auth;

/// One `{role, content}` turn, the wire shape `POST /api/llm/chat`
/// accepts and the shape every provider is called with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// A provider or transport failure. Kept separate from
/// `cratebase_core::AppError` so this module has no dependency on the
/// server's HTTP error mapping; `crate::routes::llm` converts at the
/// boundary.
#[derive(Debug)]
pub enum LlmError {
    /// The HTTP request to the provider itself failed (DNS, connect,
    /// timeout, a body that didn't decode).
    Http(reqwest::Error),
    /// The provider answered with a non-2xx status.
    Provider { status: u16, body: String },
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Http(e) => write!(f, "llm provider request failed: {e}"),
            LlmError::Provider { status, body } => {
                write!(f, "llm provider error ({status}): {body}")
            }
        }
    }
}

impl std::error::Error for LlmError {}

/// A chunk stream: each item is one incremental piece of the assistant's
/// reply, in order.
pub type ChatStream = Pin<Box<dyn Stream<Item = Result<String, LlmError>> + Send>>;

/// A backend that can turn a conversation into a streamed reply.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat_stream(&self, messages: Vec<ChatMessage>) -> Result<ChatStream, LlmError>;
}

/// The zero-config default: streams the last `user` message straight
/// back, one word at a time, with a small delay between words. No
/// network access, always available, fully deterministic — this is
/// exactly what makes it usable both as `smtp.enabled = false`'s
/// `LogBackend` analogue *and* as the harness this crate's own tests
/// (and the assignment's live verification) run the streaming and
/// persistence paths against.
pub struct EchoProvider;

/// Delay between words, long enough that a caller polling the SSE stream
/// can observe distinct arrival times rather than one blob.
const ECHO_WORD_DELAY: std::time::Duration = std::time::Duration::from_millis(20);

#[async_trait]
impl LlmProvider for EchoProvider {
    async fn chat_stream(&self, messages: Vec<ChatMessage>) -> Result<ChatStream, LlmError> {
        let last_user = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let words: Vec<String> = last_user.split_whitespace().map(str::to_string).collect();

        let stream = stream::unfold((words, 0usize), |(words, idx)| async move {
            if idx >= words.len() {
                return None;
            }
            if idx > 0 {
                tokio::time::sleep(ECHO_WORD_DELAY).await;
            }
            let mut chunk = words[idx].clone();
            if idx + 1 < words.len() {
                chunk.push(' ');
            }
            Some((Ok(chunk), (words, idx + 1)))
        });
        Ok(Box::pin(stream))
    }
}

/// An OpenAI-compatible `POST {baseUrl}/chat/completions` provider with
/// `stream: true` — works unmodified against OpenAI itself or a local
/// Ollama instance's OpenAI-compat endpoint (`http://localhost:11434/v1`),
/// since both speak the same SSE-framed `chat.completion.chunk` shape.
pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAiCompatProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        OpenAiCompatProvider {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }
}

/// One SSE-parsing cursor over a provider's streamed HTTP body: raw
/// bytes not yet split into frames, deltas already extracted from a
/// parsed frame but not yet handed to the caller, and whether the
/// provider's own `data: [DONE]` sentinel (or a closed body) has been
/// seen.
struct SseCursor {
    bytes: Pin<Box<dyn Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
    buf: Vec<u8>,
    pending: VecDeque<String>,
    done: bool,
}

/// One SSE frame (`data: ...` lines up to a blank line) → the `content`
/// deltas it carries, queued onto `pending`. A frame with no recognizable
/// delta (a role-only chunk, a `[DONE]` sentinel, an unparsable line) is
/// silently absorbed — same as the browser `EventSource` API does with a
/// frame it doesn't understand.
fn absorb_frame(frame: &[u8], pending: &mut VecDeque<String>, done: &mut bool) {
    for line in frame.split(|&b| b == b'\n') {
        let line = String::from_utf8_lossy(line);
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            *done = true;
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if let Some(delta) = parsed["choices"][0]["delta"]["content"].as_str() {
            if !delta.is_empty() {
                pending.push_back(delta.to_string());
            }
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatProvider {
    async fn chat_stream(&self, messages: Vec<ChatMessage>) -> Result<ChatStream, LlmError> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let payload = serde_json::json!({
            "model": self.model,
            "stream": true,
            "messages": messages
                .iter()
                .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
                .collect::<Vec<_>>(),
        });
        let mut request = self.client.post(&url).json(&payload);
        if !self.api_key.is_empty() {
            request = request.bearer_auth(&self.api_key);
        }
        let response = request.send().await.map_err(LlmError::Http)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(LlmError::Provider { status, body });
        }

        let cursor = SseCursor {
            bytes: Box::pin(response.bytes_stream()),
            buf: Vec::new(),
            pending: VecDeque::new(),
            done: false,
        };

        let stream = stream::unfold(cursor, |mut cursor| async move {
            loop {
                if let Some(delta) = cursor.pending.pop_front() {
                    return Some((Ok(delta), cursor));
                }
                if cursor.done {
                    return None;
                }
                // `\r` is stripped on arrival (below), so a frame
                // boundary is always exactly `\n\n` regardless of
                // whether the provider sent `\n\n` or `\r\n\r\n`.
                if let Some(pos) = cursor.buf.windows(2).position(|w| w == b"\n\n") {
                    let frame: Vec<u8> = cursor.buf.drain(..pos + 2).collect();
                    absorb_frame(&frame[..pos], &mut cursor.pending, &mut cursor.done);
                    continue;
                }
                match cursor.bytes.next().await {
                    Some(Ok(chunk)) => {
                        cursor
                            .buf
                            .extend(chunk.iter().copied().filter(|&b| b != b'\r'));
                    }
                    Some(Err(e)) => return Some((Err(LlmError::Http(e)), cursor)),
                    None => {
                        cursor.done = true;
                    }
                }
            }
        });
        Ok(Box::pin(stream))
    }
}

/// Builds the configured provider, PocketBase/mailer style:
/// `settings.enabled` picks [`OpenAiCompatProvider`] (or, for a
/// `provider` this build doesn't recognize, still the safe zero-config
/// fallback) over [`EchoProvider`] — exactly the role `smtp.enabled`
/// plays choosing between `SmtpBackend` and `LogBackend` in
/// `cratebase_mailer::Mailer::from_settings`.
pub fn provider_from_settings(settings: &Llm) -> Box<dyn LlmProvider> {
    if !settings.enabled {
        return Box::new(EchoProvider);
    }
    match settings.provider.as_str() {
        "openai" => Box::new(OpenAiCompatProvider::new(
            settings.base_url.clone(),
            settings.api_key.clone(),
            settings.model.clone(),
        )),
        _ => Box::new(EchoProvider),
    }
}

/// A rough, provider-agnostic token count: whitespace-separated words.
/// Real tokenizers are provider- and model-specific (and OpenAI's own
/// streaming API only reports exact usage when `stream_options` opts
/// in); a word count is the "simple" ledger the assignment asks for,
/// good enough to show relative cost between requests without pulling in
/// a tokenizer dependency for a number nothing enforces against yet.
pub fn count_tokens(text: &str) -> i64 {
    text.split_whitespace().count() as i64
}

/// Best-effort insert into `_llm_usage` after a chat completes. Never
/// fails the caller: a missing collection (a database migrated by an
/// older binary) or a write error is logged and dropped, exactly like
/// `crate::webhooks::deliver` swallows a delivery failure rather than
/// failing the record write that triggered it.
pub async fn log_usage(
    app: &App,
    auth: Option<&Auth>,
    model: &str,
    prompt_tokens: i64,
    completion_tokens: i64,
) {
    let Some(collection) = app.db().collections.get_by_name("_llm_usage") else {
        tracing::warn!("_llm_usage collection missing; dropping usage record");
        return;
    };

    let mut input = Map::new();
    if let Some(auth) = auth {
        input.insert("callerId".into(), Value::String(auth.id.clone()));
    }
    input.insert("model".into(), Value::String(model.to_string()));
    input.insert("promptTokens".into(), Value::from(prompt_tokens));
    input.insert("completionTokens".into(), Value::from(completion_tokens));

    let mut record = cratebase_db::records::from_body(collection.clone(), &input);
    if record.id().is_empty() {
        record.set_id(cratebase_core::record_id());
    }

    let result = app
        .run_scoped(true, move |tx| {
            crate::routes::records::write_record(
                tx,
                collection,
                record,
                None,
                crate::routes::records::Write::Create,
                Vec::new(),
            )
        })
        .await;
    if let Err(e) = result {
        tracing::warn!(error = %e, "failed to record llm usage");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn echo_provider_streams_last_user_message_word_by_word() {
        let messages = vec![
            ChatMessage {
                role: "system".into(),
                content: "ignored".into(),
            },
            ChatMessage {
                role: "user".into(),
                content: "hello there friend".into(),
            },
        ];
        let mut stream = EchoProvider.chat_stream(messages).await.unwrap();
        let mut chunks = Vec::new();
        while let Some(chunk) = stream.next().await {
            chunks.push(chunk.unwrap());
        }
        assert_eq!(chunks, vec!["hello ", "there ", "friend"]);
        assert_eq!(chunks.concat(), "hello there friend");
    }

    #[tokio::test]
    async fn echo_provider_ignores_a_trailing_assistant_message() {
        let messages = vec![
            ChatMessage {
                role: "user".into(),
                content: "first".into(),
            },
            ChatMessage {
                role: "assistant".into(),
                content: "reply".into(),
            },
        ];
        let mut stream = EchoProvider.chat_stream(messages).await.unwrap();
        let mut chunks = Vec::new();
        while let Some(chunk) = stream.next().await {
            chunks.push(chunk.unwrap());
        }
        assert_eq!(chunks.concat(), "first");
    }

    #[test]
    fn provider_from_settings_falls_back_to_echo() {
        let disabled = Llm {
            enabled: false,
            ..Llm::default()
        };
        // Just asserting this constructs without panicking; the trait
        // object gives us no further introspection, by design.
        let _: Box<dyn LlmProvider> = provider_from_settings(&disabled);

        let unknown_provider = Llm {
            enabled: true,
            provider: "does-not-exist".into(),
            ..Llm::default()
        };
        let _: Box<dyn LlmProvider> = provider_from_settings(&unknown_provider);
    }

    #[test]
    fn counts_words_as_tokens() {
        assert_eq!(count_tokens(""), 0);
        assert_eq!(count_tokens("one two three"), 3);
        assert_eq!(count_tokens("  extra   spaces  "), 2);
    }

    #[test]
    fn absorbs_a_multi_line_sse_frame() {
        let mut pending = VecDeque::new();
        let mut done = false;
        let frame = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\ndata: [DONE]";
        absorb_frame(frame, &mut pending, &mut done);
        assert_eq!(pending.pop_front(), Some("hi".to_string()));
        assert!(done);
    }
}
