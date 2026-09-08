//! Cross-site request rejection for cookie-authenticated writes.
//!
//! A bearer token in `Authorization` needs no CSRF defense: a
//! cross-site page cannot make the browser attach an arbitrary header,
//! so an attacker who can send that header already has the token itself.
//! A cookie is different — the browser attaches it automatically to any
//! request to this origin, cross-site page included — so a state-
//! changing request authenticated *only* by the session cookie has to
//! prove it actually came from a trusted origin.
//!
//! This layer is the primary defense and runs before [`rate_limit`], so
//! a flood of rejected cross-site writes never eats into a legitimate
//! caller's rate-limit budget (see `crate::lib`'s router assembly for the
//! exact ordering and why). [`crate::extract::resolve`] repeats the same
//! [`origin_allowed`] check as a fail-safe for any future route that
//! bypasses this layer — that check is not the primary defense, this one
//! is.
//!
//! [`rate_limit`]: crate::middleware::rate_limit::rate_limit

use axum::extract::{Request, State};
use axum::http::request::Parts;
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::app::App;
use crate::http_error::ApiError;

/// `GET`/`HEAD`/`OPTIONS` never mutate state, so they need no origin
/// check regardless of how the caller authenticated.
pub(crate) fn is_safe_method(parts: &Parts) -> bool {
    matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// The request's `Host` header, if present. There is deliberately no
/// `X-Forwarded-Proto`/`X-Forwarded-Host` handling here — scheme is
/// ignored by the comparison in [`origin_allowed`], and a same-host
/// cross-scheme attacker needs an active network position, which the
/// session cookie's `Secure` attribute already addresses.
fn request_host(parts: &Parts) -> Option<&str> {
    parts.headers.get(axum::http::header::HOST)?.to_str().ok()
}

/// Whether this request's `Origin` header (when present) is one this
/// deployment trusts. A missing `Origin` header (any non-browser client,
/// and same-origin navigations in some browsers) is always allowed —
/// there is nothing to compare. `app.config().origins` containing `"*"`
/// does **not** grant access here: that value only relaxes CORS for
/// bearer-token clients, and the default config ships with exactly that
/// wildcard, so treating it as permission would make this check a no-op
/// for every default deployment.
pub(crate) fn origin_allowed(parts: &Parts, app: &App) -> bool {
    let origin = match parts.headers.get(axum::http::header::ORIGIN) {
        Some(v) => match v.to_str() {
            Ok(s) => s,
            Err(_) => return false,
        },
        None => return true,
    };
    if let Some(host) = request_host(parts) {
        // Compare host:port only — ignore scheme, per the module doc.
        let origin_host = origin.split("://").nth(1).unwrap_or(origin);
        if origin_host == host {
            return true;
        }
    }
    app.config().origins.iter().any(|o| o == origin)
}

/// Rejects a state-changing, cookie-authenticated request whose `Origin`
/// is not trusted. A no-op entirely when `Config::session_cookie` is
/// off, when the request carries an `Authorization` header (bearer auth
/// needs no CSRF defense), or when the request has no session cookie at
/// all (nothing to protect).
pub async fn csrf(State(app): State<App>, req: Request, next: Next) -> Response {
    let cfg = app.config();
    if !cfg.session_cookie {
        return next.run(req).await;
    }
    let (parts, body) = req.into_parts();
    let protects = !is_safe_method(&parts)
        && !parts.headers.contains_key(axum::http::header::AUTHORIZATION)
        && crate::cookie::get(&parts, &cfg.session_cookie_name).is_some();
    if protects && !origin_allowed(&parts, &app) {
        return ApiError::forbidden("Cross-site request rejected.").into_response();
    }
    let req = Request::from_parts(parts, body);
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use axum::http::Request as HttpRequest;

    use super::*;

    fn parts(method: Method, headers: &[(&str, &str)]) -> Parts {
        let mut b = HttpRequest::builder().method(method).uri("/api/x");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(()).unwrap().into_parts().0
    }

    #[test]
    fn safe_methods_are_always_safe() {
        assert!(is_safe_method(&parts(Method::GET, &[])));
        assert!(is_safe_method(&parts(Method::HEAD, &[])));
        assert!(is_safe_method(&parts(Method::OPTIONS, &[])));
        assert!(!is_safe_method(&parts(Method::POST, &[])));
    }

    #[test]
    fn missing_origin_header_is_allowed() {
        let app = App::new(crate::config::Config::memory("/tmp/csrf-test-1"));
        let p = parts(Method::POST, &[("host", "example.com")]);
        assert!(origin_allowed(&p, &app));
    }

    #[test]
    fn same_host_origin_is_allowed() {
        let app = App::new(crate::config::Config::memory("/tmp/csrf-test-2"));
        let p = parts(
            Method::POST,
            &[("host", "example.com"), ("origin", "https://example.com")],
        );
        assert!(origin_allowed(&p, &app));
    }

    #[test]
    fn cross_site_origin_with_wildcard_config_is_rejected() {
        // The default `origins` is `["*"]`; this must not satisfy the
        // check or CSRF protection would be a no-op by default.
        let app = App::new(crate::config::Config::memory("/tmp/csrf-test-3"));
        let p = parts(
            Method::POST,
            &[("host", "example.com"), ("origin", "https://evil.example")],
        );
        assert!(!origin_allowed(&p, &app));
    }

    #[test]
    fn cross_site_origin_in_explicit_allowlist_is_allowed() {
        let mut cfg = crate::config::Config::memory("/tmp/csrf-test-4");
        cfg.origins = vec!["https://trusted.example".into()];
        let app = App::new(cfg);
        let p = parts(
            Method::POST,
            &[("host", "example.com"), ("origin", "https://trusted.example")],
        );
        assert!(origin_allowed(&p, &app));
    }
}
