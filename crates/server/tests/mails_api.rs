//! End-to-end tests for `POST /api/mails/send` and `/preview`: auth
//! (anon/user/API key/superuser), recipient validation, `_mailLog` rows,
//! preview (no send), and the queue-enabled delivery path.

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

    async fn send(&self, request: Request<Body>) -> Response {
        self.router().oneshot(request).await.expect("response")
    }

    async fn post(&self, uri: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, token, body).await
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
        let resp = self
            .send(builder.body(Body::from(body.to_string())).unwrap())
            .await;
        split(resp).await
    }

    async fn get(&self, uri: &str, token: &str) -> (StatusCode, Value) {
        let resp = self
            .send(
                Request::get(uri)
                    .header("authorization", token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        split(resp).await
    }

    /// A regular (non-superuser) `users` record with a valid session
    /// token.
    async fn create_user(&self, email: &str) -> String {
        let collection = self.app.db().collections.get("users").expect("users");
        let mut record = cratebase_core::Record::new(collection);
        record.set("email", Value::String(email.to_string()));
        record.set("password", Value::String("supersecret123".into()));
        record.set("verified", Value::Bool(true));
        cratebase_db::records::create(self.app.db(), &self.app.db().collections, &mut record)
            .await
            .expect("create user");
        self.app
            .mint_token("users", record.id(), cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token")
    }

    /// A fresh, enabled API key, minted through the real
    /// `POST /api/api-keys` endpoint.
    async fn create_api_key(&self) -> String {
        let (status, body) = self
            .post(
                "/api/api-keys",
                Some(&self.superuser_token),
                json!({ "name": "test key" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body:?}");
        body["key"].as_str().expect("key").to_string()
    }
}

async fn split(response: Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

fn raw_send_body() -> Value {
    json!({
        "to": "someone@example.com",
        "subject": "Hello",
        "html": "<p>Hi</p>",
    })
}

#[tokio::test]
async fn anonymous_is_unauthorized() {
    let h = Harness::new().await;
    let (status, _) = h.post("/api/mails/send", None, raw_send_body()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_regular_user_is_forbidden() {
    let h = Harness::new().await;
    let user_token = h.create_user("user@example.com").await;
    let (status, _) = h
        .post("/api/mails/send", Some(&user_token), raw_send_body())
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_api_key_can_send() {
    let h = Harness::new().await;
    let key = h.create_api_key().await;
    let (status, body) = h.post("/api/mails/send", Some(&key), raw_send_body()).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "sent");
}

#[tokio::test]
async fn a_superuser_can_send_and_it_lands_in_the_dev_mailbox_and_mail_log() {
    let h = Harness::new().await;
    let (status, body) = h
        .post("/api/mails/send", Some(&h.superuser_token), raw_send_body())
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "sent");
    let log_id = body["id"].as_str().expect("id").to_string();

    let mailbox = h.app.mailer().dev_mailbox().expect("dev mailbox");
    assert_eq!(mailbox.len(), 1);
    assert_eq!(mailbox.list()[0].subject, "Hello");

    let (status, log) = h
        .get(
            &format!("/api/collections/_mailLog/records/{log_id}"),
            &h.superuser_token,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{log:?}");
    assert_eq!(log["status"], "sent");
    assert_eq!(log["subject"], "Hello");
    assert_eq!(log["to"][0]["address"], "someone@example.com");
}

#[tokio::test]
async fn a_template_send_renders_through_email_templates_and_uses_data() {
    let h = Harness::new().await;
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&h.superuser_token),
            json!({
                "to": "someone@example.com",
                "template": "welcome",
                "data": { "user": { "name": "Bob" } },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "sent");
    let mailbox = h.app.mailer().dev_mailbox().expect("dev mailbox");
    assert!(mailbox.list()[0].html.contains("Bob"));
}

#[tokio::test]
async fn recipients_are_capped_at_fifty() {
    let h = Harness::new().await;
    let to: Vec<Value> = (0..51)
        .map(|i| json!(format!("user{i}@example.com")))
        .collect();
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&h.superuser_token),
            json!({ "to": Value::Array(to), "subject": "S", "html": "<p>h</p>" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test]
async fn an_invalid_address_is_rejected() {
    let h = Harness::new().await;
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&h.superuser_token),
            json!({ "to": "not-an-email", "subject": "S", "html": "<p>h</p>" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test]
async fn preview_renders_without_sending() {
    let h = Harness::new().await;
    let (status, body) = h
        .post(
            "/api/mails/preview",
            Some(&h.superuser_token),
            json!({
                "template": "welcome",
                "data": { "user": { "name": "Ada" } },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(body["html"].as_str().unwrap().contains("Ada"));
    assert!(body["subject"].as_str().is_some());
    assert!(
        h.app.mailer().dev_mailbox().unwrap().is_empty(),
        "preview must not send"
    );
}

#[tokio::test]
async fn unknown_template_key_is_a_bad_request() {
    let h = Harness::new().await;
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&h.superuser_token),
            json!({ "to": "a@example.com", "template": "does-not-exist" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test]
async fn when_the_queue_is_enabled_send_enqueues_instead_of_sending_inline() {
    let h = Harness::new().await;
    // `settings.queue.enabled` is only read at boot for plugin
    // registration (see `App::bootstrap_inner`), but `mails::send`'s own
    // branch reads it live — so flipping it via PATCH plus manually
    // provisioning `_queue_jobs` (what the boot-time gate would have
    // done) is enough to exercise the enqueue path without a real
    // worker tick.
    let (status, resp) = h
        .request(
            "PATCH",
            "/api/settings",
            Some(&h.superuser_token),
            json!({ "queue": { "enabled": true } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{resp:?}");
    cratebase_server::queue::ensure_collection(&h.app)
        .await
        .expect("ensure _queue_jobs");

    let (status, body) = h
        .post("/api/mails/send", Some(&h.superuser_token), raw_send_body())
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "queued");
    assert!(
        h.app.mailer().dev_mailbox().unwrap().is_empty(),
        "queued mail is not sent inline"
    );

    let (status, jobs) = h
        .get(
            "/api/collections/_queue_jobs/records?filter=queue='mail:send'",
            &h.superuser_token,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{jobs:?}");
    assert_eq!(jobs["items"].as_array().unwrap().len(), 1);
    assert_eq!(jobs["items"][0]["status"], "pending");
}
