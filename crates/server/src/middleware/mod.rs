//! HTTP middleware: client-IP resolution, request logging, rate limiting
//! and CORS.

pub mod client_ip;
pub mod cors;
pub mod rate_limit;
pub mod request_log;

use axum::http::request::Parts;
use axum::http::Uri;

/// The URI as the client sent it.
///
/// Both middlewares live *inside* the `/api` nest (so plugin routes are
/// covered too), and axum strips the nest prefix from `parts.uri` — a
/// request for `/api/health` reads as `/health` in there. `OriginalUri`
/// is the untouched one, which is what a log row and a rate-limit rule
/// label must both be matched against.
pub fn original_uri(parts: &Parts) -> &Uri {
    parts
        .extensions
        .get::<axum::extract::OriginalUri>()
        .map(|original| &original.0)
        .unwrap_or(&parts.uri)
}
