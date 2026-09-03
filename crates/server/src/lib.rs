//! Cratebase's HTTP server: wires the db/storage/auth crates into an axum
//! `Router` and exposes the pieces `main.rs` needs to actually run it (or a
//! test harness needs to spin up an in-process instance).

pub mod auth_fields;
pub mod config;
mod dashboard;
pub mod extract;
pub mod helpers;
pub mod http_error;
pub mod payload;
pub mod realtime;
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
    let storage = Storage::connect(&config.storage)?;

    Ok(AppState {
        db,
        storage,
        config: Arc::new(config),
        realtime: RealtimeHub::default(),
    })
}

/// One request body size cap for every route. File uploads are the only
/// large bodies Cratebase accepts; 100 MiB comfortably covers typical
/// documents/images/video clips while still bounding worst-case memory use
/// per in-flight request.
const MAX_BODY_BYTES: usize = 100 * 1024 * 1024;

pub fn build_app(state: AppState) -> Router {
    let cors = build_cors(&state.config.cors_allow_origins);

    Router::new()
        .nest("/api", routes::router())
        .merge(dashboard::router())
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
