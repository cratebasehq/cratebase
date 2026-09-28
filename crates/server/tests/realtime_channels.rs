//! End-to-end tests for realtime channels + presence
//! (`crates/server/src/realtime.rs`'s "Realtime channels + presence"
//! section): `_channels`-gated subscribe/publish authorization, channels
//! disabled by default (no matching `_channels` row), the 413 oversized-
//! publish guard, presence join/update/list/leave-on-disconnect, and a
//! single-node throughput sanity check.
//!
//! Every test here runs a real `axum::serve` listener on an ephemeral
//! port and drives it with a real `reqwest` client + a real SSE stream —
//! the same harness `postgres_multi_node.rs` uses for its cross-node
//! proof, minus the second node/Postgres requirement, since presence's
//! "leave on disconnect" needs a real TCP socket to actually close.

use std::time::Duration;

use bytes::Bytes;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use futures::{Stream, StreamExt};
use serde_json::{json, Value};

const SUPERUSER_EMAIL: &str = "admin@example.com";
const SUPERUSER_PASSWORD: &str = "hunter2hunter2";

async fn spawn_node(app: App) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let router = cratebase_server::router(app);
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await;
    });
    addr
}

/// Read one complete SSE frame off a chunked HTTP body — see
/// `postgres_multi_node.rs`'s identical helper for the full doc comment.
async fn next_sse_frame(
    stream: &mut (impl Stream<Item = reqwest::Result<Bytes>> + Unpin),
    buf: &mut String,
) -> (Option<String>, Value) {
    loop {
        if let Some(pos) = buf.find("\n\n") {
            let frame = buf[..pos].to_string();
            *buf = buf[pos + 2..].to_string();
            let mut event = None;
            let mut data = String::new();
            for line in frame.lines() {
                if let Some(v) = line.strip_prefix("event:") {
                    event = Some(v.trim().to_string());
                } else if let Some(v) = line.strip_prefix("data:") {
                    data.push_str(v.trim());
                }
            }
            let value = serde_json::from_str(&data).unwrap_or(Value::Null);
            return (event, value);
        }
        let chunk = stream
            .next()
            .await
            .expect("sse stream ended before a full frame arrived")
            .expect("sse stream error");
        buf.push_str(std::str::from_utf8(&chunk).expect("sse frame is valid utf-8"));
    }
}

/// Read frames until one carries a non-`None` `event` (skipping
/// keep-alive comments), bounded by `secs`.
async fn next_named_frame(
    stream: &mut (impl Stream<Item = reqwest::Result<Bytes>> + Unpin),
    buf: &mut String,
    secs: u64,
) -> (String, Value) {
    loop {
        let (event, payload) =
            tokio::time::timeout(Duration::from_secs(secs), next_sse_frame(stream, buf))
                .await
                .expect("named frame never arrived within the deadline");
        if let Some(event) = event {
            return (event, payload);
        }
    }
}

struct Node {
    app: App,
    addr: std::net::SocketAddr,
    client: reqwest::Client,
    superuser_token: String,
    _dir: tempfile::TempDir,
}

impl Node {
    async fn start() -> Node {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path().join("pb_data")));
        app.bootstrap().await.expect("bootstrap");
        let id = app
            .create_superuser(SUPERUSER_EMAIL, SUPERUSER_PASSWORD)
            .await
            .expect("superuser");
        let superuser_token = app
            .mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token");
        // The default `/api/` rate-limit rule (300 req/10s) exists for
        // production abuse protection, not to bound how fast a test may
        // drive the router — several tests here (500 SSE connects for
        // the throughput check, in particular) legitimately burst past
        // it. Disabled wholesale, same idiom `api.rs`'s own rate-limit
        // tests use in reverse (they build `Settings::default()` and
        // flip it *on* for a narrow, deterministic rule).
        let mut settings = cratebase_core::Settings::default();
        settings.rate_limits.enabled = false;
        app.set_settings(settings)
            .await
            .expect("disable rate limits");
        let addr = spawn_node(app.clone()).await;
        Node {
            app,
            addr,
            client: reqwest::Client::new(),
            superuser_token,
            _dir: dir,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    /// A regular (non-superuser) `users` record with a valid auth token.
    async fn create_user(&self, email: &str) -> (String, String) {
        let collection = self.app.db().collections.get("users").expect("users");
        let mut record = cratebase_core::Record::new(collection);
        record.set("email", Value::String(email.to_string()));
        record.set("password", Value::String("supersecret123".into()));
        record.set("verified", Value::Bool(true));
        cratebase_db::records::create(self.app.db(), &self.app.db().collections, &mut record)
            .await
            .expect("create user");
        let id = record.id().to_string();
        let token = self
            .app
            .mint_token("users", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token");
        (id, token)
    }

    /// Configure a `_channels` row through the real (superuser-gated)
    /// records API — exercising `normalize_json_rule_fields`'s
    /// double-JSON-encode round trip for `subscribeRule`/`publishRule`
    /// exactly as an admin/dashboard write would, rather than poking the
    /// raw column directly.
    async fn configure_channel(&self, name: &str, subscribe_rule: &str, publish_rule: &str) {
        let resp = self
            .client
            .post(self.url("/api/collections/_channels/records"))
            .header("authorization", &self.superuser_token)
            .json(&json!({
                "name": name,
                "subscribeRule": subscribe_rule,
                "publishRule": publish_rule,
            }))
            .send()
            .await
            .expect("create _channels row");
        assert_eq!(
            resp.status(),
            200,
            "{:?}",
            resp.text().await.unwrap_or_default()
        );
    }

    /// Open an SSE stream and subscribe it to the given topics, returning
    /// the client id and the still-open stream/buffer.
    async fn connect_and_subscribe(
        &self,
        token: Option<&str>,
        topics: &[&str],
    ) -> (
        String,
        impl Stream<Item = reqwest::Result<Bytes>> + Unpin,
        String,
    ) {
        let mut req = self.client.get(self.url("/api/realtime"));
        if let Some(t) = token {
            req = req.header("authorization", t);
        }
        let sse = req.send().await.expect("GET /api/realtime");
        assert!(sse.status().is_success(), "{}", sse.status());
        let mut body = sse.bytes_stream();
        let mut buf = String::new();
        let (event, hello) =
            tokio::time::timeout(Duration::from_secs(5), next_sse_frame(&mut body, &mut buf))
                .await
                .expect("PB_CONNECT hello timed out");
        assert_eq!(event.as_deref(), Some("PB_CONNECT"));
        let client_id = hello["clientId"].as_str().expect("clientId").to_string();

        let mut submit = self.client.post(self.url("/api/realtime"));
        if let Some(t) = token {
            submit = submit.header("authorization", t);
        }
        let submit = submit
            .json(&json!({ "clientId": client_id, "subscriptions": topics }))
            .send()
            .await
            .expect("POST /api/realtime");
        assert_eq!(submit.status(), 204, "{:?}", submit.text().await);

        (client_id, body, buf)
    }
}

#[tokio::test]
async fn a_channel_with_no_matching_config_row_is_disabled_by_default() {
    let node = Node::start().await;
    // No `_channels` row at all for "room1" — publish must be refused,
    // not silently accepted, and never delivered.
    let resp = node
        .client
        .post(node.url("/api/realtime/channels/room1/publish"))
        .json(&json!({ "event": "hello", "data": {} }))
        .send()
        .await
        .expect("publish");
    assert_eq!(resp.status(), 403, "{:?}", resp.text().await);

    let resp = node
        .client
        .get(node.url("/api/realtime/channels/room1/presence"))
        .send()
        .await
        .expect("presence list");
    assert_eq!(resp.status(), 403, "{:?}", resp.text().await);
}

#[tokio::test]
async fn public_channel_subscribe_and_publish_round_trips() {
    let node = Node::start().await;
    node.configure_channel("lobby", "", "").await;

    let (_client_id, mut body, mut buf) =
        node.connect_and_subscribe(None, &["channel:lobby"]).await;

    let resp = node
        .client
        .post(node.url("/api/realtime/channels/lobby/publish"))
        .json(&json!({ "event": "chat", "data": { "text": "hi" } }))
        .send()
        .await
        .expect("publish");
    assert_eq!(resp.status(), 204, "{:?}", resp.text().await);

    let (event, payload) = next_named_frame(&mut body, &mut buf, 5).await;
    assert_eq!(event, "channel:lobby");
    assert_eq!(payload["event"], "chat");
    assert_eq!(payload["data"]["text"], "hi");
}

#[tokio::test]
async fn subscribe_rule_is_enforced_per_subscriber_not_just_at_publish_time() {
    let node = Node::start().await;
    // Only an authenticated caller may subscribe/publish.
    node.configure_channel(
        "private",
        "@request.auth.id != ''",
        "@request.auth.id != ''",
    )
    .await;
    let (_uid, token) = node.create_user("member@example.com").await;

    // Anonymous subscribe: the topic is registered (no error), but the
    // subscribeRule check at delivery time must silently withhold events.
    let (_anon_id, mut anon_body, mut anon_buf) =
        node.connect_and_subscribe(None, &["channel:private"]).await;
    // Authenticated subscribe: must receive it.
    let (_auth_id, mut auth_body, mut auth_buf) = node
        .connect_and_subscribe(Some(&token), &["channel:private"])
        .await;

    // An anonymous publish must be refused outright (publishRule too).
    let resp = node
        .client
        .post(node.url("/api/realtime/channels/private/publish"))
        .json(&json!({ "event": "secret", "data": {} }))
        .send()
        .await
        .expect("anon publish");
    assert_eq!(resp.status(), 403, "{:?}", resp.text().await);

    let resp = node
        .client
        .post(node.url("/api/realtime/channels/private/publish"))
        .header("authorization", &token)
        .json(&json!({ "event": "secret", "data": { "ok": true } }))
        .send()
        .await
        .expect("authed publish");
    assert_eq!(resp.status(), 204, "{:?}", resp.text().await);

    let (event, payload) = next_named_frame(&mut auth_body, &mut auth_buf, 5).await;
    assert_eq!(event, "channel:private");
    assert_eq!(payload["data"]["ok"], true);

    // The anonymous stream must never receive it: race a short timeout
    // against a named frame arriving at all.
    let raced = tokio::time::timeout(
        Duration::from_millis(700),
        next_named_frame(&mut anon_body, &mut anon_buf, 5),
    )
    .await;
    assert!(
        raced.is_err(),
        "an unauthorized subscriber received a private channel event"
    );
}

#[tokio::test]
async fn oversized_publish_is_rejected_with_413() {
    let node = Node::start().await;
    node.configure_channel("lobby", "", "").await;
    let huge = "x".repeat(20_000);
    let resp = node
        .client
        .post(node.url("/api/realtime/channels/lobby/publish"))
        .json(&json!({ "event": "chat", "data": { "text": huge } }))
        .send()
        .await
        .expect("oversized publish");
    assert_eq!(resp.status(), 413, "{:?}", resp.text().await);
}

#[tokio::test]
async fn presence_join_update_and_leave_on_disconnect() {
    let node = Node::start().await;
    node.configure_channel("room", "", "").await;

    let (client_id, mut body, mut buf) = node.connect_and_subscribe(None, &["channel:room"]).await;

    // Join.
    let resp = node
        .client
        .post(node.url("/api/realtime/channels/room/presence"))
        .json(&json!({ "clientId": client_id, "state": { "name": "alice" } }))
        .send()
        .await
        .expect("presence track");
    assert_eq!(resp.status(), 200, "{:?}", resp.text().await);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["event"],
        "presence.join"
    );

    let (event, payload) = next_named_frame(&mut body, &mut buf, 5).await;
    assert_eq!(event, "channel:room");
    assert_eq!(payload["event"], "presence.join");
    assert_eq!(payload["data"]["clientId"], client_id);
    assert_eq!(payload["data"]["state"]["name"], "alice");

    let list = node
        .client
        .get(node.url("/api/realtime/channels/room/presence"))
        .send()
        .await
        .expect("presence list")
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(list["members"].as_array().unwrap().len(), 1);
    assert_eq!(list["members"][0]["clientId"], client_id);

    // Update (same client, new state) must be a distinct event kind.
    let resp = node
        .client
        .post(node.url("/api/realtime/channels/room/presence"))
        .json(&json!({ "clientId": client_id, "state": { "name": "alice", "typing": true } }))
        .send()
        .await
        .expect("presence update");
    assert_eq!(
        resp.json::<Value>().await.unwrap()["event"],
        "presence.update"
    );
    let (event, payload) = next_named_frame(&mut body, &mut buf, 5).await;
    assert_eq!(event, "channel:room");
    assert_eq!(payload["event"], "presence.update");
    assert_eq!(payload["data"]["state"]["typing"], true);

    // A second subscriber watches for the leave event once the first
    // client's SSE connection is dropped below.
    let (_watcher_id, mut watcher_body, mut watcher_buf) =
        node.connect_and_subscribe(None, &["channel:room"]).await;

    drop(body); // Close the presence-tracking client's SSE connection.

    let (event, payload) = next_named_frame(&mut watcher_body, &mut watcher_buf, 5).await;
    assert_eq!(event, "channel:room");
    assert_eq!(payload["event"], "presence.leave");
    assert_eq!(payload["data"]["clientId"], client_id);

    // Give the disconnect handler's spawned cleanup a moment, then
    // confirm the member list is empty.
    for _ in 0..20 {
        let list = node
            .client
            .get(node.url("/api/realtime/channels/room/presence"))
            .send()
            .await
            .expect("presence list")
            .json::<Value>()
            .await
            .unwrap();
        if list["members"].as_array().unwrap().is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("presence member was not removed after its SSE connection disconnected");
}

#[tokio::test]
async fn presence_requires_a_live_sse_connection() {
    let node = Node::start().await;
    node.configure_channel("room", "", "").await;
    let resp = node
        .client
        .post(node.url("/api/realtime/channels/room/presence"))
        .json(&json!({ "clientId": "not-a-real-client-id", "state": {} }))
        .send()
        .await
        .expect("presence track with a bogus client id");
    assert_eq!(resp.status(), 404, "{:?}", resp.text().await);
}

/// Throughput sanity check: N=500 subscribers on one node all receive one
/// publish within a bounded time. Reports the observed numbers rather
/// than asserting a tight bound, since CI hardware varies — the
/// assertion is a generous ceiling meant to catch a real regression
/// (an accidentally-serialized fan-out, say), not to pin exact latency.
#[tokio::test]
async fn throughput_sanity_check_500_subscribers_receive_one_publish() {
    const N: usize = 500;
    let node = Node::start().await;
    node.configure_channel("broadcast", "", "").await;

    let mut receivers = Vec::with_capacity(N);
    for _ in 0..N {
        let (_id, body, buf) = node
            .connect_and_subscribe(None, &["channel:broadcast"])
            .await;
        receivers.push((body, buf));
    }

    let started = std::time::Instant::now();
    let resp = node
        .client
        .post(node.url("/api/realtime/channels/broadcast/publish"))
        .json(&json!({ "event": "go", "data": { "n": N } }))
        .send()
        .await
        .expect("publish");
    assert_eq!(resp.status(), 204, "{:?}", resp.text().await);

    let waits = receivers.into_iter().map(|(mut body, mut buf)| async move {
        let (event, payload) = next_named_frame(&mut body, &mut buf, 15).await;
        assert_eq!(event, "channel:broadcast");
        assert_eq!(payload["data"]["n"], N);
    });
    futures::future::join_all(waits).await;
    let elapsed = started.elapsed();
    eprintln!(
        "throughput sanity check: {N} subscribers received one publish in {elapsed:?} \
         ({:.1} deliveries/sec)",
        N as f64 / elapsed.as_secs_f64().max(0.001)
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "delivering to {N} subscribers took {elapsed:?}, which is well outside a sane bound"
    );
}
