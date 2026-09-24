//! `GET /api/dev/mails`, `GET /api/dev/mails/{id}`, `DELETE /api/dev/mails`
//! — superuser-only access to the dev mail inbox
//! (`cratebase_mailer::DevMailbox`) captured by the `Log` mail backend.
//!
//! Every handler here 404s the moment the configured mailer isn't
//! actually using the `Log` backend (SMTP or a real transport
//! configured) — `App::mailer().dev_mailbox()` is `None` in that case —
//! so real outgoing mail is never reachable through this API and the
//! dashboard can treat a 404 as "no dev inbox" with no separate feature
//! flag round trip.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use cratebase_mailer::CapturedMail;

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiResult};

pub fn router() -> Router<App> {
    Router::new()
        .route("/dev/mails", get(list).delete(clear))
        .route("/dev/mails/{id}", get(view))
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MailAddress {
    pub address: String,
    pub name: String,
}

fn address(a: &cratebase_mailer::Address) -> MailAddress {
    MailAddress {
        address: a.0.clone(),
        name: a.1.clone(),
    }
}

/// A mail as it appears in the list — no `html`/`text`, so browsing the
/// inbox never has to download every captured body up front.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MailSummary {
    pub id: String,
    pub to: Vec<MailAddress>,
    pub from: MailAddress,
    pub subject: String,
    pub sent_at: cratebase_core::DateTime,
}

impl From<&CapturedMail> for MailSummary {
    fn from(mail: &CapturedMail) -> Self {
        MailSummary {
            id: mail.id.clone(),
            to: mail.to.iter().map(address).collect(),
            from: address(&mail.from),
            subject: mail.subject.clone(),
            sent_at: mail.sent_at,
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MailDetail {
    pub id: String,
    pub to: Vec<MailAddress>,
    pub from: MailAddress,
    pub subject: String,
    pub html: String,
    pub text: Option<String>,
    pub sent_at: cratebase_core::DateTime,
}

impl From<&CapturedMail> for MailDetail {
    fn from(mail: &CapturedMail) -> Self {
        MailDetail {
            id: mail.id.clone(),
            to: mail.to.iter().map(address).collect(),
            from: address(&mail.from),
            subject: mail.subject.clone(),
            html: mail.html.clone(),
            text: mail.text.clone(),
            sent_at: mail.sent_at,
        }
    }
}

#[derive(Debug, Serialize)]
struct MailList {
    items: Vec<MailSummary>,
}

/// The 404 every handler here returns once the mailer isn't backed by
/// `Log` — deliberately identical to "no such route", not a 403 or an
/// empty list, so nothing here hints at *why* it's unavailable to a
/// caller who isn't a superuser to begin with (that's `RequireSuperuser`'s
/// job, checked first in every handler).
fn no_dev_inbox() -> ApiError {
    ApiError::not_found("Not found.")
}

async fn list(State(app): State<App>, _su: RequireSuperuser) -> ApiResult<Json<MailList>> {
    let mailbox = app.mailer().dev_mailbox().ok_or_else(no_dev_inbox)?;
    let items = mailbox.list().iter().map(MailSummary::from).collect();
    Ok(Json(MailList { items }))
}

async fn view(
    State(app): State<App>,
    _su: RequireSuperuser,
    Path(id): Path<String>,
) -> ApiResult<Json<MailDetail>> {
    let mailbox = app.mailer().dev_mailbox().ok_or_else(no_dev_inbox)?;
    let mail = mailbox
        .get(&id)
        .ok_or_else(|| ApiError::not_found("mail not found."))?;
    Ok(Json(MailDetail::from(&mail)))
}

async fn clear(State(app): State<App>, _su: RequireSuperuser) -> ApiResult<StatusCode> {
    let mailbox = app.mailer().dev_mailbox().ok_or_else(no_dev_inbox)?;
    mailbox.clear();
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use cratebase_core::settings::Smtp;
    use cratebase_core::Record;
    use serde_json::{json, Value};
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

    async fn seed_user(app: &App, email: &str) -> String {
        let users = app.db().collections.get_by_name("users").expect("users");
        let mut record = Record::new(users);
        record.set("email", Value::String(email.into()));
        record.set("password", Value::String("whatever-password".into()));
        record.set("tokenKey", Value::String(crate::app::new_token_key()));
        cratebase_db::records::create(app.db(), &app.db().collections, &mut record)
            .await
            .expect("seed user");
        record.id().to_string()
    }

    fn empty_request(method: &str, uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    fn anon_request(method: &str, uri: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    fn json_request(method: &str, uri: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn json_body(response: axum::response::Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// End-to-end: no SMTP configured (the default), trigger a real
    /// password-reset email through the ordinary auth API, then find it
    /// in the dev inbox with its reset token link intact.
    #[tokio::test]
    async fn password_reset_email_shows_up_in_the_dev_inbox_with_its_token() {
        let (app, _dir) = test_app().await;
        let admin_token = superuser_token(&app).await;
        seed_user(&app, "reset-me@example.com").await;
        let router = crate::routes::api_router(&app).with_state(app.clone());

        let reset_response = router
            .clone()
            .oneshot(json_request(
                "POST",
                "/collections/users/request-password-reset",
                json!({ "email": "reset-me@example.com" }),
            ))
            .await
            .unwrap();
        assert_eq!(reset_response.status(), StatusCode::NO_CONTENT);

        let list_response = router
            .clone()
            .oneshot(empty_request("GET", "/dev/mails", &admin_token))
            .await
            .unwrap();
        assert_eq!(list_response.status(), StatusCode::OK);
        let list_body = json_body(list_response).await;
        let items = list_body["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["to"][0]["address"], json!("reset-me@example.com"));
        // The list is a summary — no bodies.
        assert!(items[0].get("html").is_none());
        let id = items[0]["id"].as_str().unwrap().to_string();

        let detail_response = router
            .clone()
            .oneshot(empty_request("GET", &format!("/dev/mails/{id}"), &admin_token))
            .await
            .unwrap();
        assert_eq!(detail_response.status(), StatusCode::OK);
        let detail = json_body(detail_response).await;
        assert!(
            detail["html"]
                .as_str()
                .unwrap()
                .contains("confirm-password-reset/"),
            "expected a reset token link in the captured html: {detail}"
        );

        let clear_response = router
            .clone()
            .oneshot(empty_request("DELETE", "/dev/mails", &admin_token))
            .await
            .unwrap();
        assert_eq!(clear_response.status(), StatusCode::NO_CONTENT);

        let after_clear = router
            .oneshot(empty_request("GET", "/dev/mails", &admin_token))
            .await
            .unwrap();
        let after_clear_body = json_body(after_clear).await;
        assert_eq!(after_clear_body["items"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn anonymous_caller_gets_401() {
        let (app, _dir) = test_app().await;
        let router = crate::routes::api_router(&app).with_state(app);

        let response = router
            .oneshot(anon_request("GET", "/dev/mails"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn non_superuser_gets_403() {
        let (app, _dir) = test_app().await;
        let users = app.db().collections.get_by_name("users").expect("users");
        let mut record = Record::new(users);
        record.set("email", Value::String("plain@example.com".into()));
        record.set("password", Value::String("whatever-password".into()));
        record.set("tokenKey", Value::String(crate::app::new_token_key()));
        cratebase_db::records::create(app.db(), &app.db().collections, &mut record)
            .await
            .unwrap();
        let token = app
            .mint_token("users", record.id(), cratebase_auth::TokenType::Auth, 3600)
            .await
            .unwrap();
        let router = crate::routes::api_router(&app).with_state(app);

        let response = router
            .oneshot(empty_request("GET", "/dev/mails", &token))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    /// Once SMTP is configured the mailer no longer runs the `Log`
    /// backend, so the dev inbox routes must disappear rather than serve
    /// (now stale, and potentially real) captured mail.
    #[tokio::test]
    async fn routes_404_once_smtp_is_configured() {
        let (app, _dir) = test_app().await;
        let admin_token = superuser_token(&app).await;

        let mut settings = (*app.settings()).clone();
        settings.smtp = Smtp {
            enabled: true,
            host: "smtp.example.com".into(),
            ..Smtp::default()
        };
        app.set_settings(settings).await.expect("save settings");

        let router = crate::routes::api_router(&app).with_state(app);

        let list_response = router
            .clone()
            .oneshot(empty_request("GET", "/dev/mails", &admin_token))
            .await
            .unwrap();
        assert_eq!(list_response.status(), StatusCode::NOT_FOUND);

        let view_response = router
            .clone()
            .oneshot(empty_request("GET", "/dev/mails/whatever00000000000000", &admin_token))
            .await
            .unwrap();
        assert_eq!(view_response.status(), StatusCode::NOT_FOUND);

        let clear_response = router
            .oneshot(empty_request("DELETE", "/dev/mails", &admin_token))
            .await
            .unwrap();
        assert_eq!(clear_response.status(), StatusCode::NOT_FOUND);
    }
}
