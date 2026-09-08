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
//!   collections, records, batch, auth (password) and files.
//! * [`plugin`], [`store`], [`cron`], [`extract`], [`http_error`].
//!
//! # Building an app
//!
//! ```ignore
//! let app = App::new(Config::from_env());
//! app.register_plugin(FnPlugin::new("audit", audit))?;
//! app.serve().await?;
//! ```

pub mod api_keys;
pub mod app;
pub mod audit;
pub mod config;
pub mod cookie;
pub mod cron;
pub mod cron_jobs;
mod dashboard;
pub mod embeddings;
pub mod events;
pub mod extract;
pub mod hooks;
pub mod http_error;
pub mod jsvm_host;
pub mod llm;
pub mod mcp;
pub mod middleware;
pub mod plugin;
pub mod plugin_wasm;
pub mod pocketbase_migrate;
pub mod push;
pub mod queue;
pub mod realtime;
#[cfg(test)]
mod records_multi_file_tests;
pub mod routes;
pub mod sessions;
pub mod store;
pub mod teams;
pub mod webhooks;
pub mod zip_export;

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
    let cfg = app.config();
    if cfg.session_cookie && (cfg.origins.iter().any(|o| o == "*") || cfg.origins.is_empty()) {
        tracing::warn!(
            "SESSION_COOKIE is on with a wildcard CORS origin; browsers refuse credentialed \
             cross-origin requests and the CSRF origin check will reject cross-site writes. \
             Set CORS_ALLOW_ORIGINS to an explicit list."
        );
    }
    let api = routes::api_router(&app)
        // Order matters: the rate limiter runs *before* the handler but
        // after logging, so a 429 is logged like any other response.
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            middleware::rate_limit::rate_limit,
        ))
        // CSRF sits between the rate limiter and request logging: a
        // rejected cross-site cookie write is still logged and counted
        // in `/metrics` (both wrap this layer), but never spends a
        // caller's rate-limit budget (rate limiting is *inside* this
        // layer — reached only once CSRF has passed). See
        // `middleware::csrf`'s module doc.
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            middleware::csrf::csrf,
        ))
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            middleware::request_log::log_requests,
        ))
        // Outermost of the four: counts what a caller actually
        // received, including a 429 from the rate limiter or a 403 from
        // CSRF, so `/metrics` reflects the real response mix rather
        // than only what reached a handler.
        .layer(axum::middleware::from_fn(routes::metrics::record_metrics));

    // `pb_hooks` `routerAdd` routes (mounted at the root, unprefixed,
    // exactly as registered, same as PocketBase) get the identical four
    // layers as the `/api` nest above — a hook route (e.g. a public
    // webhook receiver, or a downstream project's own custom
    // subscribe/unsubscribe endpoint) is otherwise a fully anonymous,
    // unauthenticated write with no per-caller rule of its own to fall
    // back on: exactly the shape both rate limiting and CSRF exist to
    // blunt. A rule/cookie with no matching path/tag/session is simply a
    // no-op, so this changes nothing for an install with neither
    // configured.
    let js_routes = jsvm_host::js_router(&app)
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            middleware::rate_limit::rate_limit,
        ))
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            middleware::csrf::csrf,
        ))
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            middleware::request_log::log_requests,
        ));

    Router::new()
        .nest("/api", api)
        .merge(js_routes)
        .merge(dashboard::router())
        // Deliberately outside the `/api` nest and its rate-limit/
        // logging layers — see `routes::metrics` for why (Prometheus
        // convention: unauthenticated, un-throttled, not a logged API
        // call).
        .merge(routes::metrics::router())
        .fallback(http_error::not_found_fallback)
        // PocketBase answers a wrong method on a real path with 404, not
        // 405 (verified against v0.40.2), so both fallbacks are the same.
        .method_not_allowed_fallback(http_error::not_found_fallback)
        .layer(TraceLayer::new_for_http())
        .layer(middleware::cors::layer(
            &app.config().origins,
            app.config().session_cookie,
        ))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(app)
}

