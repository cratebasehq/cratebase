//! `--dev` + `CB_TYPEGEN_OUT`: the generated TypeScript types are
//! rewritten to that path after every collection create/update/delete
//! (`crate::routes::collections::refresh_typegen_watch`).

use axum::body::Body;
use axum::http::Request;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use serde_json::json;
use tower::ServiceExt;

const SUPERUSER_EMAIL: &str = "admin@example.com";
const SUPERUSER_PASSWORD: &str = "hunter2hunter2";

struct Harness {
    app: App,
    token: String,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn with(configure: impl FnOnce(&mut Config)) -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut config = Config::memory(dir.path());
        configure(&mut config);
        let app = App::new(config);
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

    async fn create_posts_collection(&self) {
        let body = json!({
            "name": "posts",
            "type": "base",
            "fields": [{"name": "title", "type": "text"}],
        });
        let response = cratebase_server::router(self.app.clone())
            .oneshot(
                Request::post("/api/collections")
                    .header("authorization", &self.token)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .expect("response");
        assert!(
            response.status().is_success(),
            "collection create failed: {:?}",
            response.status()
        );
    }
}

#[tokio::test]
async fn rewrites_the_typegen_file_on_collection_create_when_dev_and_out_are_set() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out_path = dir.path().join("watched-types.d.ts");
    let harness = Harness::with(|config| {
        config.dev = true;
        config.typegen_out = Some(out_path.to_string_lossy().into_owned());
    })
    .await;

    assert!(!out_path.exists());
    harness.create_posts_collection().await;

    let generated = std::fs::read_to_string(&out_path).expect("typegen watch file written");
    assert!(generated.contains("export type PostsRecord = {"));
    assert!(generated.contains("export type Schema = {"));
}

#[tokio::test]
async fn does_not_write_when_typegen_out_is_unset() {
    let dir = tempfile::tempdir().expect("temp dir");
    let harness = Harness::with(|config| {
        config.dev = true;
        config.typegen_out = None;
    })
    .await;

    harness.create_posts_collection().await;

    assert!(!dir.path().join("cratebase-types.d.ts").exists());
}

#[tokio::test]
async fn does_not_write_when_dev_is_off_even_if_out_is_set() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out_path = dir.path().join("watched-types.d.ts");
    let harness = Harness::with(|config| {
        config.dev = false;
        config.typegen_out = Some(out_path.to_string_lossy().into_owned());
    })
    .await;

    harness.create_posts_collection().await;

    assert!(!out_path.exists());
}
