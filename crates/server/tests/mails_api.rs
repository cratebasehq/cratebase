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

    /// Sets `_emailTemplates.<key>.sendRule` (`None` → `null`,
    /// superuser-only; `Some("")` → public; `Some(expr)` → a rule) and
    /// returns the row's id.
    async fn set_send_rule(&self, key: &str, rule: Option<&str>) -> String {
        let (status, list) = self
            .get(
                &format!("/api/collections/_emailTemplates/records?filter=key='{key}'"),
                &self.superuser_token,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{list:?}");
        let id = list["items"][0]["id"]
            .as_str()
            .expect("template id")
            .to_string();
        let value = match rule {
            None => Value::Null,
            Some(r) => Value::String(r.to_string()),
        };
        let (status, body) = self
            .request(
                "PATCH",
                &format!("/api/collections/_emailTemplates/records/{id}"),
                Some(&self.superuser_token),
                json!({ "sendRule": value }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body:?}");
        id
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

// Anonymous and regular-user callers are no longer unconditionally
// rejected — `sendRule` (`crate::mail_templates`) decides per template.
// A *raw* send (no `template`) stays refused for everyone but a
// superuser/API key, in both cases as a `403` (not a `401`): anonymous is
// now a legitimate caller shape for this endpoint, just one this
// particular request isn't allowed to make.

#[tokio::test]
async fn anonymous_raw_send_is_forbidden() {
    let h = Harness::new().await;
    let (status, _) = h.post("/api/mails/send", None, raw_send_body()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_regular_user_raw_send_is_forbidden() {
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

// ---------------------------------------------------------- sendRule

#[tokio::test]
async fn a_null_send_rule_denies_a_regular_user() {
    let h = Harness::new().await;
    let user_token = h.create_user("user@example.com").await;
    // The seeded "welcome" template's `sendRule` is `null` by default.
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&user_token),
            json!({ "to": "someone@example.com", "template": "welcome" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body:?}");
}

#[tokio::test]
async fn a_true_send_rule_allows_a_regular_user() {
    let h = Harness::new().await;
    let user_token = h.create_user("user@example.com").await;
    h.set_send_rule("welcome", Some("@request.auth.id != ''"))
        .await;
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&user_token),
            json!({ "to": "someone@example.com", "template": "welcome" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "sent");
}

#[tokio::test]
async fn a_false_send_rule_denies_a_regular_user() {
    let h = Harness::new().await;
    let user_token = h.create_user("user@example.com").await;
    h.set_send_rule("welcome", Some("@request.body.data.vip = true"))
        .await;
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&user_token),
            json!({
                "to": "someone@example.com",
                "template": "welcome",
                "data": { "vip": false },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body:?}");
}

#[tokio::test]
async fn an_empty_send_rule_allows_anonymous_callers() {
    let h = Harness::new().await;
    h.set_send_rule("welcome", Some("")).await;
    let (status, body) = h
        .post(
            "/api/mails/send",
            None,
            json!({ "to": "someone@example.com", "template": "welcome" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "sent");
}

#[tokio::test]
async fn raw_html_by_a_regular_user_is_forbidden_even_with_a_public_send_rule() {
    let h = Harness::new().await;
    let user_token = h.create_user("user@example.com").await;
    // Even a wide-open `sendRule` on some template must not let a
    // non-superuser reach a *raw* content send — `sendRule` only ever
    // governs `template` sends.
    h.set_send_rule("welcome", Some("")).await;
    let (status, body) = h
        .post("/api/mails/send", Some(&user_token), raw_send_body())
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body:?}");
}

#[tokio::test]
async fn a_from_override_by_a_regular_user_is_forbidden() {
    let h = Harness::new().await;
    let user_token = h.create_user("user@example.com").await;
    h.set_send_rule("welcome", Some("")).await;
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&user_token),
            json!({
                "to": "someone@example.com",
                "template": "welcome",
                "from": "spoofed@example.com",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body:?}");
}

#[tokio::test]
async fn cc_and_bcc_overrides_by_a_regular_user_are_forbidden() {
    let h = Harness::new().await;
    let user_token = h.create_user("user@example.com").await;
    h.set_send_rule("welcome", Some("")).await;
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&user_token),
            json!({
                "to": "someone@example.com",
                "template": "welcome",
                "cc": "extra@example.com",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body:?}");
}

#[tokio::test]
async fn a_regular_user_is_capped_at_five_recipients() {
    let h = Harness::new().await;
    let user_token = h.create_user("user@example.com").await;
    h.set_send_rule("welcome", Some("")).await;
    let to: Vec<Value> = (0..6)
        .map(|i| json!(format!("user{i}@example.com")))
        .collect();
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&user_token),
            json!({ "to": Value::Array(to), "template": "welcome" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body:?}");
}

#[tokio::test]
async fn send_rule_is_evaluated_per_recipient_and_every_one_must_pass() {
    let h = Harness::new().await;
    let user_token = h.create_user("user@example.com").await;
    h.set_send_rule("welcome", Some("@request.body.to = 'ok@example.com'"))
        .await;

    // One address the rule accepts.
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&user_token),
            json!({ "to": "ok@example.com", "template": "welcome" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    // Two addresses, only one of which the rule accepts: the whole send
    // is denied, not just filtered down to the address that passed.
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&user_token),
            json!({ "to": ["ok@example.com", "bad@example.com"], "template": "welcome" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body:?}");
}

#[tokio::test]
async fn superusers_and_api_keys_bypass_send_rule_entirely() {
    let h = Harness::new().await;
    // Left `null` (superuser-only) deliberately: a superuser/API key
    // must still be able to send it.
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&h.superuser_token),
            json!({ "to": "someone@example.com", "template": "welcome" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let key = h.create_api_key().await;
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&key),
            json!({ "to": "someone@example.com", "template": "welcome" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
}

#[tokio::test]
async fn the_mails_send_user_rate_limit_tag_applies_to_non_superuser_callers() {
    let h = Harness::new().await;
    let user_token = h.create_user("user@example.com").await;
    h.set_send_rule("welcome", Some("")).await;

    // A bare-bones rule set with *only* a strict `mails:send:user` rule:
    // the default set also has a `/api/` catch-all, and an exact/prefix
    // match always wins over a tag match regardless of specificity
    // (`crate::middleware::rate_limit`'s module doc — "exact path beats
    // prefix beats tag" — and its own `exact_path_beats_prefix_beats_tag`
    // test), so that catch-all would otherwise shadow this tag entirely.
    let mut settings = h.app.settings().as_ref().clone();
    settings.rate_limits = cratebase_core::settings::RateLimits {
        rules: vec![cratebase_core::settings::RateLimitRule {
            label: "mails:send:user".into(),
            audience: "@auth".into(),
            duration: 3600,
            max_requests: 1,
        }],
        excluded_ips: vec![],
        enabled: true,
    };
    h.app.set_settings(settings).await.unwrap();

    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&user_token),
            json!({ "to": "someone@example.com", "template": "welcome" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    // Second call within the window trips the tighter per-user tag.
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&user_token),
            json!({ "to": "someone@example.com", "template": "welcome" }),
        )
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body:?}");

    // A superuser is on the general `mails:send` ceiling only, not this
    // per-user one, and is unaffected by it.
    let (status, body) = h
        .post(
            "/api/mails/send",
            Some(&h.superuser_token),
            json!({ "to": "someone@example.com", "template": "welcome" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
}
