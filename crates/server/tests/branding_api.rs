//! End-to-end tests for the uploadable brand logo:
//! `POST/DELETE /api/settings/logo` and the public `GET /api/branding/logo`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use serde_json::{json, Value};
use tower::ServiceExt;

const BOUNDARY: &str = "XBOUNDARYX";

/// A real 1x1 PNG.
fn tiny_png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")
        .unwrap()
}

struct Resp {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    bytes: Vec<u8>,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or(Value::Null)
    }
}

struct Harness {
    app: App,
    superuser_token: String,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path().join("cb_data")));
        app.bootstrap().await.expect("bootstrap");
        let id = app
            .create_superuser("admin@example.com", "hunter2hunter2")
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

    async fn send(&self, req: Request<Body>) -> Resp {
        let resp = cratebase_server::router(self.app.clone())
            .oneshot(req)
            .await
            .expect("response");
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        Resp {
            status,
            headers,
            bytes,
        }
    }

    async fn json_req(&self, method: &str, uri: &str, token: Option<&str>, body: Value) -> Resp {
        let mut b = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(t) = token {
            b = b.header("authorization", t);
        }
        let body = if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
        self.send(b.body(body).unwrap()).await
    }

    async fn upload(&self, token: Option<&str>, filename: &str, ctype: &str, data: &[u8]) -> Resp {
        let mut body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {ctype}\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        let mut b = Request::builder()
            .method("POST")
            .uri("/api/settings/logo")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={BOUNDARY}"),
            );
        if let Some(t) = token {
            b = b.header("authorization", t);
        }
        self.send(b.body(Body::from(body)).unwrap()).await
    }

    async fn logo_url_setting(&self) -> String {
        let r = self
            .json_req(
                "GET",
                "/api/settings",
                Some(&self.superuser_token),
                Value::Null,
            )
            .await;
        r.json()["meta"]["logoUrl"].as_str().unwrap().to_string()
    }
}

#[tokio::test]
async fn upload_sets_logo_url_and_serves_publicly() {
    let h = Harness::new().await;
    let png = tiny_png();
    let r = h
        .upload(Some(&h.superuser_token), "logo.png", "image/png", &png)
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.json());
    let url = r.json()["logoUrl"].as_str().unwrap().to_string();
    assert!(
        url.starts_with("http://localhost:8090/api/branding/logo?v="),
        "{url}"
    );
    assert_eq!(h.logo_url_setting().await, url);

    // Public: no Authorization header.
    let get = h
        .send(
            Request::builder()
                .uri("/api/branding/logo")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(get.status, StatusCode::OK);
    assert_eq!(get.bytes, png);
    let hv = |n: &str| get.headers.get(n).unwrap().to_str().unwrap().to_string();
    assert_eq!(hv("content-type"), "image/png");
    assert_eq!(hv("x-content-type-options"), "nosniff");
    assert_eq!(hv("cache-control"), "public, max-age=31536000, immutable");
    assert_eq!(hv("content-disposition"), "inline");
}

#[tokio::test]
async fn rejects_svg_non_image_and_oversize() {
    let h = Harness::new().await;
    let t = Some(h.superuser_token.as_str());

    // Declared type is image/png but the bytes are SVG: sniffing wins.
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>";
    let r = h.upload(t, "logo.png", "image/png", svg).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.json()["message"].as_str().unwrap().contains("SVG"));

    let r = h.upload(t, "a.txt", "text/plain", b"hello").await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    let mut big = tiny_png();
    big.resize(1024 * 1024 + 1, 0);
    let r = h.upload(t, "big.png", "image/png", &big).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.json()["message"].as_str().unwrap().contains("1 MiB"));

    // Exactly 1 MiB is fine.
    big.truncate(1024 * 1024);
    let r = h.upload(t, "big.png", "image/png", &big).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.json());

    // Nothing stored by the failures: fresh instance has no logo.
    let h2 = Harness::new().await;
    let _ = h2.upload(t, "a.txt", "text/plain", b"hello").await;
    let get = h2
        .send(
            Request::builder()
                .uri("/api/branding/logo")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(get.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn upload_and_delete_require_superuser() {
    let h = Harness::new().await;
    let collection = h.app.db().collections.get("users").expect("users");
    let mut record = cratebase_core::Record::new(collection);
    record.set("email", Value::String("u@example.com".into()));
    record.set("password", Value::String("supersecret123".into()));
    record.set("verified", Value::Bool(true));
    cratebase_db::records::create(h.app.db(), &h.app.db().collections, &mut record)
        .await
        .expect("create user");
    let user = h
        .app
        .mint_token("users", record.id(), cratebase_auth::TokenType::Auth, 3600)
        .await
        .expect("token");

    let r = h
        .upload(Some(&user), "logo.png", "image/png", &tiny_png())
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = h
        .json_req("DELETE", "/api/settings/logo", Some(&user), Value::Null)
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = h.upload(None, "logo.png", "image/png", &tiny_png()).await;
    assert!(
        r.status == StatusCode::UNAUTHORIZED || r.status == StatusCode::FORBIDDEN,
        "{}",
        r.status
    );
}

#[tokio::test]
async fn delete_clears_uploaded_logo() {
    let h = Harness::new().await;
    let t = h.superuser_token.clone();
    let r = h
        .upload(Some(&t), "logo.png", "image/png", &tiny_png())
        .await;
    assert_eq!(r.status, StatusCode::OK);

    let r = h
        .json_req("DELETE", "/api/settings/logo", Some(&t), Value::Null)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(h.logo_url_setting().await, "");
    let get = h
        .send(
            Request::builder()
                .uri("/api/branding/logo")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(get.status, StatusCode::NOT_FOUND);
    assert!(get.json()["message"].is_string());
}

#[tokio::test]
async fn delete_keeps_custom_external_logo_url() {
    let h = Harness::new().await;
    let t = h.superuser_token.clone();
    let r = h
        .json_req(
            "PATCH",
            "/api/settings",
            Some(&t),
            json!({"meta": {"logoUrl": "https://cdn.example.com/logo.png"}}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.json());

    let r = h
        .json_req("DELETE", "/api/settings/logo", Some(&t), Value::Null)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(
        h.logo_url_setting().await,
        "https://cdn.example.com/logo.png"
    );
}
