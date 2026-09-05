//! Cross-process proof that Postgres `LISTEN`/`NOTIFY` really carries
//! realtime events between two independent server instances sharing one
//! database — not just the in-process `publish`/`fan_out` plumbing
//! `crates/server/src/realtime.rs`'s own `#[cfg(test)] mod tests` covers
//! (those never leave one process, so they cannot exercise
//! `notify_cross_node`/`start_cross_node_listener` at all).
//!
//! Two full `App`s each get a real `axum::serve` listener on their own
//! ephemeral port, both pointed at the same `TEST_POSTGRES_URL`
//! database. A real `reqwest` client opens `GET /api/realtime` against
//! instance A and subscribes to `posts`; a second real client then POSTs
//! a record create to instance B's REST API. The assertion is that A's
//! SSE stream — a connection instance B has never seen — receives the
//! `create` event, which only Postgres `NOTIFY` can have delivered.
//!
//! Skips (rather than fails) when `TEST_POSTGRES_URL` is unset, matching
//! `crates/db/tests/postgres.rs`'s convention for Postgres-only tests.

use std::time::Duration;

use bytes::Bytes;
use cratebase_core::{Collection, CollectionType, Field, FieldKind, FieldType};
use cratebase_db::{Db, Executor};
use cratebase_server::app::App;
use cratebase_server::config::Config;
use futures::{Stream, StreamExt};
use serde_json::{json, Value};

/// A `Config` for one node: its own data directory (hooks/migrations/
/// local storage must not collide between the two instances), but the
/// same database as every other node built with the same `url`.
fn node_config(url: &str, dir: &std::path::Path) -> Config {
    Config {
        database_url: url.to_string(),
        log_requests: false,
        secret: "test-secret-0123456789".into(),
        ..Config::for_data_dir(dir)
    }
}

/// Bind a real TCP listener on an OS-assigned port and start serving
/// `app` on it in the background, exactly as `App::serve` does (down to
/// `into_make_service_with_connect_info`, since some middleware resolves
/// the caller's IP off it) minus the `bootstrap()` call, which the
/// caller has already done so it can seed a collection first.
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

/// Read one complete SSE frame (`field:value` lines terminated by a
/// blank line) off a chunked HTTP body, blocking on more chunks until a
/// full frame is buffered. Returns the `event:` name (`None` for a
/// nameless keep-alive comment) and the `data:` line parsed as JSON
/// (`Value::Null` when there was none, e.g. a keep-alive ping).
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

#[tokio::test]
async fn cross_node_realtime_round_trip_through_postgres_listen_notify() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping cross_node_realtime_round_trip_through_postgres_listen_notify: \
             TEST_POSTGRES_URL not set"
        );
        return;
    };

    // Fresh schema on its own short-lived connection, before either node
    // opens its own pool — same convention as `postgres_full_suite`.
    {
        let dir = tempfile::tempdir().unwrap();
        let wipe = Db::connect(&url, &dir.path().to_string_lossy())
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
    let app_a = App::new(node_config(&url, dir_a.path()));
    let app_b = App::new(node_config(&url, dir_b.path()));
    // Node A boots and creates the collection first; node B's own
    // `bootstrap` — which ends with `collections.load()` — picks it up
    // from `_collections` during its own startup, exactly as it would
    // if it joined a cluster after the schema already existed. Nothing
    // here re-teaches B about a *later* schema change; that is a
    // separate, real limitation (no cross-node schema-reload signal)
    // and out of scope for this realtime test.
    app_a.bootstrap().await.expect("bootstrap node A");

    // A public collection: anonymous list/create, so the round trip
    // below exercises cross-node fan-out itself rather than the rule
    // engine (already covered elsewhere).
    let mut posts = Collection::new("posts", CollectionType::Base);
    let pos = posts.fields.len() - 2;
    posts.fields.insert(
        pos,
        Field::new("title", FieldKind::default_for(FieldType::Text)),
    );
    // Large enough that a delete snapshot carrying it (with_hidden or
    // not) blows Postgres's 8000-byte `NOTIFY` payload cap on its own —
    // exercises the minimal-payload delete fallback below.
    posts.fields.insert(
        pos + 1,
        Field::new("body", FieldKind::default_for(FieldType::Text)),
    );
    posts.list_rule = Some(String::new());
    posts.create_rule = Some(String::new());
    posts.delete_rule = Some(String::new());
    app_a
        .db()
        .collections
        .insert(&*app_a.db().engine, &posts)
        .await
        .expect("create posts collection");

    app_b.bootstrap().await.expect("bootstrap node B");

    let addr_a = spawn_node(app_a.clone()).await;
    let addr_b = spawn_node(app_b.clone()).await;

    let client = reqwest::Client::new();

    // Open the SSE stream on instance A and read its PB_CONNECT hello.
    let sse = client
        .get(format!("http://{addr_a}/api/realtime"))
        .send()
        .await
        .expect("GET /api/realtime on node A");
    assert!(sse.status().is_success(), "{}", sse.status());
    let mut body = sse.bytes_stream();
    let mut buf = String::new();

    let (event, hello) =
        tokio::time::timeout(Duration::from_secs(5), next_sse_frame(&mut body, &mut buf))
            .await
            .expect("PB_CONNECT hello timed out");
    assert_eq!(event.as_deref(), Some("PB_CONNECT"));
    let client_id = hello["clientId"]
        .as_str()
        .expect("hello carries a clientId")
        .to_string();

    // Subscribe that client to "posts", still against node A.
    let submit = client
        .post(format!("http://{addr_a}/api/realtime"))
        .json(&json!({ "clientId": client_id, "subscriptions": ["posts"] }))
        .send()
        .await
        .expect("POST /api/realtime on node A");
    assert_eq!(submit.status(), 204, "{:?}", submit.text().await);

    // From node B — a process that has never seen this SSE connection —
    // create a record through the real REST API.
    let started = std::time::Instant::now();
    let create = client
        .post(format!("http://{addr_b}/api/collections/posts/records"))
        .json(&json!({ "title": "cross-node" }))
        .send()
        .await
        .expect("POST records on node B");
    let create_status = create.status();
    let created: Value = create.json().await.expect("created record body");
    assert_eq!(create_status, 200, "{created}");

    // The event must arrive on A's stream — through Postgres NOTIFY,
    // since instance A never touched the database for this write itself.
    let (event, payload) = loop {
        let (event, payload) =
            tokio::time::timeout(Duration::from_secs(5), next_sse_frame(&mut body, &mut buf))
                .await
                .expect("cross-node realtime event never arrived within 5s");
        // Skip keep-alive comments (nameless, no `data:`); anything
        // named is the one subscription this client has.
        if event.is_some() {
            break (event, payload);
        }
    };
    let latency = started.elapsed();
    eprintln!("cross-node realtime round trip: {latency:?}");

    assert_eq!(event.as_deref(), Some("posts"));
    assert_eq!(payload["action"], "create");
    assert_eq!(payload["record"]["title"], "cross-node");
    assert_eq!(payload["record"]["id"], created["id"]);

    // A round trip through a real network stack, two Postgres pools and
    // LISTEN/NOTIFY should still land well under the 5s deadline above,
    // or something regressed badly enough to be worth failing loudly on.
    assert!(
        latency < Duration::from_secs(5),
        "cross-node realtime latency {latency:?} exceeded the test's own timeout"
    );

    // --- large-field delete: minimal-payload fallback -----------------
    //
    // A record whose "body" alone is well over Postgres's 8000-byte
    // `NOTIFY` cap. Its delete's cross-node snapshot can't ride along in
    // full, so `notify_cross_node` must fall back to an id-only
    // snapshot rather than silently dropping the notify (the old
    // behavior) — proven by the delete event still arriving on A with
    // just the id, not by it carrying `body`.
    let huge_body = "x".repeat(20_000);
    let create_huge = client
        .post(format!("http://{addr_b}/api/collections/posts/records"))
        .json(&json!({ "title": "huge", "body": huge_body }))
        .send()
        .await
        .expect("POST huge record on node B");
    assert_eq!(create_huge.status(), 200, "{:?}", create_huge.text().await);
    let created_huge: Value = create_huge.json().await.expect("created huge record body");
    let huge_id = created_huge["id"]
        .as_str()
        .expect("huge record has an id")
        .to_string();

    // Drain A's stream past that create event (not the assertion here;
    // the delete fallback is) before triggering the delete.
    loop {
        let (event, payload) =
            tokio::time::timeout(Duration::from_secs(5), next_sse_frame(&mut body, &mut buf))
                .await
                .expect("huge record's cross-node create event never arrived within 5s");
        if event.is_some() {
            assert_eq!(payload["action"], "create");
            assert_eq!(payload["record"]["id"], huge_id);
            break;
        }
    }

    let delete_huge = client
        .delete(format!(
            "http://{addr_b}/api/collections/posts/records/{huge_id}"
        ))
        .send()
        .await
        .expect("DELETE huge record on node B");
    assert_eq!(delete_huge.status(), 204, "{:?}", delete_huge.text().await);

    let (event, payload) = loop {
        let (event, payload) =
            tokio::time::timeout(Duration::from_secs(5), next_sse_frame(&mut body, &mut buf))
                .await
                .expect("cross-node delete event never arrived within 5s");
        if event.is_some() {
            break (event, payload);
        }
    };
    assert_eq!(event.as_deref(), Some("posts"));
    assert_eq!(payload["action"], "delete");
    assert_eq!(payload["record"]["id"], huge_id);
    // The fallback path: the minimal snapshot never carried `body`, so
    // there was nothing for the receiving node to project into the
    // delivered event — it renders as the field's empty zero value, not
    // the 20,000-char content this record was created with.
    let delivered_body = payload["record"]["body"].as_str().unwrap_or_default();
    assert!(
        delivered_body.is_empty(),
        "delete event unexpectedly carried the huge `body` field ({} bytes): the minimal-\
         payload fallback did not take effect",
        delivered_body.len()
    );
}
