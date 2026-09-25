//! End-to-end coverage for `_emailTriggers`: a record write on a
//! configured collection/event fires the matching template through the
//! ordinary send pipeline, gated by `condition`/`enabled`, and a bad
//! `toField` lands in `_mailLog` as a failure without ever failing the
//! triggering request.
//!
//! Dispatch is fire-and-forget (`crate::email_triggers::
//! dispatch_after_success` spawns it), so every assertion here polls
//! briefly rather than asserting immediately after the triggering write.

use std::time::Duration;

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
    superuser_token: String,
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

    fn router(&self) -> axum::Router {
        cratebase_server::router(self.app.clone())
    }

    async fn send(&self, request: Request<Body>) -> Response {
        self.router().oneshot(request).await.expect("response")
    }

    async fn admin(&self, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", &self.superuser_token)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        split(self.send(request).await).await
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", &self.superuser_token)
            .body(Body::empty())
            .unwrap();
        split(self.send(request).await).await
    }

    async fn create_collection(&self, body: Value) -> String {
        let (status, value) = self.admin("POST", "/api/collections", body).await;
        assert_eq!(status, StatusCode::OK, "{value:?}");
        value["id"].as_str().expect("an id").to_string()
    }

    async fn create_trigger(&self, body: Value) -> String {
        let (status, value) = self
            .admin("POST", "/api/collections/_emailTriggers/records", body)
            .await;
        assert_eq!(status, StatusCode::OK, "{value:?}");
        value["id"].as_str().expect("an id").to_string()
    }

    /// Polls `_mailLog` (filtered by `template`) until at least one row
    /// exists, or gives up after ~1s and returns whatever it has (empty,
    /// if none ever showed up).
    /// Waits for a row and for its `status` to leave the initial
    /// `"queued"` state (written before delivery is even attempted —
    /// see `crate::mails`'s module doc) so a caller never observes it
    /// mid-flight.
    async fn poll_mail_log(&self, template: &str) -> Vec<Value> {
        for _ in 0..50 {
            let (status, body) = self
                .get(&format!(
                    "/api/collections/_mailLog/records?filter=template='{template}'&sort=-created"
                ))
                .await;
            assert_eq!(status, StatusCode::OK, "{body:?}");
            let items = body["items"].as_array().cloned().unwrap_or_default();
            if !items.is_empty() && items[0]["status"] != "queued" {
                return items;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        vec![]
    }

    /// Waits briefly and asserts no `_mailLog` row for `template` ever
    /// showed up — used for the "no mail" cases (condition false,
    /// disabled), where there's nothing to poll *for*.
    async fn assert_no_mail_log(&self, template: &str) {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let (status, body) = self
            .get(&format!(
                "/api/collections/_mailLog/records?filter=template='{template}'"
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{body:?}");
        assert_eq!(
            body["items"].as_array().unwrap().len(),
            0,
            "expected no _mailLog row for {template}: {body:?}"
        );
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

fn orders_collection(name: &str) -> Value {
    json!({
        "name": name,
        "type": "base",
        "listRule": "",
        "viewRule": "",
        "createRule": "",
        "updateRule": "",
        "deleteRule": "",
        "fields": [
            {"name": "email", "type": "text"},
            {"name": "status", "type": "text"},
        ],
    })
}

#[tokio::test]
async fn a_create_trigger_fires_with_the_written_record_as_data() {
    let h = Harness::new().await;
    h.create_collection(orders_collection("orders")).await;
    h.create_trigger(json!({
        "collection": "orders",
        "event": "create",
        "template": "welcome",
        "toField": "email",
        "enabled": true,
    }))
    .await;

    let (status, order) = h
        .admin(
            "POST",
            "/api/collections/orders/records",
            json!({ "email": "buyer@example.com", "status": "paid" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{order:?}");

    let rows = h.poll_mail_log("welcome").await;
    assert_eq!(rows.len(), 1, "expected exactly one mail log row");
    assert_eq!(rows[0]["status"], "sent");
    assert_eq!(rows[0]["to"][0]["address"], "buyer@example.com");

    let mailbox = h.app.mailer().dev_mailbox().expect("dev mailbox");
    assert_eq!(mailbox.len(), 1);
}

#[tokio::test]
async fn a_false_condition_sends_no_mail() {
    let h = Harness::new().await;
    h.create_collection(orders_collection("orders")).await;
    h.create_trigger(json!({
        "collection": "orders",
        "event": "create",
        "template": "welcome",
        "toField": "email",
        "condition": "status = \"paid\"",
        "enabled": true,
    }))
    .await;

    let (status, order) = h
        .admin(
            "POST",
            "/api/collections/orders/records",
            json!({ "email": "buyer@example.com", "status": "pending" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{order:?}");

    h.assert_no_mail_log("welcome").await;
}

#[tokio::test]
async fn a_true_condition_still_sends() {
    let h = Harness::new().await;
    h.create_collection(orders_collection("orders")).await;
    h.create_trigger(json!({
        "collection": "orders",
        "event": "create",
        "template": "welcome",
        "toField": "email",
        "condition": "status = \"paid\"",
        "enabled": true,
    }))
    .await;

    let (status, order) = h
        .admin(
            "POST",
            "/api/collections/orders/records",
            json!({ "email": "buyer@example.com", "status": "paid" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{order:?}");

    let rows = h.poll_mail_log("welcome").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], "sent");
}

#[tokio::test]
async fn a_disabled_trigger_sends_no_mail() {
    let h = Harness::new().await;
    h.create_collection(orders_collection("orders")).await;
    h.create_trigger(json!({
        "collection": "orders",
        "event": "create",
        "template": "welcome",
        "toField": "email",
        "enabled": false,
    }))
    .await;

    let (status, order) = h
        .admin(
            "POST",
            "/api/collections/orders/records",
            json!({ "email": "buyer@example.com", "status": "paid" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{order:?}");

    h.assert_no_mail_log("welcome").await;
}

#[tokio::test]
async fn a_bad_to_field_is_logged_as_a_failure_but_the_request_still_succeeds() {
    let h = Harness::new().await;
    h.create_collection(orders_collection("orders")).await;
    h.create_trigger(json!({
        "collection": "orders",
        "event": "create",
        "template": "welcome",
        // Neither a field on `orders` nor a literal address.
        "toField": "notAField",
        "enabled": true,
    }))
    .await;

    let (status, order) = h
        .admin(
            "POST",
            "/api/collections/orders/records",
            json!({ "email": "buyer@example.com", "status": "paid" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{order:?}");

    let rows = h.poll_mail_log("welcome").await;
    assert_eq!(rows.len(), 1, "expected a failure row: {rows:?}");
    assert_eq!(rows[0]["status"], "failed");
    assert!(rows[0]["error"].as_str().unwrap().contains("toField"));

    let mailbox = h.app.mailer().dev_mailbox().expect("dev mailbox");
    assert!(mailbox.is_empty(), "nothing should actually have sent");
}

#[tokio::test]
async fn an_update_trigger_fires_on_update_not_create() {
    let h = Harness::new().await;
    h.create_collection(orders_collection("orders")).await;
    h.create_trigger(json!({
        "collection": "orders",
        "event": "update",
        "template": "welcome",
        "toField": "email",
        "enabled": true,
    }))
    .await;

    let (status, order) = h
        .admin(
            "POST",
            "/api/collections/orders/records",
            json!({ "email": "buyer@example.com", "status": "pending" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{order:?}");
    let id = order["id"].as_str().unwrap();

    // Creating the record must not have fired the update-only trigger.
    h.assert_no_mail_log("welcome").await;

    let (status, updated) = h
        .admin(
            "PATCH",
            &format!("/api/collections/orders/records/{id}"),
            json!({ "status": "paid" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated:?}");

    let rows = h.poll_mail_log("welcome").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], "sent");
}
