//! CORS, driven by `config.origins` (`CORS_ALLOW_ORIGINS` / `--origins`).
//!
//! `*` mirrors PocketBase's default (it ships wide open, on the grounds
//! that the API is token-authenticated and cookies are never used). A
//! concrete list switches to an allow-list and drops any entry that is
//! not a valid header value.

use axum::http::{HeaderValue, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};

pub fn layer(origins: &[String]) -> CorsLayer {
    let base = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(tower_http::cors::Any)
        .expose_headers(tower_http::cors::Any);

    if origins.iter().any(|o| o == "*") || origins.is_empty() {
        base.allow_origin(tower_http::cors::Any)
    } else {
        let list: Vec<HeaderValue> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        base.allow_origin(AllowOrigin::list(list))
    }
}
