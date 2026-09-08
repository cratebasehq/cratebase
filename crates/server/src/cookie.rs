//! httpOnly session-cookie parsing and construction.
//!
//! Cookies are entirely opt-in (`Config::session_cookie`, off by
//! default): the bearer `Authorization` header keeps working exactly as
//! before whether or not this is on, and `Authorization` always wins
//! when both are present (`crate::extract::request_token`). This module
//! only knows how to read the `Cookie` request header and build
//! `Set-Cookie` response values — it never touches the database or the
//! `_sessions` ledger.

use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};

use crate::config::Config;

/// The value of cookie `name` in the request's `Cookie` header, if
/// present. Cookies are `;`-separated `name=value` pairs with optional
/// surrounding whitespace (RFC 6265 §4.2.1).
pub fn get<'a>(parts: &'a Parts, name: &str) -> Option<&'a str> {
    let raw = parts.headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k.trim() == name).then(|| v.trim())
    })
}

/// Builds a `Set-Cookie` header value for `name=value`, always
/// `HttpOnly`, with `Secure`/`SameSite`/`Domain` taken from `cfg` and a
/// `Max-Age` of `max_age` seconds (0 or negative clears it — the same
/// convention `clear_session` uses with an empty value).
pub fn build(cfg: &Config, name: &str, value: &str, max_age: i64, path: &str) -> HeaderValue {
    let mut out = format!("{name}={value}; Path={path}; HttpOnly; SameSite={}", cfg.session_cookie_same_site.as_str());
    out.push_str(&format!("; Max-Age={}", max_age.max(0)));
    if cfg.session_cookie_secure {
        out.push_str("; Secure");
    }
    if !cfg.session_cookie_domain.is_empty() {
        out.push_str(&format!("; Domain={}", cfg.session_cookie_domain));
    }
    // A cookie name/value built entirely from server-controlled ASCII
    // (bearer tokens, JWTs, our own configured name) is always a valid
    // header value; a stray control character would be a programming
    // error worth panicking on rather than silently dropping the cookie.
    HeaderValue::from_str(&out).expect("cookie header value must be valid ASCII")
}

/// Appends (never replaces) a `set-cookie` header, so more than one
/// cookie can ship in a single response (e.g. the session cookie plus
/// `cb_session_prev` on `impersonate`).
pub fn attach(headers: &mut HeaderMap, value: HeaderValue) {
    headers.append(axum::http::header::SET_COOKIE, value);
}

/// The main session cookie (`cfg.session_cookie_name`), `Path=/`.
pub fn session(cfg: &Config, token: &str, max_age: i64) -> HeaderValue {
    build(cfg, &cfg.session_cookie_name, token, max_age, "/")
}

/// Clears the main session cookie (`Max-Age=0`, empty value).
pub fn clear_session(cfg: &Config) -> HeaderValue {
    build(cfg, &cfg.session_cookie_name, "", 0, "/")
}

#[cfg(test)]
mod tests {
    use axum::http::Request;

    use super::*;

    fn parts_with_cookie(raw: &str) -> Parts {
        Request::builder()
            .header(axum::http::header::COOKIE, raw)
            .body(())
            .unwrap()
            .into_parts()
            .0
    }

    #[test]
    fn get_finds_a_named_cookie_among_several() {
        let parts = parts_with_cookie("a=1; cb_session=tok123; b=2");
        assert_eq!(get(&parts, "cb_session"), Some("tok123"));
        assert_eq!(get(&parts, "missing"), None);
    }

    #[test]
    fn build_sets_httponly_and_configured_attributes() {
        let mut cfg = Config::memory("/tmp/x");
        cfg.session_cookie_secure = true;
        cfg.session_cookie_domain = "example.com".into();
        let v = build(&cfg, "cb_session", "tok", 3600, "/").to_str().unwrap().to_string();
        assert!(v.contains("HttpOnly"));
        assert!(v.contains("Secure"));
        assert!(v.contains("SameSite=Lax"));
        assert!(v.contains("Domain=example.com"));
        assert!(v.contains("Max-Age=3600"));
    }

    #[test]
    fn clear_session_zeroes_max_age_and_empties_value() {
        let cfg = Config::memory("/tmp/x");
        let v = clear_session(&cfg).to_str().unwrap().to_string();
        assert!(v.starts_with("cb_session=;"));
        assert!(v.contains("Max-Age=0"));
    }
}
