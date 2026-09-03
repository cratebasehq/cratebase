//! Cratebase's HTTP server: wires the db/storage/auth crates into an axum
//! `Router` and exposes the pieces `main.rs` needs to actually run it (or a
//! test harness needs to spin up an in-process instance).

pub mod auth_fields;
pub mod config;
mod dashboard;
pub mod extract;
pub mod helpers;
pub mod http_error;
pub mod mail;
pub mod oauth2;
pub mod payload;
pub mod plugin;
pub mod plugins;
pub mod realtime;
mod request_log;
mod routes;
pub mod state;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, Method};
use axum::Router;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

use config::Config;
use cratebase_db::{system, Db};
use cratebase_storage::Storage;
use realtime::RealtimeHub;
use state::AppState;

/// Connect to the database and storage backend, run system-table bootstrap,
/// and assemble the shared application state. Called once at startup (and
/// by integration tests that want a fully wired instance without a real
/// network listener).
pub async fn build_state(config: Config) -> anyhow::Result<AppState> {
    let db = Db::connect(&config.database_url).await?;
    system::ensure_system_tables(&db).await?;
    system::ensure_request_logs_table(&db).await?;
    system::ensure_default_collections(&db).await?;
    let storage = Storage::connect(&config.storage)?;
    let mailer = cratebase_mailer::Mailer::connect(
        &config.mailer,
        &config.mail_from_address,
        &config.mail_from_name,
    )?;

    Ok(AppState {
        db,
        storage,
        config: Arc::new(config),
        realtime: RealtimeHub::default(),
        mailer,
    })
}

/// One request body size cap for every route. File uploads are the only
/// large bodies Cratebase accepts; 100 MiB comfortably covers typical
/// documents/images/video clips while still bounding worst-case memory use
/// per in-flight request.
const MAX_BODY_BYTES: usize = 100 * 1024 * 1024;

/// Assembles the HTTP router. `plugins` is injected rather than hardcoded
/// to `plugins::registry()` so a downstream Rust binary can depend on this
/// crate as a library and register its own [`plugin::Plugin`]s, instead of
/// only being able to add plugins by editing
/// `crates/server/src/plugins/mod.rs` in this repo directly (see
/// `plugin`'s module doc for the full pattern). Pass `&plugins::registry()`
/// to keep every built-in plugin, or `&PluginRegistry::new().register(YourPlugin)`
/// for a binary that ships only your own.
pub fn build_app(state: AppState, plugins: &plugin::PluginRegistry) -> Router {
    let cors = build_cors(&state.config.cors_allow_origins);

    let api = routes::router(state.config.auth_rate_limit_enabled).layer(
        axum::middleware::from_fn_with_state(state.clone(), request_log::log_requests),
    );

    Router::new()
        .nest("/api", api)
        .merge(dashboard::router())
        .merge(plugins.router())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

fn build_cors(allowed: &[String]) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(tower_http::cors::Any);

    if allowed.iter().any(|o| o == "*") {
        layer.allow_origin(tower_http::cors::Any)
    } else {
        let origins: Vec<HeaderValue> = allowed.iter().filter_map(|o| o.parse().ok()).collect();
        layer.allow_origin(AllowOrigin::list(origins))
    }
}
