mod auth;
mod collections;
mod files;
mod health;
mod realtime;
mod records;

use axum::Router;

use crate::state::AppState;

/// Assembles the full `/api/*` router. Split by resource into one module
/// each so a route's rule/validation logic sits next to its handler.
pub fn router() -> Router<AppState> {
    Router::new()
        .merge(health::router())
        .merge(collections::router())
        .merge(records::router())
        .merge(auth::router())
        .merge(files::router())
        .merge(realtime::router())
}
