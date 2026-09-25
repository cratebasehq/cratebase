//! Dogfooding bug report: a running server never saw a collection
//! created by a *separate* process sharing the same on-disk SQLite
//! database (`cratebase schema push`, most notably) -- every request for
//! it 404'd with "Missing collection context." until the server
//! restarted, because `cratebase_db::collections::CollectionStore`'s
//! in-memory snapshot only ever got reloaded by *this* process's own
//! writes.
//!
//! `App::start_schema_watch` (crates/server/src/app.rs) now polls
//! `cratebase_db::collections::watermark` (a cheap `_collections`
//! `COUNT`/`MAX` pair) every `SCHEMA_WATCH_INTERVAL` and reloads the
//! snapshot the moment it disagrees with the last-seen value. This test
//! reproduces the real shape of the bug -- two separate `App` instances
//! (standing in for a running server and a separate `cratebase schema
//! push` invocation) against one on-disk data dir, no in-memory SQLite
//! shortcut, since `:memory:` is private to a single connection and
//! could never have shared anything between them in the first place.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use serde_json::{json, Value};
use tower::ServiceExt;

fn test_config(dir: &std::path::Path) -> Config {
    // `Config::for_data_dir` derives `hooks_dir`/`migrations_dir`/
    // `seed_dir` as *siblings* of the data dir, not children of it (see
    // `crate::config::sibling`) -- passing the tempdir's own root would
    // put those a level up, outside the tempdir entirely (i.e. straight
    // into the real `/tmp`), leaking directories that could collide with
    // other tests. Nesting under `pb_data`, the same convention
    // `Config::memory` and the jsvm hot-reload tests use, keeps
    // everything this test touches inside `dir`.
    let mut cfg = Config::for_data_dir(dir.join("pb_data"));
    cfg.secret = "test-secret-0123456789".into();
    cfg.log_requests = false;
    cfg
}

async fn get(app: &App, uri: &str) -> (StatusCode, Value) {
    let response = cratebase_server::router(app.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("response");
    split(response).await
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

#[tokio::test]
async fn a_collection_created_by_a_separate_process_is_visible_without_restarting_this_one() {
    let dir = tempfile::tempdir().expect("temp dir");

    // "App A": the already-running server.
    let app_a = App::new(test_config(dir.path()));
    app_a.bootstrap().await.expect("bootstrap A");

    // Not there yet, from A's point of view -- the eventual 200 below can
    // only be the schema-change poll's doing, not something A already
    // knew about.
    let (status, body) = get(&app_a, "/api/collections/widgets/records").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "widgets must not exist yet: {body}"
    );

    // "App B": a *separate* process sharing the same data dir -- exactly
    // what `cratebase schema push` is, and exactly what `main.rs`'s own
    // `schema()` CLI handler does: a fresh `App`, bootstrapped against
    // the same on-disk database, calling the same
    // `routes::schema::plan_and_apply` the CLI calls, with no HTTP layer
    // and no auth (the CLI already has filesystem-level access).
    let app_b = App::new(test_config(dir.path()));
    app_b.bootstrap().await.expect("bootstrap B");

    let info = cratebase_server::extract::RequestInfo::default();
    cratebase_server::routes::schema::plan_and_apply(
        &app_b,
        &json!({
            "collections": [{
                "name": "widgets",
                "type": "base",
                "listRule": "",
                "viewRule": "",
                "createRule": "",
                "updateRule": "",
                "deleteRule": "",
                "fields": [{"name": "name", "type": "text"}],
            }],
        }),
        false,
        false,
        &info,
        None,
    )
    .await
    .expect("schema push from the second process");

    // App A never wrote `_collections` itself and is never restarted --
    // only `App::start_schema_watch`'s background poll can make this
    // eventually succeed. Bounded wait, well over the poll interval.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let (status, body) = get(&app_a, "/api/collections/widgets/records").await;
        if status == StatusCode::OK {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "app A never picked up app B's schema push within the bounded wait (last: {status} {body})"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_collection_deleted_by_a_separate_process_stops_being_served_without_restarting_this_one(
) {
    let dir = tempfile::tempdir().expect("temp dir");

    let app_a = App::new(test_config(dir.path()));
    app_a.bootstrap().await.expect("bootstrap A");

    let info = cratebase_server::extract::RequestInfo::default();
    cratebase_server::routes::schema::plan_and_apply(
        &app_a,
        &json!({
            "collections": [{
                "name": "widgets",
                "type": "base",
                "listRule": "",
                "viewRule": "",
                "createRule": "",
                "updateRule": "",
                "deleteRule": "",
                "fields": [{"name": "name", "type": "text"}],
            }],
        }),
        false,
        false,
        &info,
        None,
    )
    .await
    .expect("initial schema push");

    let (status, body) = get(&app_a, "/api/collections/widgets/records").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // A second process deletes it straight from the store -- the same
    // shape of external change as a `cratebase schema push` that drops a
    // collection. Regression pin for a real bug found writing this test:
    // deleting the very collection this same test just created can bring
    // `_collections`' row count *and* its max `updated` back to exactly
    // what they were before that create ever happened, touching no other
    // row -- a poller that tracked "the last watermark value I, the
    // poller, happened to observe" independently of what this process's
    // own create/reload already put in the cache would see that
    // coincidence as "nothing changed" and never notice the delete at
    // all. See `cratebase_db::collections::Snapshot::watermark`'s doc.
    let app_b = App::new(test_config(dir.path()));
    app_b.bootstrap().await.expect("bootstrap B");
    app_b
        .db()
        .collections
        .delete(app_b.db().engine.as_ref(), "widgets")
        .await
        .expect("delete widgets from the second process");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let (status, body) = get(&app_a, "/api/collections/widgets/records").await;
        if status == StatusCode::NOT_FOUND {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "app A never noticed app B's delete within the bounded wait (last: {status} {body})"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
