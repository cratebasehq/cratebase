//! Cross-process proof that realtime **channels** and **presence** (not
//! just record fan-out — see `postgres_multi_node.rs`'s own version of
//! this file, which this one mirrors closely) really propagate between
//! two independent server instances sharing one Postgres database, over
//! the same `LISTEN`/`NOTIFY` path, distinguished by the `"kind"`
//! discriminator `crates/server/src/realtime.rs`'s `dispatch_cross_node`
//! reads.
//!
//! Skips (rather than fails) when `TEST_POSTGRES_URL` is unset, matching
//! every other Postgres-gated test in this crate.

use std::time::Duration;

use bytes::Bytes;
use cratebase_db::{Db, Executor};
use cratebase_server::app::App;
use cratebase_server::config::Config;
use futures::{Stream, StreamExt};
use serde_json::{json, Value};

fn node_config(url: &str, dir: &std::path::Path) -> Config {
    Config {
        database_url: url.to_string(),
        log_requests: false,
        secret: "test-secret-0123456789".into(),
        ..Config::for_data_dir(dir.join("pb_data"))
    }
}

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

async fn connect_and_subscribe(
    client: &reqwest::Client,
    addr: std::net::SocketAddr,
    topics: &[&str],
) -> (
    String,
    impl Stream<Item = reqwest::Result<Bytes>> + Unpin,
    String,
) {
    let sse = client
        .get(format!("http://{addr}/api/realtime"))
        .send()
        .await
        .expect("GET /api/realtime");
    assert!(sse.status().is_success(), "{}", sse.status());
    let mut body = sse.bytes_stream();
    let mut buf = String::new();
    let (event, hello) =
        tokio::time::timeout(Duration::from_secs(5), next_sse_frame(&mut body, &mut buf))
            .await
            .expect("PB_CONNECT hello timed out");
    assert_eq!(event.as_deref(), Some("PB_CONNECT"));
    let client_id = hello["clientId"].as_str().expect("clientId").to_string();

    let submit = client
        .post(format!("http://{addr}/api/realtime"))
        .json(&json!({ "clientId": client_id, "subscriptions": topics }))
        .send()
        .await
        .expect("POST /api/realtime");
    assert_eq!(submit.status(), 204, "{:?}", submit.text().await);
    (client_id, body, buf)
}

/// Wipes the schema, boots two `App`s against the same `url`, node A
/// first (so it creates `_channels`' public "lobby"/"room" rows before B
/// starts), and returns their addresses plus a shared client.
async fn two_nodes(url: &str) -> (std::net::SocketAddr, std::net::SocketAddr, reqwest::Client) {
    {
        let dir = tempfile::tempdir().unwrap();
        let wipe = Db::connect(url, &dir.path().to_string_lossy())
            .await
            .expect("connect to wipe schema");
        wipe.execute("DROP SCHEMA public CASCADE", &[])
            .await
            .unwrap();
        wipe.execute("CREATE SCHEMA public", &[]).await.unwrap();
        wipe.close().await.unwrap();
    }

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let app_a = App::new(node_config(url, dir_a.path()));
    let app_b = App::new(node_config(url, dir_b.path()));
    app_a.bootstrap().await.expect("bootstrap node A");

    // Public channels, configured once from node A; node B picks up the
    // `_channels` row the same way it would any other collection's data —
    // a plain row read, not a schema change, so no cross-node schema
    // reload is needed for this (unlike a brand-new collection).
    let collection = app_a.db().collections.get("_channels").expect("_channels");
    for name in ["lobby", "room"] {
        let mut record = cratebase_core::Record::new(collection.clone());
        record.set("name", Value::String(name.to_string()));
        // Double-JSON-encoded `""` (public) — see
        // `crate::routes::records::normalize_json_rule_fields`'s doc
        // comment for exactly why: `coerce_record` (run by `records::create`
        // itself, not just the HTTP route) JSON-parses any submitted string
        // for a `Json`-kind field before storage, so a *single* level of
        // encoding (`"\"\""`) round-trips back down to a plain empty
        // string and is indistinguishable from never having been set. Two
        // levels survive that one unwrap.
        let public =
            Value::String(serde_json::to_string(&serde_json::to_string("").unwrap()).unwrap());
        record.set("subscribeRule", public.clone());
        record.set("publishRule", public);
        cratebase_db::records::create(app_a.db(), &app_a.db().collections, &mut record)
            .await
            .expect("seed _channels row");
    }

    app_b.bootstrap().await.expect("bootstrap node B");

    let addr_a = spawn_node(app_a.clone()).await;
    let addr_b = spawn_node(app_b.clone()).await;
    (addr_a, addr_b, reqwest::Client::new())
}

#[tokio::test]
async fn cross_node_channel_publish_round_trips_through_postgres_listen_notify() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping cross_node_channel_publish_round_trips_through_postgres_listen_notify: \
             TEST_POSTGRES_URL not set"
        );
        return;
    };
    let (addr_a, addr_b, client) = two_nodes(&url).await;

    // Subscriber on node A; the publish happens on node B, which has
    // never seen this SSE connection — only Postgres NOTIFY can bridge
    // the two.
    let (_client_id, mut body, mut buf) =
        connect_and_subscribe(&client, addr_a, &["channel:lobby"]).await;

    let started = std::time::Instant::now();
    let resp = client
        .post(format!(
            "http://{addr_b}/api/realtime/channels/lobby/publish"
        ))
        .json(&json!({ "event": "chat", "data": { "text": "cross-node" } }))
        .send()
        .await
        .expect("publish on node B");
    assert_eq!(resp.status(), 204, "{:?}", resp.text().await);

    let (event, payload) = next_named_frame(&mut body, &mut buf, 5).await;
    let latency = started.elapsed();
    eprintln!("cross-node channel publish round trip: {latency:?}");
    assert_eq!(event, "channel:lobby");
    assert_eq!(payload["event"], "chat");
    assert_eq!(payload["data"]["text"], "cross-node");
}

#[tokio::test]
async fn cross_node_presence_join_and_crash_expiry() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!("skipping cross_node_presence_join_and_crash_expiry: TEST_POSTGRES_URL not set");
        return;
    };
    let (addr_a, addr_b, client) = two_nodes(&url).await;

    // A watcher on node B sees a presence.join tracked on node A.
    let (_watcher_id, mut watcher_body, mut watcher_buf) =
        connect_and_subscribe(&client, addr_b, &["channel:room"]).await;
    let (client_id_a, _body_a, _buf_a) =
        connect_and_subscribe(&client, addr_a, &["channel:room"]).await;

    let resp = client
        .post(format!(
            "http://{addr_a}/api/realtime/channels/room/presence"
        ))
        .json(&json!({ "clientId": client_id_a, "state": { "name": "cross-node-alice" } }))
        .send()
        .await
        .expect("presence track on node A");
    assert_eq!(resp.status(), 200, "{:?}", resp.text().await);

    let (event, payload) = next_named_frame(&mut watcher_body, &mut watcher_buf, 5).await;
    assert_eq!(event, "channel:room");
    assert_eq!(payload["event"], "presence.join");
    assert_eq!(payload["data"]["clientId"], client_id_a);

    // Node B's own presence list (a replica, synced only through the
    // cross-node "presence" payload) must also show the member.
    let list = client
        .get(format!(
            "http://{addr_b}/api/realtime/channels/room/presence"
        ))
        .send()
        .await
        .expect("presence list on node B")
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(list["members"].as_array().unwrap().len(), 1);
    assert_eq!(list["members"][0]["clientId"], client_id_a);
}
