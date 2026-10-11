//! Uploadable brand logo.
//!
//! * `POST /api/settings/logo` (superuser, multipart field `file`) stores the
//!   image under a reserved object key and points `settings.meta.logoUrl` at
//!   the public `GET /api/branding/logo` URL.
//! * `GET /api/branding/logo` (public) streams it. System emails embed the
//!   URL, so mail clients must be able to fetch it without credentials; the
//!   `?v=<hash>` query is a cache buster that lets the response be immutable.
//! * `DELETE /api/settings/logo` (superuser) removes the object and clears
//!   `logoUrl` only if it still points at the uploaded logo.
//!
//! Only raster formats are accepted (png, jpeg, webp, gif), identified by
//! magic bytes rather than the client-declared type. SVG is rejected: it can
//! carry script (the file is served from the dashboard's origin) and most
//! mail clients do not render it.

use axum::body::Body;
use axum::extract::{Multipart, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiResult};

/// Reserved storage key. Content type is re-derived from the bytes on read,
/// so no extension or metadata is needed (works on every storage driver).
const LOGO_KEY: &str = "_branding/logo";
/// Path of the public URL stored in `meta.logoUrl` (without the query).
const LOGO_PATH: &str = "/api/branding/logo";
const MAX_LOGO_BYTES: usize = 1024 * 1024;

pub fn router() -> Router<App> {
    Router::new()
        .route("/settings/logo", axum::routing::post(upload).delete(remove))
        .route("/branding/logo", get(serve))
}

/// Magic-byte sniff; `None` for anything that is not png/jpeg/webp/gif.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn short_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn upload(
    State(app): State<App>,
    _su: RequireSuperuser,
    mut multipart: Multipart,
) -> ApiResult<Json<Value>> {
    let invalid =
        || ApiError::bad_request("Failed to load the submitted data due to invalid formatting.");
    let mut data: Option<Vec<u8>> = None;
    while let Some(mut field) = multipart.next_field().await.map_err(|_| invalid())? {
        if field.name() != Some("file") {
            continue;
        }
        let mut buf = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(|_| invalid())? {
            if buf.len() + chunk.len() > MAX_LOGO_BYTES {
                return Err(ApiError::bad_request("The logo must be 1 MiB or smaller."));
            }
            buf.extend_from_slice(&chunk);
        }
        data = Some(buf);
    }
    let Some(data) = data.filter(|d| !d.is_empty()) else {
        return Err(ApiError::bad_request(
            "Missing logo file: send it as the multipart field \"file\".",
        ));
    };
    if sniff(&data).is_none() {
        return Err(ApiError::bad_request(
            "The logo must be a PNG, JPEG, WebP or GIF image (SVG is not supported).",
        ));
    }

    let version = short_hash(&data);
    app.storage().put(LOGO_KEY, data.into()).await?;

    let mut settings = (*app.settings()).clone();
    let base = settings.meta.app_url.trim_end_matches('/').to_string();
    let url = format!("{base}{LOGO_PATH}?v={version}");
    settings.meta.logo_url = url.clone();
    app.set_settings(settings)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({ "logoUrl": url })))
}

async fn remove(State(app): State<App>, _su: RequireSuperuser) -> ApiResult<StatusCode> {
    app.storage().delete(LOGO_KEY).await?;

    let current = app.settings();
    let is_uploaded = current
        .meta
        .logo_url
        .split('?')
        .next()
        .is_some_and(|p| p.ends_with(LOGO_PATH));
    if is_uploaded {
        let mut settings = (*current).clone();
        settings.meta.logo_url = String::new();
        app.set_settings(settings)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn serve(State(app): State<App>) -> ApiResult<Response> {
    let bytes = app
        .storage()
        .get(LOGO_KEY)
        .await
        .map_err(|_| ApiError::not_found("No logo has been uploaded."))?;
    let mime = sniff(&bytes).unwrap_or("application/octet-stream");

    let mut response = Body::from(bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("inline"),
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_raster_formats_only() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff(b"GIF89a.."), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), None);
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVE"), None);
    }
}
