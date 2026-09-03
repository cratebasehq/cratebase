mod auth;
mod backups;
mod batch;
mod collections;
mod files;
mod health;
mod logs;
mod otp_auth;
mod realtime;
mod records;

use axum::Router;

use crate::state::AppState;

/// Assembles the full `/api/*` router. Split by resource into one module
/// each so a route's rule/validation logic sits next to its handler.
pub fn router(auth_rate_limit_enabled: bool) -> Router<AppState> {
    Router::new()
        .merge(health::router())
        .merge(collections::router())
        .merge(records::router())
        .merge(auth::router(auth_rate_limit_enabled))
        .merge(otp_auth::router())
        .merge(files::router())
        .merge(realtime::router())
        .merge(batch::router())
        .merge(backups::router())
        .merge(logs::router())
}
