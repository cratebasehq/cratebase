//! The admin dashboard, embedded in the binary (spec §13).
//!
//! `web/admin/dist` is committed, as PocketBase commits `ui/dist`, so a
//! plain `cargo build` produces one self-contained executable with no
//! separate frontend to host. The bundle is served at `/_/` (PocketBase's
//! path, which the SDK's `getDownloadURL`-style helpers assume) and `/`
//! redirects there.
//!
//! The email templates that used to be embedded here are gone: templates
//! now live on the collection and are rendered with
//! [`cratebase_mailer::render_template`].

use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::RustEmbed;

use crate::app::App;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/admin/dist"]
struct DashboardAssets;

pub fn router() -> Router<App> {
    Router::new()
        .route("/", get(|| async { Redirect::temporary("/_/") }))
        .route("/_/", get(serve))
        .route("/_/{*path}", get(serve))
}

/// Serves `path` from the embedded bundle, falling back to `index.html`
/// so the dashboard's client-side routes resolve to the SPA shell.
async fn serve(State(app): State<App>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches("/_/").trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // `--publicDir` lets an operator override the embedded bundle without
    // rebuilding, which is how PocketBase's `--publicDir` behaves.
    if let Some(dir) = &app.config().public_dir {
        let candidate = std::path::Path::new(dir).join(path);
        if candidate.starts_with(dir) {
            if let Ok(bytes) = tokio::fs::read(&candidate).await {
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                return (
                    [(header::CONTENT_TYPE, mime.essence_str().to_string())],
                    bytes,
                )
                    .into_response();
            }
        }
    }

    match DashboardAssets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.essence_str().to_string())],
                file.data,
            )
                .into_response()
        }
        None => match DashboardAssets::get("index.html") {
            Some(file) => ([(header::CONTENT_TYPE, "text/html")], file.data).into_response(),
            None => (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "text/plain")],
                "dashboard not built — run `bun run admin:build`",
            )
                .into_response(),
        },
    }
}
