//! End-to-end coverage for the magic-link login flow: request (captured
//! in the dev inbox) -> extract the token from the link -> auth-with -> a
//! normal `{token, record}` response; reuse fails; expiry fails; disabled
//! is rejected; an unknown email is still a 200 with no mail sent.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use serde_json::{json, Value};
use tower::ServiceExt;

const SUPERUSER_EMAIL: &str = "admin@example.com";
const SUPERUSER_PASSWORD: &str = "hunter2hunter2";

struct Harness {
    app: App,
    superuser_token: String,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        let id = app
            .create_superuser(SUPERUSER_EMAIL, SUPERUSER_PASSWORD)
            .await
            .expect("superuser");
        let superuser_token = app
            .mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token");
        Harness {
            app,
            superuser_token,
            _dir: dir,
        }
    }

    fn router(&self) -> axum::Router {
        cratebase_server::router(self.app.clone())
    }

    async fn request(
        &self,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", token);
        }
        let resp: Response = self
            .router()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .expect("response");
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, value)
    }

    async fn create_user(&self, email: &str) -> String {
        let collection = self.app.db().collections.get("users").expect("users");
        let mut record = cratebase_core::Record::new(collection);
        record.set("email", Value::String(email.to_string()));
        record.set("password", Value::String("supersecret123".into()));
        cratebase_db::records::create(self.app.db(), &self.app.db().collections, &mut record)
            .await
            .expect("create user");
        record.id().to_string()
    }

    async fn enable_magic_link(&self, duration: Option<i64>) {
        let mut magic_link = json!({ "enabled": true });
        if let Some(d) = duration {
            magic_link["duration"] = json!(d);
        }
        let (status, resp) = self
            .request(
                "PATCH",
                "/api/collections/users",
                Some(&self.superuser_token),
                json!({ "magicLink": magic_link }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{resp:?}");
    }

    /// Extracts `?token=...`/`&token=...` from a captured mail's link.
    fn extract_token(html: &str) -> String {
        let start = html.find("token=").expect("token in mail") + "token=".len();
        let rest = &html[start..];
        let end = rest.find(['"', '&', '<']).unwrap_or(rest.len());
        rest[..end].to_string()
    }
}

#[tokio::test]
async fn full_flow_request_then_auth_with_magic_link() {
    let h = Harness::new().await;
    h.enable_magic_link(None).await;
    h.create_user("user@example.com").await;

    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/request-magic-link",
            None,
            json!({ "email": "user@example.com" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let mailbox = h.app.mailer().dev_mailbox().expect("dev mailbox");
    assert_eq!(mailbox.len(), 1);
    let token = Harness::extract_token(&mailbox.list()[0].html);
    assert!(!token.is_empty());

    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/auth-with-magic-link",
            None,
            json!({ "token": token }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(body["token"].as_str().is_some_and(|t| !t.is_empty()));
    assert_eq!(body["record"]["email"], "user@example.com");
    assert_eq!(
        body["record"]["verified"], true,
        "a successful magic-link login marks the record verified"
    );
}

#[tokio::test]
async fn a_reused_token_fails_the_second_time() {
    let h = Harness::new().await;
    h.enable_magic_link(None).await;
    h.create_user("user2@example.com").await;
    h.request(
        "POST",
        "/api/collections/users/request-magic-link",
        None,
        json!({ "email": "user2@example.com" }),
    )
    .await;
    let mailbox = h.app.mailer().dev_mailbox().expect("dev mailbox");
    let token = Harness::extract_token(&mailbox.list()[0].html);

    let (status, _) = h
        .request(
            "POST",
            "/api/collections/users/auth-with-magic-link",
            None,
            json!({ "token": token }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/auth-with-magic-link",
            None,
            json!({ "token": token }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test]
async fn an_expired_token_is_rejected() {
    let h = Harness::new().await;
    h.enable_magic_link(Some(0)).await;
    h.create_user("user3@example.com").await;
    h.request(
        "POST",
        "/api/collections/users/request-magic-link",
        None,
        json!({ "email": "user3@example.com" }),
    )
    .await;
    let mailbox = h.app.mailer().dev_mailbox().expect("dev mailbox");
    let token = Harness::extract_token(&mailbox.list()[0].html);

    // `duration.max(1)` gives a 1-second window; `age.num_seconds()`
    // truncates, so sleeping just past 2s (not 1s) reliably lands past it
    // without needing to fake the clock.
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;

    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/auth-with-magic-link",
            None,
            json!({ "token": token }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test]
async fn disabled_is_rejected_on_both_endpoints() {
    let h = Harness::new().await;
    h.create_user("user4@example.com").await;
    let (status, _) = h
        .request(
            "POST",
            "/api/collections/users/request-magic-link",
            None,
            json!({ "email": "user4@example.com" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = h
        .request(
            "POST",
            "/api/collections/users/auth-with-magic-link",
            None,
            json!({ "token": "whatever" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_unknown_email_is_still_ok_and_sends_no_mail() {
    let h = Harness::new().await;
    h.enable_magic_link(None).await;

    let (status, _) = h
        .request(
            "POST",
            "/api/collections/users/request-magic-link",
            None,
            json!({ "email": "nobody@example.com" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(h.app.mailer().dev_mailbox().unwrap().is_empty());
}

#[tokio::test]
async fn auth_methods_reports_magic_link() {
    let h = Harness::new().await;
    let (status, body) = h
        .request(
            "GET",
            "/api/collections/users/auth-methods",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["magicLink"]["enabled"], false);
    assert_eq!(body["magicLink"]["duration"], 0);

    h.enable_magic_link(Some(600)).await;
    let (status, body) = h
        .request(
            "GET",
            "/api/collections/users/auth-methods",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["magicLink"]["enabled"], true);
    assert_eq!(body["magicLink"]["duration"], 600);
}
