//! Embedding providers for `vector` fields with an `embedding` config
//! (see [`cratebase_core::field::EmbeddingConfig`]), and the
//! application-side ranking that backs `?nearestTo=` on the records list
//! endpoint (see `crate::routes::records`).
//!
//! # v1 scope (deliberate)
//!
//! A vector field stores a plain JSON array of floats — there is no
//! native SQLite (`sqlite-vec`) or Postgres (`pgvector`) extension behind
//! it. Similarity search is a Rust-side cosine comparison run *after* the
//! normal rule-filtered fetch, not pushed into SQL. That is fine for
//! collections with up to a few thousand rows matching the list rule; it
//! is not a substitute for a real ANN index and was never meant to be —
//! adding one (via `sqlite-vec`/`pgvector` or an external index) is
//! explicit future work, not an oversight of this pass. See
//! [`crate::routes::records::nearest`] for the honest ceiling on how many
//! candidate rows it will fetch before giving up.
//!
//! # Providers
//!
//! * [`EchoProvider`] — a deterministic, network-free fake: it hashes the
//!   input text into a fixed-size float vector with no network call, so
//!   it is always available and is what this module's own tests (and a
//!   fresh install with no embedding provider configured) run against.
//!   Cosine similarity between two *different* echo-embedded texts is
//!   meaningless; the point is that the *same* text always embeds to the
//!   same vector, which is enough to exercise auto-embed-on-write and
//!   nearest-neighbor ranking end to end without a paid API key.
//! * [`HttpEmbeddingProvider`] — an OpenAI-compatible `POST
//!   {baseUrl}/embeddings` call, selected and configured from
//!   `EMBEDDINGS_BASE_URL` / `EMBEDDINGS_API_KEY`, the same env-var
//!   convention `crates/server/src/config.rs` uses for `SMTP_*`/`S3_*`.
//!   Works unmodified against OpenAI itself or a local Ollama instance's
//!   OpenAI-compat endpoint (`http://localhost:11434/v1`).

use std::sync::Arc;

use async_trait::async_trait;
use cratebase_core::field::EmbeddingConfig;
use cratebase_core::{FieldKind, Record};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// Env vars selecting the HTTP embedding provider, mirroring
/// `crates/server/src/config.rs`'s `SMTP_HOST`/`S3_BUCKET` convention:
/// presence of the base URL is what turns the real provider on.
const BASE_URL_ENV: &str = "EMBEDDINGS_BASE_URL";
const API_KEY_ENV: &str = "EMBEDDINGS_API_KEY";

/// A provider or transport failure. Kept separate from
/// `cratebase_core::AppError` so this module has no dependency on the
/// server's HTTP error mapping, matching `crate::llm::LlmError`'s split.
#[derive(Debug)]
pub enum EmbeddingError {
    /// The HTTP request to the provider itself failed (DNS, connect,
    /// timeout, a body that didn't decode).
    Http(reqwest::Error),
    /// The provider answered, but not with `2xx`.
    Provider { status: u16, body: String },
    /// A `2xx` response whose body wasn't the shape expected.
    Decode(String),
}

impl std::fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbeddingError::Http(e) => write!(f, "embedding provider request failed: {e}"),
            EmbeddingError::Provider { status, body } => {
                write!(f, "embedding provider returned {status}: {body}")
            }
            EmbeddingError::Decode(msg) => {
                write!(f, "embedding provider response was not valid: {msg}")
            }
        }
    }
}

impl std::error::Error for EmbeddingError {}

/// Something that turns text into a fixed-size float vector.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;
}

/// Deterministic, network-free fake: repeatedly hashes `text` with
/// SHA-256 to fill `dimensions` floats in `[-1, 1)`. Always available; the
/// zero-config fallback when no `EMBEDDINGS_BASE_URL` is configured, and
/// what this module's tests assert against.
pub struct EchoProvider {
    dimensions: usize,
}

impl EchoProvider {
    pub fn new(dimensions: usize) -> Self {
        EchoProvider { dimensions }
    }
}

#[async_trait]
impl EmbeddingProvider for EchoProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        Ok(hash_vector(text, self.dimensions))
    }
}

/// Fill `dimensions` floats in `[-1, 1)` from repeated SHA-256 hashing of
/// `text`, cycling the digest as needed. Pure and deterministic: the same
/// text always produces the same vector, which is what makes
/// [`EchoProvider`] usable in assertions.
fn hash_vector(text: &str, dimensions: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(dimensions);
    let mut seed = text.as_bytes().to_vec();
    while out.len() < dimensions {
        let digest = Sha256::digest(&seed);
        for chunk in digest.as_chunks::<4>().0 {
            if out.len() == dimensions {
                break;
            }
            let bits = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            out.push((bits as f32 / u32::MAX as f32) * 2.0 - 1.0);
        }
        seed = digest.to_vec();
    }
    out
}

/// An OpenAI-compatible `POST {baseUrl}/embeddings` provider — works
/// unmodified against OpenAI itself or a local Ollama instance's
/// OpenAI-compat endpoint, since both accept `{"model", "input"}` and
/// answer with `{"data": [{"embedding": [...]}]}`.
pub struct HttpEmbeddingProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
}

impl HttpEmbeddingProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
    ) -> Self {
        HttpEmbeddingProvider {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key,
            model: model.into(),
        }
    }

    /// Builds from `EMBEDDINGS_BASE_URL` / `EMBEDDINGS_API_KEY`, `None`
    /// when no base URL is configured — the signal callers use to fall
    /// back to [`EchoProvider`] instead of failing every write.
    pub fn from_env(model: impl Into<String>) -> Option<Self> {
        let base_url = std::env::var(BASE_URL_ENV).ok().filter(|v| !v.is_empty())?;
        let api_key = std::env::var(API_KEY_ENV).ok().filter(|v| !v.is_empty());
        Some(HttpEmbeddingProvider::new(base_url, api_key, model))
    }
}

#[async_trait]
impl EmbeddingProvider for HttpEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let url = format!("{}/embeddings", self.base_url.trim_end_matches('/'));
        let body = serde_json::to_vec(&serde_json::json!({
            "model": self.model,
            "input": text,
        }))
        .expect("an embedding request body is always serializable");
        let mut builder = self
            .client
            .post(&url)
            .header("Content-Type", "application/json");
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        let response = builder
            .body(body)
            .send()
            .await
            .map_err(EmbeddingError::Http)?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(EmbeddingError::Http)?;
        if !status.is_success() {
            return Err(EmbeddingError::Provider {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
        let parsed: Value =
            serde_json::from_slice(&bytes).map_err(|e| EmbeddingError::Decode(e.to_string()))?;
        let vector = parsed["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| EmbeddingError::Decode("missing data[0].embedding".into()))?;
        Ok(vector
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect())
    }
}

/// Resolve a field's `embedding.provider` name to a concrete provider.
/// `"echo"` is the explicit, network-free choice used by tests; anything
/// else tries the HTTP provider configured via env, falling back to
/// [`EchoProvider`] when no base URL is configured — a fresh install
/// still writes *something* deterministic on create rather than failing
/// every write until an operator wires up a real provider.
pub fn provider_for(config: &EmbeddingConfig, dimensions: usize) -> Arc<dyn EmbeddingProvider> {
    if config.provider == "echo" {
        return Arc::new(EchoProvider::new(dimensions));
    }
    match HttpEmbeddingProvider::from_env(config.model.clone()) {
        Some(p) => Arc::new(p),
        None => Arc::new(EchoProvider::new(dimensions)),
    }
}

/// Cosine similarity between two vectors; `0.0` when they differ in
/// length, either is empty, or either has zero magnitude (all of which
/// PocketBase-style filters would otherwise turn into a `NaN` that sorts
/// unpredictably).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// A vector field's current value as `Vec<f32>`, or `None` when it isn't
/// a well-formed array of numbers (blank, wrong shape, not yet embedded).
pub fn vector_of(record: &Record, field: &str) -> Option<Vec<f32>> {
    match record.get(field) {
        Some(Value::Array(items)) => items.iter().map(|v| v.as_f64().map(|n| n as f32)).collect(),
        _ => None,
    }
}

/// Compute embeddings for every `vector` field on `record`'s collection
/// that has `embedding` configured, and write the resulting float arrays
/// directly onto `record`.
///
/// `input` is the caller's request body (after the number/multi-value
/// modifier passes, before the write), used only to tell "the caller
/// supplied this field/its source explicitly this request" from "nothing
/// changed" — never mutated:
///
/// * a vector field present in `input` is caller-supplied and is never
///   overridden;
/// * on create, a configured vector field is always computed (when its
///   source field has non-blank text);
/// * on update, it is only recomputed when the source field itself was
///   part of this request — an update that doesn't touch the source text
///   leaves a previously computed vector alone.
pub async fn apply_embeddings(
    record: &mut Record,
    input: &Map<String, Value>,
) -> Result<(), EmbeddingError> {
    let collection = record.collection().clone();
    let is_new = record.is_new();
    for field in &collection.fields {
        let FieldKind::Vector {
            dimensions,
            embedding: Some(cfg),
        } = &field.kind
        else {
            continue;
        };
        if input.contains_key(&field.name) {
            continue;
        }
        if !is_new && !input.contains_key(&cfg.source_field) {
            continue;
        }
        let source = record.get_string(&cfg.source_field);
        if source.trim().is_empty() {
            continue;
        }
        let provider = provider_for(cfg, *dimensions);
        let vector = provider.embed(&source).await?;
        record.set(&field.name, serde_json::json!(vector));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_provider_is_deterministic_and_sized() {
        let a = hash_vector("hello world", 8);
        let b = hash_vector("hello world", 8);
        let c = hash_vector("goodbye", 8);
        assert_eq!(a.len(), 8);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.iter().all(|v| (-1.0..1.0).contains(v)));
    }

    #[test]
    fn cosine_similarity_matches_known_values() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert!((cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]) - -1.0).abs() < 1e-6);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[tokio::test]
    async fn provider_for_echo_produces_matching_dimensions() {
        let cfg = EmbeddingConfig {
            provider: "echo".into(),
            model: String::new(),
            source_field: "body".into(),
        };
        let provider = provider_for(&cfg, 16);
        let vector = provider.embed("some text").await.unwrap();
        assert_eq!(vector.len(), 16);
    }
}
