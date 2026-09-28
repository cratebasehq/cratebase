//! End-to-end tests for `POST /api/files/presign` and its local-storage
//! upload fallback (`PUT /api/files/presign-upload/{token}`), driven
//! through `tower::ServiceExt::oneshot` against an in-memory database and
//! local disk storage — no S3 needed, since the local driver's presign
//! response is a same-origin URL this server handles itself, and the
//! resolution/validation logic (rules, size/mime, reuse, expiry) is
//! identical either way.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_db::{Executor, Sql};
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

    async fn raw(&self, request: Request<Body>) -> Response {
        cratebase_server::router(self.app.clone())
            .oneshot(request)
            .await
            .expect("response")
    }

    async fn admin(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.as_user(method, uri, body, Some(&self.token)).await
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

    async fn collection(&self, body: Value) -> String {
        let (status, value) = self.admin("POST", "/api/collections", Some(body)).await;
        assert_eq!(status, 200, "{value}");
        value["id"].as_str().expect("an id").to_string()
    }

    /// Push `_pendingUploads.expiresAt` for `token`'s row into the past,
    /// so an expiry check can be tested without sleeping.
    async fn expire_token(&self, token: &str) {
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(&mut hasher, token.as_bytes());
        let digest: [u8; 32] = sha2::Digest::finalize(hasher).into();
        let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        self.app
            .db()
            .execute(
                r#"UPDATE "_pendingUploads" SET "expiresAt" = '2000-01-01 00:00:00.000Z' WHERE "tokenHash" = $1"#,
                &[Sql::Text(hash)],
            )
            .await
            .expect("expire token");
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

fn docs_collection(name: &str) -> Value {
    json!({
        "name": name,
        "type": "base",
        "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": "",
        "fields": [
            {"name": "title", "type": "text"},
            {"name": "doc", "type": "file", "maxSelect": 1, "maxSize": 1000, "mimeTypes": ["text/plain"]},
        ],
    })
}

/// The full happy path: presign → upload the bytes to the local-storage
/// fallback URL → create the record with the token as the field's value
/// → the record ends up with the real stored filename, and the bytes are
/// readable back through the ordinary files route.
#[tokio::test]
async fn presign_upload_and_claim_round_trips() {
    let harness = Harness::new().await;
    let collection_id = harness.collection(docs_collection("docs")).await;

    let (status, presigned) = harness
        .admin(
            "POST",
            "/api/files/presign",
            Some(json!({
                "collection": "docs",
                "field": "doc",
                "filename": "notes.txt",
                "contentType": "text/plain",
                "size": 11,
            })),
        )
        .await;
    assert_eq!(status, 200, "{presigned}");
    let token = presigned["token"].as_str().unwrap().to_string();
    let record_id = presigned["recordId"].as_str().unwrap().to_string();
    let upload_url = presigned["uploadUrl"].as_str().unwrap().to_string();
    assert!(token.starts_with("CBUP_"), "{token}");
    assert!(
        upload_url.starts_with("/api/files/presign-upload/"),
        "{upload_url}"
    );

    // Upload straight to the local-storage fallback URL, no auth header:
    // the token itself is the bearer, same trust model as an S3 signature.
    let response = harness
        .raw(
            Request::put(&upload_url)
                .body(Body::from("hello world"))
                .unwrap(),
        )
        .await;
    assert!(response.status().is_success(), "{}", response.status());

    // Claim it by sending the token as the field's value in the create
    // call, with the exact `id` the presign response minted.
    let (status, created) = harness
        .admin(
            "POST",
            "/api/collections/docs/records",
            Some(json!({"id": record_id, "title": "From presign", "doc": token})),
        )
        .await;
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["id"], record_id);
    let stored = created["doc"].as_str().unwrap().to_string();
    assert!(stored.starts_with("notes_"), "{stored}");

    let url = format!("/api/files/{collection_id}/{record_id}/{stored}");
    let response = harness
        .raw(Request::get(&url).body(Body::empty()).unwrap())
        .await;
    assert_eq!(response.status(), 200);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], b"hello world");
}

/// `maxSize`/`mimeTypes` are enforced at presign time, before the client
/// wastes an upload on a request that would be rejected anyway.
#[tokio::test]
async fn presign_enforces_max_size_and_mime_types() {
    let harness = Harness::new().await;
    harness.collection(docs_collection("docs")).await;

    let (status, body) = harness
        .admin(
            "POST",
            "/api/files/presign",
            Some(json!({
                "collection": "docs", "field": "doc", "filename": "big.txt",
                "contentType": "text/plain", "size": 5000,
            })),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = harness
        .admin(
            "POST",
            "/api/files/presign",
            Some(json!({
                "collection": "docs", "field": "doc", "filename": "x.png",
                "contentType": "image/png", "size": 10,
            })),
        )
        .await;
    assert_eq!(status, 400, "{body}");
}

/// `POST /api/files/presign` for an *update* respects the target
/// record's `updateRule` — an anonymous request against an
/// owner-restricted collection is refused before any token is minted.
#[tokio::test]
async fn presign_update_is_gated_by_the_update_rule() {
    let harness = Harness::new().await;
    harness
        .collection(json!({
            "name": "owned_docs",
            "type": "base",
            "listRule": "", "viewRule": "",
            "createRule": "@request.auth.id != ''",
            "updateRule": "@request.auth.id != '' && owner = @request.auth.id",
            "deleteRule": null,
            "fields": [
                {"name": "owner", "type": "text"},
                {"name": "doc", "type": "file", "maxSelect": 1},
            ],
        }))
        .await;
    let (status, created) = harness
        .admin(
            "POST",
            "/api/collections/owned_docs/records",
            Some(json!({"owner": "someone-else"})),
        )
        .await;
    assert_eq!(status, 200, "{created}");
    let id = created["id"].as_str().unwrap();

    // Anonymous: the update rule requires auth at all, so this is refused.
    let (status, body) = harness
        .as_user(
            "POST",
            "/api/files/presign",
            Some(json!({
                "collection": "owned_docs", "field": "doc", "filename": "x.txt",
                "contentType": "text/plain", "size": 1, "recordId": id,
            })),
            None,
        )
        .await;
    assert_eq!(status, 404, "{body}");
}

/// A presign token can only be claimed once: a second create/update that
/// reuses it is rejected outright.
#[tokio::test]
async fn presign_token_reuse_is_rejected() {
    let harness = Harness::new().await;
    harness.collection(docs_collection("docs")).await;

    let (_, presigned) = harness
        .admin(
            "POST",
            "/api/files/presign",
            Some(json!({
                "collection": "docs", "field": "doc", "filename": "notes.txt",
                "contentType": "text/plain", "size": 11,
            })),
        )
        .await;
    let token = presigned["token"].as_str().unwrap().to_string();
    let record_id = presigned["recordId"].as_str().unwrap().to_string();
    let upload_url = presigned["uploadUrl"].as_str().unwrap().to_string();
    harness
        .raw(
            Request::put(&upload_url)
                .body(Body::from("hello world"))
                .unwrap(),
        )
        .await;

    let (status, first) = harness
        .admin(
            "POST",
            "/api/collections/docs/records",
            Some(json!({"id": record_id.clone(), "title": "one", "doc": token.clone()})),
        )
        .await;
    assert_eq!(status, 200, "{first}");

    // A second record trying to claim the same, already-consumed token.
    let (status, second) = harness
        .admin(
            "POST",
            "/api/collections/docs/records",
            Some(json!({"title": "two", "doc": token})),
        )
        .await;
    assert_eq!(status, 400, "{second}");
}

/// An expired ticket is refused both at upload time and at claim time,
/// with no way to resurrect it.
#[tokio::test]
async fn presign_expiry_is_enforced() {
    let harness = Harness::new().await;
    harness.collection(docs_collection("docs")).await;

    let (_, presigned) = harness
        .admin(
            "POST",
            "/api/files/presign",
            Some(json!({
                "collection": "docs", "field": "doc", "filename": "notes.txt",
                "contentType": "text/plain", "size": 11,
            })),
        )
        .await;
    let token = presigned["token"].as_str().unwrap().to_string();
    let record_id = presigned["recordId"].as_str().unwrap().to_string();
    let upload_url = presigned["uploadUrl"].as_str().unwrap().to_string();

    harness.expire_token(&token).await;

    let response = harness
        .raw(
            Request::put(&upload_url)
                .body(Body::from("hello world"))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let (status, body) = harness
        .admin(
            "POST",
            "/api/collections/docs/records",
            Some(json!({"id": record_id, "title": "x", "doc": token})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
}

/// The hourly sweep (`crate::presign::sweep_expired`, cron id
/// `__cbPendingUploadsSweep__`) removes an expired, still-`pending`
/// ticket and its orphaned object, but never touches a `consumed` one
/// (that key is a record's live file) even if its `expiresAt` has long
/// since passed.
#[tokio::test]
async fn sweep_removes_expired_pending_tickets_but_not_consumed_ones() {
    let harness = Harness::new().await;
    harness.collection(docs_collection("docs")).await;

    // Ticket 1: presigned, uploaded, never claimed, then expired —
    // should be swept, object and all.
    let (_, presigned1) = harness
        .admin(
            "POST",
            "/api/files/presign",
            Some(json!({
                "collection": "docs", "field": "doc", "filename": "orphan.txt",
                "contentType": "text/plain", "size": 5,
            })),
        )
        .await;
    let token1 = presigned1["token"].as_str().unwrap().to_string();
    let upload_url1 = presigned1["uploadUrl"].as_str().unwrap().to_string();
    harness
        .raw(
            Request::put(&upload_url1)
                .body(Body::from("hello"))
                .unwrap(),
        )
        .await;
    harness.expire_token(&token1).await;

    // Ticket 2: presigned, uploaded, claimed into a real record — its
    // row is `consumed`; even backdating `expiresAt` must not sweep it.
    let (_, presigned2) = harness
        .admin(
            "POST",
            "/api/files/presign",
            Some(json!({
                "collection": "docs", "field": "doc", "filename": "keep.txt",
                "contentType": "text/plain", "size": 11,
            })),
        )
        .await;
    let token2 = presigned2["token"].as_str().unwrap().to_string();
    let record_id2 = presigned2["recordId"].as_str().unwrap().to_string();
    let upload_url2 = presigned2["uploadUrl"].as_str().unwrap().to_string();
    harness
        .raw(
            Request::put(&upload_url2)
                .body(Body::from("hello world"))
                .unwrap(),
        )
        .await;
    let (status, created) = harness
        .admin(
            "POST",
            "/api/collections/docs/records",
            Some(json!({"id": record_id2, "title": "keep", "doc": token2.clone()})),
        )
        .await;
    assert_eq!(status, 200, "{created}");
    harness.expire_token(&token2).await;

    cratebase_server::presign::sweep_expired(&harness.app).await;

    let remaining: i64 = harness
        .app
        .db()
        .query_scalar(r#"SELECT COUNT(*) FROM "_pendingUploads""#, &[])
        .await
        .unwrap()
        .and_then(|v| v.as_i64())
        .unwrap();
    assert_eq!(remaining, 1, "only the consumed ticket should remain");

    // The claimed record's file is still there — the sweep must not
    // have deleted its object.
    let stored = created["doc"].as_str().unwrap();
    let url = format!(
        "/api/files/{}/{}/{}",
        created["collectionId"].as_str().unwrap(),
        created["id"].as_str().unwrap(),
        stored
    );
    let response = harness
        .raw(Request::get(&url).body(Body::empty()).unwrap())
        .await;
    assert_eq!(response.status(), 200);
}

/// `settings.storage.userQuotaBytes` gates a presign against the sum of
/// `ownerField`-matching, already-consumed presigned uploads.
#[tokio::test]
async fn per_user_quota_gates_further_presigns_once_exceeded() {
    let harness = Harness::new().await;
    harness
        .collection(json!({
            "name": "photos",
            "type": "base",
            "listRule": "", "viewRule": "", "createRule": "", "updateRule": "", "deleteRule": "",
            "ownerField": "owner",
            "fields": [
                {"name": "owner", "type": "relation", "collectionId": "pbc_3142635823", "maxSelect": 1},
                {"name": "photo", "type": "file", "maxSelect": 1},
            ],
        }))
        .await;

    // 15 bytes total quota.
    let mut settings = harness.app.settings().as_ref().clone();
    settings.storage.user_quota_bytes = 15;
    harness.app.set_settings(settings).await.unwrap();

    let admin_id = harness
        .admin("GET", "/api/collections/_superusers/records", None)
        .await
        .1["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // First upload: 10 bytes, well under quota.
    let (status, presigned) = harness
        .admin(
            "POST",
            "/api/files/presign",
            Some(json!({
                "collection": "photos", "field": "photo", "filename": "a.bin",
                "contentType": "application/octet-stream", "size": 10,
            })),
        )
        .await;
    assert_eq!(status, 200, "{presigned}");
    let token = presigned["token"].as_str().unwrap().to_string();
    let record_id = presigned["recordId"].as_str().unwrap().to_string();
    let upload_url = presigned["uploadUrl"].as_str().unwrap().to_string();
    harness
        .raw(
            Request::put(&upload_url)
                .body(Body::from(vec![0u8; 10]))
                .unwrap(),
        )
        .await;
    let (status, created) = harness
        .admin(
            "POST",
            "/api/collections/photos/records",
            Some(json!({"id": record_id, "owner": admin_id, "photo": token})),
        )
        .await;
    assert_eq!(status, 200, "{created}");

    // Second upload: only 5 bytes of quota left, this one asks for 10 —
    // over budget, refused before any ticket is minted.
    let (status, body) = harness
        .admin(
            "POST",
            "/api/files/presign",
            Some(json!({
                "collection": "photos", "field": "photo", "filename": "b.bin",
                "contentType": "application/octet-stream", "size": 10,
            })),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    // A 5-byte upload still fits exactly.
    let (status, body) = harness
        .admin(
            "POST",
            "/api/files/presign",
            Some(json!({
                "collection": "photos", "field": "photo", "filename": "c.bin",
                "contentType": "application/octet-stream", "size": 5,
            })),
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

/// An unknown token in a file field is a normal validation error, not a
/// panic or a 500 — it just isn't a token this server ever issued.
#[tokio::test]
async fn unknown_token_is_a_bad_request_not_a_crash() {
    let harness = Harness::new().await;
    harness.collection(docs_collection("docs")).await;
    let (status, body) = harness
        .admin(
            "POST",
            "/api/collections/docs/records",
            Some(json!({"title": "x", "doc": "CBUP_doesnotexist"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
}
