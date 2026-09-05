//! Proves `PostgresEngine::close()` actually stops the
//! `subscribe_realtime` reconnect loop rather than leaving it to
//! reconnect forever. Skips (rather than fails) when `TEST_POSTGRES_URL`
//! is unset, matching every other Postgres-only test's convention.
//!
//! Schema-independent by design (no tables, no `bootstrap()`) so it can
//! run concurrently with `postgres_full_suite` and the multi-node
//! realtime test against the same database without racing a shared
//! schema wipe.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cratebase_db::{Engine, Executor, PostgresEngine, Sql};

/// Number of server-side connections whose last statement was `LISTEN
/// ...` — exactly the dedicated connections `subscribe_realtime` opens,
/// and nothing else in a fresh test database ever issues that command.
async fn listen_connection_count(observer: &PostgresEngine) -> i64 {
    observer
        .query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE query ILIKE 'LISTEN %'",
            &[],
        )
        .await
        .unwrap()
        .and_then(|v| v.as_i64())
        .unwrap_or(-1)
}

#[tokio::test]
async fn close_stops_the_listen_reconnect_loop() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!("skipping close_stops_the_listen_reconnect_loop: TEST_POSTGRES_URL not set");
        return;
    };

    let engine = PostgresEngine::connect(&url, 2).await.unwrap();
    let observer = PostgresEngine::connect(&url, 2).await.unwrap();

    let received = Arc::new(AtomicUsize::new(0));
    let received_clone = received.clone();
    engine.subscribe_realtime(Arc::new(move |_payload: String| {
        received_clone.fetch_add(1, Ordering::SeqCst);
    }));

    // Wait for the dedicated LISTEN connection to actually establish.
    let mut connected = false;
    for _ in 0..50 {
        if listen_connection_count(&observer).await == 1 {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        connected,
        "subscribe_realtime's LISTEN connection never showed up in pg_stat_activity"
    );

    // Sanity: it is genuinely live — a real NOTIFY on another connection
    // reaches our callback.
    observer
        .execute("SELECT pg_notify('cratebase_realtime', 'probe')", &[])
        .await
        .unwrap();
    let mut saw_probe = false;
    for _ in 0..50 {
        if received.load(Ordering::SeqCst) >= 1 {
            saw_probe = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        saw_probe,
        "the LISTEN connection never delivered a real NOTIFY"
    );

    // Close the engine, then kill its dedicated LISTEN connection out
    // from under it — `close()` only closes the pooled connections
    // (`pool.close()`), never this one, so without the fix the loop
    // would treat this exactly like any other transient network blip
    // and reconnect after its normal 3s backoff.
    engine.close().await.unwrap();
    let pid = observer
        .query_scalar(
            "SELECT pid FROM pg_stat_activity WHERE query ILIKE 'LISTEN %' LIMIT 1",
            &[],
        )
        .await
        .unwrap()
        .and_then(|v| v.as_i64());
    if let Some(pid) = pid {
        observer
            .execute("SELECT pg_terminate_backend($1)", &[Sql::Int(pid)])
            .await
            .unwrap();
    }

    // Give it comfortably longer than the loop's own 3s reconnect
    // backoff to prove it does *not* come back.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        listen_connection_count(&observer).await,
        0,
        "a LISTEN connection reappeared after close(): the reconnect loop did not stop"
    );
}
