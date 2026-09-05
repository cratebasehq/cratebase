//! `settings.teams.enabled` actually gates `crate::teams::bind_hooks`:
//! with the flag off (the default), creating a `_teams` row never gets a
//! bootstrap `_team_members` owner row, because the reactive hook was
//! never bound at all. With it on, the existing bootstrap-owner behavior
//! works exactly as documented in `crates/server/src/teams.rs`.

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
    token: String,
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
        let token = app
            .mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token");
        Harness {
            app,
            token,
            _dir: dir,
        }
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, Value) {
        let response = cratebase_server::router(self.app.clone())
            .oneshot(request)
            .await
            .expect("response");
        split(response).await
    }

    async fn as_user(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
        token: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header("authorization", token);
        }
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        self.send(request).await
    }

    async fn admin(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.as_user(method, uri, body, Some(&self.token)).await
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

/// Registers a `users` record (open registration, `default_users`'s own
/// `createRule`) and returns its id and a session token.
async fn register_user(harness: &Harness, email: &str) -> (String, String) {
    let (status, created) = harness
        .as_user(
            "POST",
            "/api/collections/users/records",
            Some(json!({
                "email": email,
                "password": "hunter2hunter2",
                "passwordConfirm": "hunter2hunter2",
            })),
            None,
        )
        .await;
    assert_eq!(status, 200, "{created}");
    let id = created["id"].as_str().unwrap().to_string();

    let (status, auth) = harness
        .as_user(
            "POST",
            "/api/collections/users/auth-with-password",
            Some(json!({"identity": email, "password": "hunter2hunter2"})),
            None,
        )
        .await;
    assert_eq!(status, 200, "{auth}");
    (id, auth["token"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn a_disabled_teams_module_never_bootstraps_the_owner_membership() {
    let harness = Harness::new().await;
    assert!(
        !harness.app.settings().teams.enabled,
        "teams.enabled defaults to false"
    );
    let (user_id, user_token) = register_user(&harness, "owner1@example.com").await;

    let (status, team) = harness
        .as_user(
            "POST",
            "/api/collections/_teams/records",
            Some(json!({"name": "Acme", "ownerRef": user_id})),
            Some(&user_token),
        )
        .await;
    assert_eq!(status, 200, "{team}");
    let team_id = team["id"].as_str().unwrap().to_string();

    // With the hook never bound, no bootstrap owner row was inserted — a
    // direct superuser query proves it (the caller has no membership row
    // to satisfy `_team_members`'s own list rule).
    let (status, rows) = harness
        .admin(
            "GET",
            &format!("/api/collections/_team_members/records?filter=teamRef='{team_id}'"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{rows}");
    assert_eq!(
        rows["items"].as_array().unwrap().len(),
        0,
        "disabled teams module must not bootstrap an owner membership row"
    );
}

#[tokio::test]
async fn an_enabled_teams_module_bootstraps_the_owner_membership_as_before() {
    let harness = Harness::new().await;
    let mut settings = (*harness.app.settings()).clone();
    settings.teams.enabled = true;
    harness
        .app
        .set_settings(settings)
        .await
        .expect("enable teams");
    cratebase_server::teams::bind_hooks(&harness.app);

    let (user_id, user_token) = register_user(&harness, "owner2@example.com").await;

    let (status, team) = harness
        .as_user(
            "POST",
            "/api/collections/_teams/records",
            Some(json!({"name": "Acme", "ownerRef": user_id})),
            Some(&user_token),
        )
        .await;
    assert_eq!(status, 200, "{team}");
    let team_id = team["id"].as_str().unwrap().to_string();

    let (status, rows) = harness
        .admin(
            "GET",
            &format!("/api/collections/_team_members/records?filter=teamRef='{team_id}'"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{rows}");
    let items = rows["items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        1,
        "should have bootstrapped exactly one owner row"
    );
    assert_eq!(items[0]["userRef"], user_id);
    assert_eq!(items[0]["role"], "owner");
}
