//! End-to-end coverage for TOTP 2FA: setup -> confirm (returns backup
//! codes) -> a password login now returns `401 {mfaId}` even though
//! collection-level `authOptions.mfa` is off -> `auth-with-totp`
//! completes it with either a live code or a backup code (consumed on
//! use) -> disable removes it and logins go back to a single factor.

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
const USER_PASSWORD: &str = "hunter2hunter2";

struct Harness {
    app: App,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        app.create_superuser(SUPERUSER_EMAIL, SUPERUSER_PASSWORD)
            .await
            .expect("superuser");
        Harness { app, _dir: dir }
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
        record.set("password", Value::String(USER_PASSWORD.to_string()));
        cratebase_db::records::create(self.app.db(), &self.app.db().collections, &mut record)
            .await
            .expect("create user");
        record.id().to_string()
    }

    async fn token_for(&self, id: &str) -> String {
        self.app
            .mint_token("users", id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token")
    }

    async fn login(&self, email: &str) -> (StatusCode, Value) {
        self.request(
            "POST",
            "/api/collections/users/auth-with-password",
            None,
            json!({ "identity": email, "password": USER_PASSWORD }),
        )
        .await
    }

    /// Sets up and confirms TOTP for `id`, returning `(secret, backup
    /// codes)`.
    async fn enable_totp(&self, token: &str) -> (String, Vec<String>) {
        let (status, body) = self
            .request(
                "POST",
                "/api/collections/users/totp/setup",
                Some(token),
                Value::Null,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body:?}");
        let secret = body["secret"].as_str().unwrap().to_string();

        let code = cratebase_auth::totp_at(&secret, now()).unwrap();
        let (status, body) = self
            .request(
                "POST",
                "/api/collections/users/totp/confirm",
                Some(token),
                json!({ "code": code }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body:?}");
        let backup_codes: Vec<String> = body["backupCodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(backup_codes.len(), 10);
        (secret, backup_codes)
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[tokio::test]
async fn setup_confirm_and_login_completes_with_a_live_code() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com").await;
    let token = h.token_for(&id).await;
    let (secret, _backup) = h.enable_totp(&token).await;

    // Collection-level MFA was never enabled -- proves the per-record
    // gate fires on its own.
    let (status, body) = h.login("jo@example.com").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body:?}");
    let mfa_id = body["mfaId"].as_str().expect("mfaId").to_string();
    assert!(body.get("token").is_none());

    // `enable_totp`'s own `confirm` call already consumed whatever step
    // `now()` falls in (replay protection) -- a step ahead is
    // guaranteed distinct while still comfortably inside the server's
    // own +/-1 step verification window relative to real time.
    let code = cratebase_auth::totp_at(&secret, now() + cratebase_auth::TOTP_PERIOD_SECS).unwrap();
    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/auth-with-totp",
            None,
            json!({ "mfaId": mfa_id, "code": code }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(body["token"].as_str().is_some_and(|t| !t.is_empty()));
    assert_eq!(body["record"]["email"], "jo@example.com");
}

#[tokio::test]
async fn a_wrong_code_does_not_complete_the_challenge() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com").await;
    let token = h.token_for(&id).await;
    h.enable_totp(&token).await;

    let (_, body) = h.login("jo@example.com").await;
    let mfa_id = body["mfaId"].as_str().unwrap().to_string();

    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/auth-with-totp",
            None,
            json!({ "mfaId": mfa_id, "code": "000000" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test]
async fn a_backup_code_completes_the_challenge_and_is_consumed_on_use() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com").await;
    let token = h.token_for(&id).await;
    let (_secret, backup_codes) = h.enable_totp(&token).await;
    let backup = backup_codes[0].clone();

    let (_, body) = h.login("jo@example.com").await;
    let mfa_id = body["mfaId"].as_str().unwrap().to_string();

    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/auth-with-totp",
            None,
            json!({ "mfaId": mfa_id, "code": backup }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    // Same backup code, a fresh login challenge -- must not work twice.
    let (_, body) = h.login("jo@example.com").await;
    let mfa_id = body["mfaId"].as_str().unwrap().to_string();
    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/auth-with-totp",
            None,
            json!({ "mfaId": mfa_id, "code": backup }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test]
async fn replaying_the_same_totp_code_within_its_window_fails() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com").await;
    let token = h.token_for(&id).await;
    let (secret, _) = h.enable_totp(&token).await;
    // A step ahead of `now()`, same reasoning as above: `enable_totp`'s
    // `confirm` already consumed whatever step `now()` itself falls in.
    let code = cratebase_auth::totp_at(&secret, now() + cratebase_auth::TOTP_PERIOD_SECS).unwrap();

    let (_, body) = h.login("jo@example.com").await;
    let mfa_id = body["mfaId"].as_str().unwrap().to_string();
    let (status, _) = h
        .request(
            "POST",
            "/api/collections/users/auth-with-totp",
            None,
            json!({ "mfaId": mfa_id, "code": code }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // A brand new login challenge, but the *same* still-numerically-valid
    // code -- replay protection must reject it regardless.
    let (_, body) = h.login("jo@example.com").await;
    let mfa_id = body["mfaId"].as_str().unwrap().to_string();
    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/auth-with-totp",
            None,
            json!({ "mfaId": mfa_id, "code": code }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test]
async fn disable_with_the_current_code_removes_totp_and_logins_go_back_to_single_factor() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com").await;
    let token = h.token_for(&id).await;
    let (secret, _) = h.enable_totp(&token).await;

    // A step ahead of `now()`: `enable_totp`'s own `confirm` already
    // consumed whatever step `now()` falls in.
    let code = cratebase_auth::totp_at(&secret, now() + cratebase_auth::TOTP_PERIOD_SECS).unwrap();
    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/totp/disable",
            Some(&token),
            json!({ "code": code }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body:?}");

    let (status, body) = h.login("jo@example.com").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(body["token"].as_str().is_some_and(|t| !t.is_empty()));
}

#[tokio::test]
async fn disable_with_the_password_also_works() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com").await;
    let token = h.token_for(&id).await;
    h.enable_totp(&token).await;

    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/totp/disable",
            Some(&token),
            json!({ "password": USER_PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body:?}");
}

#[tokio::test]
async fn regenerating_backup_codes_invalidates_the_old_ones() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com").await;
    let token = h.token_for(&id).await;
    let (_secret, old_codes) = h.enable_totp(&token).await;

    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/totp/backup-codes/regenerate",
            Some(&token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let new_codes: Vec<String> = body["backupCodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(new_codes.len(), 10);
    assert_ne!(new_codes, old_codes);

    let (_, body) = h.login("jo@example.com").await;
    let mfa_id = body["mfaId"].as_str().unwrap().to_string();
    let (status, _) = h
        .request(
            "POST",
            "/api/collections/users/auth-with-totp",
            None,
            json!({ "mfaId": mfa_id, "code": old_codes[0] }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn setup_twice_while_already_confirmed_is_refused() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com").await;
    let token = h.token_for(&id).await;
    h.enable_totp(&token).await;

    let (status, body) = h
        .request(
            "POST",
            "/api/collections/users/totp/setup",
            Some(&token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test]
async fn a_superuser_can_reset_someones_totp_via_the_generic_collection_api() {
    let h = Harness::new().await;
    let id = h.create_user("jo@example.com").await;
    let token = h.token_for(&id).await;
    h.enable_totp(&token).await;

    let superuser_row = h
        .app
        .db()
        .query_one(r#"SELECT "id" FROM "_superusers" LIMIT 1"#, &[])
        .await
        .unwrap()
        .unwrap();
    let superuser_id = superuser_row.get_str("id").unwrap();
    let superuser_token = h
        .app
        .mint_token(
            "_superusers",
            superuser_id,
            cratebase_auth::TokenType::Auth,
            3600,
        )
        .await
        .unwrap();

    let (status, body) = h
        .request(
            "GET",
            "/api/collections/_totps/records",
            Some(&superuser_token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let row_id = body["items"][0]["id"].as_str().unwrap().to_string();

    let (status, body) = h
        .request(
            "DELETE",
            &format!("/api/collections/_totps/records/{row_id}"),
            Some(&superuser_token),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body:?}");

    // TOTP is gone -- a login now succeeds outright, single factor.
    let (status, body) = h.login("jo@example.com").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let _ = id;
}
