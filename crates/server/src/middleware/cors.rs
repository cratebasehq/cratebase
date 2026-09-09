//! CORS, driven by `config.origins` (`CORS_ALLOW_ORIGINS` / `--origins`).
//!
//! `*` mirrors PocketBase's default (it ships wide open, on the grounds
//! that the API is token-authenticated and cookies are never used by
//! default). A concrete list switches to an allow-list and drops any
//! entry that is not a valid header value.
//!
//! `allow_headers` mirrors the preflight's own `Access-Control-Request-Headers`
//! back verbatim instead of literally sending a `*` wildcard: per the
//! Fetch spec, a wildcard in `Access-Control-Allow-Headers` never covers
//! `Authorization` (browsers explicitly special-case it out of `*`
//! matching), so a token-authenticated client attaching its own
//! `Authorization` header would have every cross-origin request silently
//! blocked by the browser regardless of what this server sends — a real
//! bug confirmed live via a Firefox CORS console warning against a
//! deployed instance, not a hypothetical. Mirroring is the tower-http-
//! documented way to effectively "allow any header" while still actually
//! covering `Authorization`.
//!
//! # Credentials (cookie sessions)
//!
//! `allow_credentials` is only ever set when `Config::session_cookie` is
//! on **and** `origins` is an explicit, non-wildcard list — the Fetch
//! spec forbids `Access-Control-Allow-Credentials: true` together with a
//! wildcard `Access-Control-Allow-Origin`, and `tower_http`'s
//! `CorsLayer` enforces the same pairing for `Access-Control-Expose-Headers`:
//! combining credentials with `expose_headers(Any)` panics at router
//! construction (`ensure_usable_cors_rules`). So credentials mode also
//! swaps the wildcard `expose_headers` for the small explicit list this
//! API actually sets from JS (`Content-Disposition`, on file downloads).
//! A bearer-only deployment is unaffected either way — it never sends
//! cookies, so `Access-Control-Allow-Credentials` being absent changes
//! nothing for it.

use axum::http::{HeaderValue, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};

pub fn layer(origins: &[String], allow_credentials: bool) -> CorsLayer {
    let is_wildcard = origins.iter().any(|o| o == "*") || origins.is_empty();
    let credentials = allow_credentials && !is_wildcard;

    let mut base = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(tower_http::cors::AllowHeaders::mirror_request());

    base = if credentials {
        base.expose_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::CONTENT_LENGTH,
            axum::http::header::CONTENT_DISPOSITION,
        ])
        .allow_credentials(true)
    } else {
        base.expose_headers(tower_http::cors::Any)
    };

    if is_wildcard {
        base.allow_origin(tower_http::cors::Any)
    } else {
        let list: Vec<HeaderValue> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        base.allow_origin(AllowOrigin::list(list))
    }
}
