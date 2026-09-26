//! End-to-end proof that `request-verification` (and, by the same code
//! path, every other auth-flow mail) is actually wired through
//! `crate::mail_templates::resolve_auth_mail`, not just covered by that
//! module's own unit tests: editing the seeded `_emailTemplates` row
//! changes what a real `POST .../request-verification` call sends, and a
//! customized `authOptions.verificationTemplate` still wins over it.

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

    async fn create_unverified_user(&self, email: &str) -> String {
        let collection = self.app.db().collections.get("users").expect("users");
        let mut record = cratebase_core::Record::new(collection);
        record.set("email", Value::String(email.to_string()));
        record.set("password", Value::String("supersecret123".into()));
        cratebase_db::records::create(self.app.db(), &self.app.db().collections, &mut record)
            .await
            .expect("create user");
        record.id().to_string()
    }
}

#[tokio::test]
async fn editing_the_emailtemplates_row_changes_what_request_verification_sends() {
    let h = Harness::new().await;
    h.create_unverified_user("user@example.com").await;

    // Baseline: the seeded row's own wording shows up.
    let (status, _) = h
        .request(
            "POST",
            "/api/collections/users/request-verification",
            None,
            json!({ "email": "user@example.com" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let mailbox = h.app.mailer().dev_mailbox().expect("dev mailbox");
    assert_eq!(mailbox.len(), 1);
    assert!(mailbox.list()[0].html.contains("Thank you for joining us"));

    // Edit the _emailTemplates row for auth.verification.
    let (status, list) = h
        .request(
            "GET",
            "/api/collections/_emailTemplates/records?filter=key='auth.verification'",
            Some(&h.superuser_token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{list:?}");
    let row_id = list["items"][0]["id"].as_str().unwrap().to_string();
    let (status, _) = h
        .request(
            "PATCH",
            &format!("/api/collections/_emailTemplates/records/{row_id}"),
            Some(&h.superuser_token),
            json!({ "html": "<p>Custom verify link: {{token}}</p>" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = h
        .request(
            "POST",
            "/api/collections/users/request-verification",
            None,
            json!({ "email": "user@example.com" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(mailbox.len(), 2);
    // `DevMailbox::list()` is newest-first, so the just-sent mail is [0].
    assert!(mailbox.list()[0].html.contains("Custom verify link:"));
}

#[tokio::test]
async fn a_customized_authoptions_template_still_wins_over_the_emailtemplates_row() {
    let h = Harness::new().await;

    // Customize the users collection's own verificationTemplate away
    // from its default.
    let (status, list) = h
        .request(
            "GET",
            "/api/collections/users",
            Some(&h.superuser_token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{list:?}");
    let (status, _) = h
        .request(
            "PATCH",
            "/api/collections/users",
            Some(&h.superuser_token),
            json!({
                "verificationTemplate": {
                    "subject": "Custom subject",
                    "body": "<p>Custom legacy body {TOKEN}</p>"
                }
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{status}");

    h.create_unverified_user("user2@example.com").await;
    let (status, _) = h
        .request(
            "POST",
            "/api/collections/users/request-verification",
            None,
            json!({ "email": "user2@example.com" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let mailbox = h.app.mailer().dev_mailbox().expect("dev mailbox");
    let mail = mailbox.list();
    let sent = &mail[0];
    assert_eq!(sent.subject, "Custom subject");
    assert!(sent.html.contains("Custom legacy body"));
}
