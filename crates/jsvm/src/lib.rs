//! Embedded JavaScript runtime for Cratebase: PocketBase's `pb_hooks` and
//! `pb_migrations`, on QuickJS (via `rquickjs`).
//!
//! The crate is decoupled from the server through the [`HostApi`] trait:
//! JavaScript globals (`$app`, `$http`, `routerAdd`, `on*`, ...) call
//! into the host, and the host calls back into the runtime through
//! [`Runtime::call_hook`], [`Runtime::call_route`] and
//! [`Runtime::call_cron`]. See [`runtime`] for the threading model and
//! the `e.next()` semantics, and [`host`] for what an embedder must
//! implement.
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use cratebase_jsvm::{Runtime, RuntimeConfig, HostApi};
//! # async fn demo(host: Arc<dyn HostApi>) -> Result<(), cratebase_core::AppError> {
//! let rt = Runtime::start(host, RuntimeConfig::new("pb_hooks", "pb_migrations")).await?;
//! // The host now knows every hook/route/cron the hook files registered
//! // and dispatches back through rt.call_hook / call_route / call_cron.
//! # Ok(()) }
//! ```

mod bridge;
mod convert;
pub mod host;
mod modules;
pub mod runtime;
mod security;
pub mod types;
mod worker;

pub use host::{
    CronHandlerId, HookHandlerId, HookKind, HostApi, HttpRequest, HttpResponse, RecordTokenKind,
    RouteHandlerId, TransactionFn,
};
pub use runtime::{
    JsBody, JsEvent, JsEventOutcome, JsRequest, JsResponse, MigrationLedger, Runtime,
    RuntimeConfig, JS_RECORD_OPTIONS,
};
pub use types::{write_types_file, TYPES_D_TS};
