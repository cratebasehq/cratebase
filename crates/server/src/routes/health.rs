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
//!
//! Liveness alone (the process is up) is not enough to call the API
//! "healthy" — a database that has gone away leaves every other route
//! failing while `/api/health` kept saying 200. A cheap `SELECT 1` gates
//! the response: on failure this returns `503` in the same envelope
//! shape, just with `code: 503` and no superuser diagnostics (which would
//! themselves need a working database to compute).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use cratebase_db::engine::Executor;
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
) -> (StatusCode, Json<Value>) {
    if app.db().query("SELECT 1", &[]).await.is_err() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "code": 503,
                "message": "Database is unavailable.",
                "data": {},
            })),
        );
    }

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
        // Lets the dashboard show its "Mail inbox" settings page only
        // when there's actually something to look at: the mailer's `Log`
        // backend, and thus `/api/dev/mails`, only runs when no real
        // transport (SMTP/Resend) is configured.
        data.insert(
            "devMailInbox".into(),
            json!(app.mailer().dev_mailbox().is_some()),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "code": 200,
            "message": "API is healthy.",
            "data": data,
        })),
    )
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::app::App;
    use crate::config::Config;

    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    async fn superuser_token(app: &App) -> String {
        let id = app
            .create_superuser("admin@example.com", "password12345")
            .await
            .expect("create superuser");
        app.mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("mint token")
    }

    /// The dashboard's "Mail inbox" settings page reads this to decide
    /// whether to show itself at all — it must track whether
    /// `/api/dev/mails` will actually answer.
    #[tokio::test]
    async fn superuser_sees_dev_mail_inbox_flag_reflecting_the_mailer_backend() {
        let (app, _dir) = test_app().await;
        let token = superuser_token(&app).await;
        let router = crate::routes::api_router(&app).with_state(app.clone());

        let response = router
            .clone()
            .oneshot(
                Request::get("/health")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"]["devMailInbox"], serde_json::json!(true));

        let mut settings = (*app.settings()).clone();
        settings.smtp = cratebase_core::settings::Smtp {
            enabled: true,
            host: "smtp.example.com".into(),
            ..Default::default()
        };
        app.set_settings(settings).await.expect("save settings");

        let response = router
            .oneshot(
                Request::get("/health")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"]["devMailInbox"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn a_working_database_reports_healthy() {
        let (app, _dir) = test_app().await;
        let router = crate::routes::api_router(&app).with_state(app);

        let response = router
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], 200);
        assert_eq!(json["message"], "API is healthy.");
        assert_eq!(json["data"], serde_json::json!({}));
    }

    #[tokio::test]
    async fn a_closed_database_reports_503_in_the_same_envelope_shape() {
        let (app, _dir) = test_app().await;
        // Simulates a real DB outage: every subsequent query fails.
        app.db().close().await.expect("close");
        let router = crate::routes::api_router(&app).with_state(app);

        let response = router
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Same PocketBase-shaped envelope (`code`, not `status`) as the
        // healthy response, so clients don't need a special case.
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(keys, ["code", "data", "message"]);
        assert_eq!(json["code"], 503);
        assert_eq!(json["data"], serde_json::json!({}));
    }
}
