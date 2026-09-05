//! The contract between a WASM plugin and the application that embeds
//! it — the same split `cratebase_jsvm::HostApi` uses for `pb_hooks`
//! (see that crate's module doc): the runtime never touches a database
//! or an HTTP client itself, every host import a plugin calls ends up on
//! [`PluginHostApi`], and the server crate implements it on top of
//! `App`. [`crate::runtime`] is buildable and testable with a mock host
//! with no server, database or axum in the dependency tree at all.
//!
//! This surface is deliberately much narrower than `HostApi`: a plugin
//! is untrusted third-party code, not first-party `pb_hooks`, so it gets
//! exactly the two capabilities [`crate::manifest::Capabilities`] can
//! declare — reading a record through the normal rule-enforced path, and
//! finding out who the caller is — rather than arbitrary `$app.*` access.

use async_trait::async_trait;
use serde_json::Value;

/// Who the plugin is running on behalf of, resolved by the HTTP layer
/// *before* the plugin ever runs (from the same `Auth` extractor every
/// other route uses) and handed to the runtime as plain data. The
/// runtime does not resolve auth itself.
#[derive(Debug, Clone, Default)]
pub struct PluginRequestContext {
    /// `Some((collection, id, record_json))` for an authenticated
    /// caller; `None` for anonymous. `record_json` is the minimal
    /// `{"id", "email", "collectionId", "collectionName"}` shape — never
    /// the full record — the same trimmed identity `@request.auth`
    /// exposes to a rule.
    pub auth: Option<AuthIdentity>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuthIdentity {
    pub id: String,
    pub collection_id: String,
    pub collection_name: String,
    pub email: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PluginHostError {
    #[error("collection '{0}' not found")]
    NotFound(String),
    #[error("host call failed: {0}")]
    Internal(String),
}

/// Everything a plugin's host imports need from the embedding
/// application. Every method is capability-checked by the *caller*
/// ([`crate::runtime::PluginInstance`]) against the plugin's manifest
/// before it ever reaches here — an implementation does not have to
/// re-derive "is this plugin allowed to ask this" itself.
#[async_trait]
pub trait PluginHostApi: Send + Sync + 'static {
    /// Fetch one record through the exact rule-enforced path
    /// `GET /api/collections/{collection}/records/{id}` uses for
    /// `ctx.auth` — a failing `viewRule` returns `Ok(None)`, indistinguishable
    /// from a genuinely missing id, same as the HTTP endpoint.
    async fn get_record(
        &self,
        ctx: &PluginRequestContext,
        collection: &str,
        id: &str,
    ) -> Result<Option<Value>, PluginHostError>;

    /// A structured log line from a plugin, tagged with its name.
    fn log(&self, plugin: &str, level: &str, message: &str);
}
