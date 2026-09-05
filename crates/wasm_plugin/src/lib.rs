//! A sandboxed runtime for third-party WASM plugins.
//!
//! # Scope
//!
//! This is the R&D foundation for a plugin *system*, not a finished one
//! — see this crate's tests and `crates/server/src/plugin_wasm.rs`'s
//! module doc for what is and is not built yet. What exists:
//!
//! * a real, embeddable WASM sandbox ([`wasmtime`]) with a genuine fuel
//!   budget and a genuine linear-memory cap — not best-effort limits;
//! * a manifest format ([`manifest::Manifest`]) that declares a
//!   plugin's identity and the specific host capabilities it wants,
//!   checked before install and re-checked on every call;
//! * a narrow [`host::PluginHostApi`] a plugin calls back into, in the
//!   same "runtime never touches the database directly" shape
//!   `cratebase_jsvm::HostApi` uses for `pb_hooks`.
//!
//! What this crate deliberately does not attempt: a component-model /
//! WIT-typed interface (the ABI in [`runtime`] is hand-rolled instead —
//! see that module's doc for the exact contract), multi-plugin
//! isolation from *each other* beyond what separate `Store`s already
//! give for free, plugin signing/provenance, or a hosted registry —
//! that last one is explicitly out of scope for this batch.
//!
//! # Building a plugin
//!
//! A plugin is any `wasm32-unknown-unknown` module that exports
//! `memory`, `alloc(i32) -> i32` and `handle_request(i32, i32) -> i64`,
//! alongside a `plugin.toml` manifest declaring its name, version and
//! capabilities. See `examples/wasm-plugin-hello` in the repository for
//! a complete, working one.

pub mod host;
pub mod manifest;
pub mod runtime;

pub use host::{AuthIdentity, PluginHostApi, PluginHostError, PluginRequestContext};
pub use manifest::{Capabilities, Manifest, ManifestError};
pub use runtime::{PluginEngine, PluginProgram, PluginRequest, PluginResponse, RuntimeError};
