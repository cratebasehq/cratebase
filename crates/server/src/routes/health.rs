//! `GET /api/health`.
//!
//! The one endpoint that still spells the envelope key `code` rather than
//! `status` — a PocketBase inconsistency the SDK depends on, so it is
//! reproduced deliberately (spec §2, and `tests/conformance/health.test.ts`).
//!
//! `data` is empty for anyone but a superuser; a superuser additionally
//! gets `canBackup`, the resolved `realIP` and `possibleProxyHeader`, the
//! last being a hint that a proxy header is present that
//! `settings.trustedProxy.headers` does not trust.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::extract::MaybeAuth;
use crate::middleware::client_ip::{self, PeerAddr};

pub fn router() -> Router<App> {
    Router::new().route("/health", get(health))
}

async fn health(
    State(app): State<App>,
    MaybeAuth(auth): MaybeAuth,
    PeerAddr(peer): PeerAddr,
    headers: HeaderMap,
) -> Json<Value> {
    let mut data = Map::new();
    if auth.is_some_and(|a| a.is_superuser) {
        let settings = app.settings();
        data.insert(
            "canBackup".into(),
            json!(app.backups_storage().is_ok() && app.config().sqlite_main_path().is_some()),
        );
        data.insert(
            "realIP".into(),
            json!(client_ip::client_ip(
                &headers,
                peer,
                &settings.trusted_proxy
            )),
        );
        data.insert(
            "possibleProxyHeader".into(),
            json!(client_ip::possible_proxy_header(
                &headers,
                &settings.trusted_proxy
            )),
        );
    }
    Json(json!({
        "code": 200,
        "message": "API is healthy.",
        "data": data,
    }))
}
