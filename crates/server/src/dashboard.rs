//! Serves the built admin dashboard (`web/admin/dist`) straight out of the
//! compiled binary, the same trick PocketBase uses to ship its Svelte admin
//! UI as part of a single Go executable. Run `bun run admin:build` (repo
//! root) before `cargo build` to embed the real dashboard instead of the
//! placeholder checked into `web/admin/dist`.

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::RustEmbed;

use crate::state::AppState;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/admin/dist"]
struct DashboardAssets;

pub fn router() -> Router<AppState> {
    Router::new().fallback(get(serve))
}

/// Serves `path` if it exists in the embedded bundle; otherwise falls back
/// to `index.html` so client-side routes (`/collections/posts`, ...)
/// resolve to the SPA shell instead of a 404.
async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

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
                "dashboard not built — run `bun run admin:build`",
            )
                .into_response(),
        },
    }
}
