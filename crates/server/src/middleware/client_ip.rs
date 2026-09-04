//! Resolving the client's IP address.
//!
//! # Why this is not "just read `X-Forwarded-For`"
//!
//! `X-Forwarded-For` is attacker-controlled: anybody can send
//! `X-Forwarded-For: 1.2.3.4` and, if the server believes it, sail past
//! any per-IP rate limit or superuser IP allowlist. The Phase 1 code
//! trusted the header unconditionally, which the audit flagged.
//!
//! So a forwarded header is honoured **only** when the operator listed it
//! in `settings.trustedProxy.headers` — i.e. only when they have told us
//! there really is a proxy in front rewriting it. With no configured
//! header the peer address is the client address, full stop.
//!
//! `useLeftmostIP` picks which end of the chain to trust: the leftmost
//! entry is the original client as reported by the first proxy (right for
//! a chain you fully control), the rightmost is the address the *last*
//! proxy saw (right when only the nearest proxy is trustworthy, and
//! PocketBase's default).

use std::net::SocketAddr;

use axum::http::HeaderMap;
use cratebase_core::settings::TrustedProxy;

/// Proxy headers PocketBase warns about in `GET /api/health`
/// (`possibleProxyHeader`) when they are present but untrusted.
pub const KNOWN_PROXY_HEADERS: &[&str] = &[
    "CF-Connecting-IP",
    "Fly-Client-IP",
    "X-Forwarded-For",
    "X-Real-IP",
    "True-Client-IP",
];

/// The address the peer connected from, as recorded by axum's
/// `ConnectInfo`. `None` when the router was driven directly (tests).
pub fn remote_ip(peer: Option<SocketAddr>) -> String {
    peer.map(|a| a.ip().to_string()).unwrap_or_default()
}

/// The peer address as an infallible extractor.
///
/// `ConnectInfo` itself rejects when the service was not built with
/// `into_make_service_with_connect_info` — which is exactly what a
/// `tower::Service`-driven test does — so handlers take this instead and
/// treat a missing peer as "no address".
#[derive(Debug, Clone, Copy, Default)]
pub struct PeerAddr(pub Option<SocketAddr>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for PeerAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(PeerAddr(
            parts
                .extensions
                .get::<axum::extract::ConnectInfo<SocketAddr>>()
                .map(|ci| ci.0),
        ))
    }
}

/// The client's IP: a trusted proxy header when configured and present,
/// otherwise the peer address.
pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>, proxy: &TrustedProxy) -> String {
    for name in &proxy.headers {
        let Some(raw) = headers.get(name.as_str()).and_then(|v| v.to_str().ok()) else {
            continue;
        };
        let mut parts = raw.split(',').map(str::trim).filter(|s| !s.is_empty());
        let picked = if proxy.use_leftmost_ip {
            parts.next()
        } else {
            parts.next_back()
        };
        if let Some(ip) = picked {
            return ip.to_string();
        }
    }
    remote_ip(peer)
}

/// The first well-known proxy header present on the request that is *not*
/// trusted — PocketBase surfaces it in `/api/health` so an operator
/// notices they are behind a proxy they have not configured.
pub fn possible_proxy_header(headers: &HeaderMap, proxy: &TrustedProxy) -> String {
    KNOWN_PROXY_HEADERS
        .iter()
        .find(|candidate| {
            headers.contains_key(**candidate)
                && !proxy
                    .headers
                    .iter()
                    .any(|trusted| trusted.eq_ignore_ascii_case(candidate))
        })
        .map(|h| h.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    fn peer() -> Option<SocketAddr> {
        Some("203.0.113.9:5555".parse().unwrap())
    }

    #[test]
    fn a_spoofed_header_is_ignored_when_no_proxy_is_configured() {
        let h = headers(&[("X-Forwarded-For", "1.2.3.4")]);
        assert_eq!(
            client_ip(&h, peer(), &TrustedProxy::default()),
            "203.0.113.9"
        );
    }

    #[test]
    fn a_trusted_header_wins_and_the_side_follows_use_leftmost() {
        let h = headers(&[("X-Forwarded-For", "1.2.3.4, 10.0.0.1, 10.0.0.2")]);
        let rightmost = TrustedProxy {
            headers: vec!["X-Forwarded-For".into()],
            use_leftmost_ip: false,
        };
        assert_eq!(client_ip(&h, peer(), &rightmost), "10.0.0.2");

        let leftmost = TrustedProxy {
            headers: vec!["X-Forwarded-For".into()],
            use_leftmost_ip: true,
        };
        assert_eq!(client_ip(&h, peer(), &leftmost), "1.2.3.4");
    }

    #[test]
    fn headers_are_tried_in_order_and_fall_through_when_absent() {
        let h = headers(&[("X-Real-IP", "8.8.8.8")]);
        let proxy = TrustedProxy {
            headers: vec!["CF-Connecting-IP".into(), "X-Real-IP".into()],
            use_leftmost_ip: false,
        };
        assert_eq!(client_ip(&h, peer(), &proxy), "8.8.8.8");
        assert_eq!(client_ip(&HeaderMap::new(), peer(), &proxy), "203.0.113.9");
    }

    #[test]
    fn untrusted_proxy_headers_are_reported_by_health() {
        let h = headers(&[("X-Forwarded-For", "1.2.3.4")]);
        assert_eq!(
            possible_proxy_header(&h, &TrustedProxy::default()),
            "X-Forwarded-For"
        );
        let trusted = TrustedProxy {
            headers: vec!["x-forwarded-for".into()],
            use_leftmost_ip: false,
        };
        assert_eq!(possible_proxy_header(&h, &trusted), "");
    }
}
