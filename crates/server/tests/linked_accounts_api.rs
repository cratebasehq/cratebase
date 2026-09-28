//! `GET/DELETE .../records/{id}/external-auths[/{provider}]`: listing and
//! unlinking a record's linked OAuth2 providers, including the
//! "don't strand the account" refusal `unlinkExternalAuth` layers on top
//! of PocketBase's own version of this endpoint.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_db::Executor;
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

    /// A `users` record, with a password only when `password` is
    /// `Some` — a record created with `None` has no way to sign in
    /// except whatever the test links/enables afterwards, exercising
    /// `unlink_external_auth`'s "don't strand the account" check. The
    /// `password` field is required to create a record at all (default
    /// `passwordAuth`), so `None` creates one normally and then blanks
    /// the stored hash directly — there is no supported API to create a
    /// genuinely passwordless auth record.
    async fn create_user(&self, email: &str, password: Option<&str>) -> String {
        let collection = self.app.db().collections.get("users").expect("users");
        let mut record = cratebase_core::Record::new(collection);
        record.set("email", Value::String(email.to_string()));
        record.set(
            "password",
            Value::String(password.unwrap_or("temporary-placeholder-1").to_string()),
        );
        cratebase_db::records::create(self.app.db(), &self.app.db().collections, &mut record)
            .await
            .expect("create user");
        let id = record.id().to_string();
        if password.is_none() {
            self.app
                .db()
                .execute(
                    r#"UPDATE "users" SET "password" = '' WHERE "id" = $1"#,
                    &[cratebase_db::engine::Sql::Text(id.clone())],
                )
                .await
                .expect("blank password");
        }
        id
    }

    async fn token_for(&self, id: &str) -> String {
        self.app
            .mint_token("users", id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token")
    }

    /// Links `provider` to `record_id` directly, bypassing the real
    /// OAuth2 dance entirely (the login flow itself is covered by
    /// `crates/auth/src/oauth2.rs`'s and `routes::oidc`'s own tests) —
    /// this file only needs a linked row to exist.
    async fn link_provider(&self, record_id: &str, provider: &str, provider_id: &str) {
        let externals = self
            .app
            .db()
            .collections
            .get("_externalAuths")
            .expect("_externalAuths");
        let mut row = cratebase_core::Record::new(externals);
        // `resolve_oauth2_record` stores the auth collection's *id*, not
        // its name, in `collectionRef` — mirror that exactly.
        let users_id = self.app.db().collections.get("users").unwrap().id.clone();
        row.set("collectionRef", Value::String(users_id));
        row.set("recordRef", Value::String(record_id.to_string()));
        row.set("provider", Value::String(provider.to_string()));
        row.set("providerId", Value::String(provider_id.to_string()));
        cratebase_db::records::create(self.app.db(), &self.app.db().collections, &mut row)
            .await
            .expect("link provider");
    }

    async fn enable_otp(&self) {
        let (status, resp) = self
            .request(
                "PATCH",
                "/api/collections/users",
                Some(&self.superuser_token),
                json!({ "otp": { "enabled": true } }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{resp:?}");
    }
}

#[tokio::test]
async fn owner_can_list_their_own_linked_accounts() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com", Some("hunter2hunter2")).await;
    h.link_provider(&id, "google", "google-sub-1").await;
    h.link_provider(&id, "github", "12345").await;
    let token = h.token_for(&id).await;

    let (status, body) = h
        .request(
            "GET",
            &format!("/api/collections/users/records/{id}/external-auths"),
            Some(&token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let items = body.as_array().expect("bare array response");
    assert_eq!(items.len(), 2);
    let providers: Vec<&str> = items.iter().map(|i| i["provider"].as_str().unwrap()).collect();
    assert!(providers.contains(&"google"));
    assert!(providers.contains(&"github"));
}

#[tokio::test]
async fn another_records_own_account_cannot_list_someone_elses() {
    let h = Harness::new().await;
    let victim = h.create_user("victim@example.com", Some("hunter2hunter2")).await;
    h.link_provider(&victim, "google", "google-sub-1").await;
    let attacker = h.create_user("attacker@example.com", Some("hunter2hunter2")).await;
    let attacker_token = h.token_for(&attacker).await;

    let (status, _) = h
        .request(
            "GET",
            &format!("/api/collections/users/records/{victim}/external-auths"),
            Some(&attacker_token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_superuser_can_list_and_unlink_anyones_linked_accounts() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com", Some("hunter2hunter2")).await;
    h.link_provider(&id, "google", "google-sub-1").await;

    let (status, body) = h
        .request(
            "GET",
            &format!("/api/collections/users/records/{id}/external-auths"),
            Some(&h.superuser_token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body.as_array().unwrap().len(), 1);

    let (status, body) = h
        .request(
            "DELETE",
            &format!("/api/collections/users/records/{id}/external-auths/google"),
            Some(&h.superuser_token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body:?}");
}

#[tokio::test]
async fn unlinking_is_allowed_when_the_record_has_a_password() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com", Some("hunter2hunter2")).await;
    h.link_provider(&id, "google", "google-sub-1").await;
    let token = h.token_for(&id).await;

    let (status, body) = h
        .request(
            "DELETE",
            &format!("/api/collections/users/records/{id}/external-auths/google"),
            Some(&token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body:?}");
}

#[tokio::test]
async fn unlinking_is_allowed_when_another_provider_remains() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com", None).await;
    h.link_provider(&id, "google", "google-sub-1").await;
    h.link_provider(&id, "github", "12345").await;
    let token = h.token_for(&id).await;

    let (status, body) = h
        .request(
            "DELETE",
            &format!("/api/collections/users/records/{id}/external-auths/google"),
            Some(&token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body:?}");
}

#[tokio::test]
async fn unlinking_is_allowed_when_otp_login_is_enabled() {
    let h = Harness::new().await;
    h.enable_otp().await;
    let id = h.create_user("jo@example.com", None).await;
    h.link_provider(&id, "google", "google-sub-1").await;
    let token = h.token_for(&id).await;

    let (status, body) = h
        .request(
            "DELETE",
            &format!("/api/collections/users/records/{id}/external-auths/google"),
            Some(&token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body:?}");
}

#[tokio::test]
async fn unlinking_the_only_way_to_sign_in_is_refused() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com", None).await;
    h.link_provider(&id, "google", "google-sub-1").await;
    let token = h.token_for(&id).await;

    let (status, body) = h
        .request(
            "DELETE",
            &format!("/api/collections/users/records/{id}/external-auths/google"),
            Some(&token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");

    // Confirmed still linked afterwards -- the refusal didn't half-apply.
    let (status, body) = h
        .request(
            "GET",
            &format!("/api/collections/users/records/{id}/external-auths"),
            Some(&token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body.as_array().unwrap().len(), 1);
}
