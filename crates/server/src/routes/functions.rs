//! `GET /api/functions` — superuser only.
//!
//! Read-only view over `pb_hooks/`: which `*.pb.js` files exist and which
//! HTTP routes they registered via `routerAdd` (see `crate::jsvm_host`).
//! There is deliberately no write path here — hooks are edited as files on
//! disk, not through the dashboard (see `ARCHITECTURE.md`'s "Extending
//! with JavaScript" section) — so this module only ever reads
//! `app.config().hooks_dir` and `app.js_routes()`.

use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::app::App;
use crate::extract::RequireSuperuser;

pub fn router() -> Router<App> {
    Router::new().route("/functions", get(list))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HookFile {
    name: String,
    size_bytes: u64,
    modified_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HookRoute {
    method: String,
    pattern: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FunctionsResponse {
    hooks_dir: String,
    files: Vec<HookFile>,
    routes: Vec<HookRoute>,
}

async fn list(
    axum::extract::State(app): axum::extract::State<App>,
    _su: RequireSuperuser,
) -> Json<FunctionsResponse> {
    let hooks_dir = app.config().hooks_dir.clone();
    let mut files = hook_files(&hooks_dir);
    files.sort_by(|a, b| a.name.cmp(&b.name));

    let routes = app
        .js_routes()
        .into_iter()
        .map(|route| HookRoute {
            method: if route.method.is_empty() {
                "ANY".to_string()
            } else {
                route.method
            },
            pattern: route.pattern,
        })
        .collect();

    Json(FunctionsResponse {
        hooks_dir,
        files,
        routes,
    })
}

/// Lists `*.pb.js` files directly inside `dir`. Mirrors
/// `jsvm_host::has_hook_files`'s filter, but collects the matches instead
/// of just checking whether any exist. A missing or unreadable directory
/// is not an error — same no-op philosophy as the rest of the jsvm
/// integration — it just yields no files.
fn hook_files(dir: &str) -> Vec<HookFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter(|entry| {
            entry.file_type().is_ok_and(|t| t.is_file())
                && entry.file_name().to_string_lossy().ends_with(".pb.js")
        })
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let metadata = entry.metadata().ok();
            let size_bytes = metadata.as_ref().map(std::fs::Metadata::len).unwrap_or(0);
            let modified_at = metadata
                .and_then(|m| m.modified().ok())
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
            HookFile {
                name,
                size_bytes,
                modified_at,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
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
            .create_superuser("functions-test@example.com", "password12345")
            .await
            .expect("create superuser");
        app.mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("mint token")
    }

    #[tokio::test]
    async fn missing_hooks_dir_returns_empty_lists() {
        let (app, _dir) = test_app().await;
        // `Config::memory` points `hooks_dir` at a sibling `pb_hooks` that
        // was never created for this temp dir.
        let hooks_dir = app.config().hooks_dir.clone();
        assert!(!std::path::Path::new(&hooks_dir).exists());

        let router = crate::routes::api_router(&app).with_state(app.clone());
        let token = superuser_token(&app).await;

        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/functions")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["hooksDir"], serde_json::Value::String(hooks_dir));
        assert_eq!(json["files"], serde_json::json!([]));
        assert_eq!(json["routes"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn requires_a_superuser_token() {
        let (app, _dir) = test_app().await;
        let router = crate::routes::api_router(&app).with_state(app);

        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/functions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
