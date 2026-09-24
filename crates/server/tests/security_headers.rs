//! Stored-XSS hardening: response security headers, the dashboard's
//! clickjacking gate, and upload MIME sniffing (magic bytes, not the
//! client-declared `Content-Type`).
//!
//! An uploaded `.html`/`.svg` is user-controlled content served from the
//! same origin as the admin dashboard (`/_/`); without these headers a
//! stored file *is* script execution on that origin. See
//! `crates/server/src/routes/files.rs` and
//! `crates/server/src/routes/common.rs::mime_of`.

use axum::body::Body;
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

    async fn raw(&self, request: Request<Body>) -> Response {
        cratebase_server::router(self.app.clone())
            .oneshot(request)
            .await
            .expect("response")
    }

    async fn admin(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", &self.token);
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => {
                builder = builder.header("content-type", "application/json");
                builder.body(Body::empty()).unwrap()
            }
        };
        let response = self.raw(request).await;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, value)
    }

    /// Create a collection through the HTTP API and return its id.
    async fn collection(&self, body: Value) -> String {
        let (status, value) = self.admin("POST", "/api/collections", Some(body)).await;
        assert_eq!(status, 200, "{value}");
        value["id"].as_str().expect("an id").to_string()
    }
}

/// A multipart file part: field name, filename, declared (possibly
/// lying) `Content-Type`, and the actual bytes — the whole point of these
/// tests is that the declared type must not be trusted.
type FilePart<'a> = (&'a str, &'a str, &'a str, &'a [u8]);

/// One part of a hand-built `multipart/form-data` body.
fn multipart(parts: &[FilePart<'_>]) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "----cratebasesecurityheaders";
    let mut body = Vec::new();
    for (name, filename, content_type, content) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

/// A `docs` collection with a single file field, wide open on every rule.
async fn file_collection(harness: &Harness, mime_types: &[&str]) -> String {
    harness
        .collection(json!({
            "name": "docs",
            "type": "base",
            "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": "",
            "fields": [
                {"name": "title", "type": "text"},
                {"name": "asset", "type": "file", "maxSelect": 1, "mimeTypes": mime_types},
            ],
        }))
        .await
}

async fn upload(
    harness: &Harness,
    collection_id: &str,
    filename: &str,
    declared_type: &str,
    content: &[u8],
) -> (StatusCode, Value) {
    let (content_type, body) = multipart(&[("asset", filename, declared_type, content)]);
    let response = harness
        .raw(
            Request::post(format!("/api/collections/{collection_id}/records"))
                .header("authorization", &harness.token)
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

#[tokio::test]
async fn dashboard_responses_get_frame_and_clickjacking_headers() {
    let harness = Harness::new().await;
    let response = harness
        .raw(Request::get("/_/").body(Body::empty()).unwrap())
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(
        headers["referrer-policy"],
        "strict-origin-when-cross-origin"
    );
    assert_eq!(headers["x-frame-options"], "SAMEORIGIN");
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("frame-ancestors 'self'"), "{csp}");
}

#[tokio::test]
async fn api_responses_get_nosniff_and_referrer_policy_but_no_frame_ancestors_gate() {
    let harness = Harness::new().await;
    let response = harness
        .raw(Request::get("/api/health").body(Body::empty()).unwrap())
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(
        headers["referrer-policy"],
        "strict-origin-when-cross-origin"
    );
    // The dashboard's clickjacking gate is not this endpoint's business —
    // a JSON API response embedded in a frame is not the vulnerability.
    assert!(!headers.contains_key("x-frame-options"));
}

#[tokio::test]
async fn an_upload_declaring_png_but_containing_html_is_rejected_by_a_png_only_field() {
    let harness = Harness::new().await;
    let collection_id = file_collection(&harness, &["image/png"]).await;

    let (status, body) = upload(
        &harness,
        &collection_id,
        "photo.png",
        "image/png",
        b"<!DOCTYPE html><html><body><script>alert(document.cookie)</script></body></html>",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["data"]["asset"]["code"], "validation_invalid_mime_type",
        "{body}"
    );
}

#[tokio::test]
async fn a_genuine_png_upload_still_passes_a_png_only_field() {
    let harness = Harness::new().await;
    let collection_id = file_collection(&harness, &["image/png"]).await;

    // A real (tiny, 1x1) PNG, magic bytes and all.
    let png: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    let (status, body) = upload(&harness, &collection_id, "photo.png", "image/png", png).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn file_downloads_get_a_sandbox_content_security_policy() {
    let harness = Harness::new().await;
    let collection_id = file_collection(&harness, &[]).await;

    let (status, created) = upload(
        &harness,
        &collection_id,
        "notes.txt",
        "text/plain",
        b"hello world",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().unwrap();
    let stored = created["asset"].as_str().unwrap();

    let response = harness
        .raw(
            Request::get(format!("/api/files/{collection_id}/{id}/{stored}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers["content-security-policy"],
        "default-src 'none'; media-src 'self'; style-src 'unsafe-inline'; sandbox"
    );
    assert_eq!(headers["x-content-type-options"], "nosniff");
    // A harmless text file is still served inline.
    assert_eq!(
        headers["content-disposition"],
        format!("inline; filename=\"{stored}\"")
    );
}

#[tokio::test]
async fn an_svg_upload_is_forced_to_download_with_a_sandbox_csp() {
    let harness = Harness::new().await;
    let collection_id = file_collection(&harness, &["image/svg+xml"]).await;

    // No `<?xml ...?>` prolog — a bare `<svg>` root has no reliable magic
    // bytes to sniff, same as PocketBase's own content sniffer, so this
    // still relies on the declared/extension fallback to be accepted.
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
    let (status, created) =
        upload(&harness, &collection_id, "icon.svg", "image/svg+xml", svg).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().unwrap();
    let stored = created["asset"].as_str().unwrap();

    let response = harness
        .raw(
            Request::get(format!("/api/files/{collection_id}/{id}/{stored}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers["content-security-policy"],
        "default-src 'none'; media-src 'self'; style-src 'unsafe-inline'; sandbox"
    );
    // Active content is never served inline, `?download=` or not.
    assert_eq!(
        headers["content-disposition"],
        format!("attachment; filename=\"{stored}\"")
    );
}
