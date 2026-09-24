//! `GET /api/typegen` — superuser-only download of the generated
//! TypeScript types.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use cratebase_server::app::App;
use cratebase_server::config::Config;
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

    fn router(&self) -> axum::Router {
        cratebase_server::router(self.app.clone())
    }
}

#[tokio::test]
async fn requires_superuser_auth() {
    let harness = Harness::new().await;
    let response = harness
        .router()
        .oneshot(Request::get("/api/typegen").body(Body::empty()).unwrap())
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn returns_generated_types_as_a_downloadable_typescript_file() {
    let harness = Harness::new().await;
    let response = harness
        .router()
        .oneshot(
            Request::get("/api/typegen")
                .header("authorization", &harness.token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .expect("content-type")
        .to_str()
        .unwrap()
        .to_string();
    assert!(content_type.contains("typescript"));
    let disposition = response
        .headers()
        .get(axum::http::header::CONTENT_DISPOSITION)
        .expect("content-disposition")
        .to_str()
        .unwrap()
        .to_string();
    assert!(disposition.contains("attachment"));
    assert!(disposition.contains("cratebase-types.d.ts"));

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("export interface UsersRecord {"));
    assert!(body.contains("export interface Schema {"));
    assert!(body.contains("export interface SchemaCreate {"));
}
