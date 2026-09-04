//! Cratebase's HTTP server: the [`App`] handle, the hook system, the
//! services that do not depend on the record layer, and the axum router
//! that ties them together.
//!
//! # Layout
//!
//! * [`app`] — the `App`/`TxApp` handles and the boot/serve/terminate
//!   lifecycle.
//! * [`hooks`] + [`events`] — PocketBase's middleware-style hook chains.
//! * [`config`] — boot-time settings from the environment; everything
//!   else lives in [`cratebase_core::Settings`].
//! * [`middleware`] — client-IP resolution, request logging, rate limits,
//!   CORS.
//! * [`routes`] — health, settings, logs, backups, crons,
//!   collections, records, auth (password) and files. The realtime and
//!   batch routes are W4b-2 and are marked in `routes::api_router`.
//! * [`plugin`], [`store`], [`cron`], [`extract`], [`http_error`].
//!
//! # Building an app
//!
//! ```ignore
//! let app = App::new(Config::from_env());
//! app.register_plugin(FnPlugin::new("audit", audit))?;
//! app.serve().await?;
//! ```

pub mod app;
pub mod config;
pub mod cron;
mod dashboard;
pub mod events;
pub mod extract;
pub mod hooks;
pub mod http_error;
pub mod middleware;
pub mod plugin;
pub mod realtime;
pub mod routes;
pub mod store;

use axum::extract::DefaultBodyLimit;
use axum::Router;
use tower_http::trace::TraceLayer;

pub use app::{App, TxApp};
pub use config::Config;
pub use cron::CronService;
pub use hooks::{Chain, Event, Handler, Hook, HookResult, Hooks};
pub use http_error::{ApiError, ApiResult};
pub use plugin::{FnPlugin, Plugin, PluginRegistry};
pub use store::Store;

/// One request-body cap for every route. File uploads are the only large
/// bodies Cratebase accepts; 100 MiB covers typical documents, images and
/// video clips while bounding worst-case memory per in-flight request.
pub const MAX_BODY_BYTES: usize = 100 * 1024 * 1024;

// `App` is itself the router state; axum's blanket `impl<T: Clone>
// FromRef<T> for T` already covers it, so the extractors that ask for
// `App: FromRef<S>` work with `State<App>` out of the box.

/// Assemble the complete router: `/api` (with logging and rate limiting),
/// the embedded dashboard, and PocketBase's error envelope on every
/// fallback.
pub fn router(app: App) -> Router {
    let api = routes::api_router(&app)
        // Order matters: the rate limiter runs *before* the handler but
        // after logging, so a 429 is logged like any other response.
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            middleware::rate_limit::rate_limit,
        ))
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            middleware::request_log::log_requests,
        ));

    Router::new()
        .nest("/api", api)
        .merge(dashboard::router())
        .fallback(http_error::not_found_fallback)
        // PocketBase answers a wrong method on a real path with 404, not
        // 405 (verified against v0.40.2), so both fallbacks are the same.
        .method_not_allowed_fallback(http_error::not_found_fallback)
        .layer(TraceLayer::new_for_http())
        .layer(middleware::cors::layer(&app.config().origins))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(app)
}
