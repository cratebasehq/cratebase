//! Persists a bounded history of API requests to `_request_logs`, feeding
//! the superuser-only `GET /api/logs` dashboard page (see
//! `routes::logs`). Applied as an outermost layer in `build_app` so it
//! sees every `/api/*` call, not just ones a particular route module opts
//! into.
//!
//! The middleware itself never touches the database: it pushes an entry
//! onto an unbounded channel and a single background task
//! ([`RequestLogWriter::spawn`]) drains that channel into one multi-row
//! `INSERT` per batch. The first version of this middleware spawned a
//! task per request that ran its own `INSERT` — on SQLite that turned
//! every read-only API call into a write transaction competing for the
//! same writer lock as user traffic, measured in `benchmarks/` as the
//! largest single contributor to the `search` gap vs PocketBase.

use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use cratebase_db::system::{self, RequestLogEntry, REQUEST_LOG_BATCH_MAX};
use cratebase_db::Db;
use tokio::sync::{mpsc, oneshot};

use crate::extract::CurrentAuth;
use crate::state::AppState;

/// Longest an entry sits in the buffer before being written.
const FLUSH_INTERVAL: Duration = Duration::from_millis(250);

enum Msg {
    Entry(RequestLogEntry),
    /// Write everything buffered so far, then signal. Used by the logs
    /// endpoint so a freshly made request is visible on the very next
    /// dashboard read, and by tests.
    Flush(oneshot::Sender<()>),
}

/// Handle to the background log writer. Cheap to clone; `None` inside
/// means request logging is disabled (`LOG_REQUESTS=false`).
#[derive(Clone)]
pub struct RequestLogWriter {
    tx: Option<mpsc::UnboundedSender<Msg>>,
}

impl RequestLogWriter {
    /// A writer that drops everything. Used when logging is disabled.
    pub fn disabled() -> Self {
        Self { tx: None }
    }

    /// Start the background flusher on the current tokio runtime.
    pub fn spawn(db: Db) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run_writer(db, rx));
        Self { tx: Some(tx) }
    }

    pub fn is_enabled(&self) -> bool {
        self.tx.is_some()
    }

    pub fn record(&self, entry: RequestLogEntry) {
        if let Some(tx) = &self.tx {
            // The only way this fails is if the writer task is gone, in
            // which case losing a debugging-aid row is the right outcome.
            let _ = tx.send(Msg::Entry(entry));
        }
    }

    /// Wait until every entry recorded before this call has been written.
    pub async fn flush(&self) {
        if let Some(tx) = &self.tx {
            let (done_tx, done_rx) = oneshot::channel();
            if tx.send(Msg::Flush(done_tx)).is_ok() {
                let _ = done_rx.await;
            }
        }
    }
}

async fn run_writer(db: Db, mut rx: mpsc::UnboundedReceiver<Msg>) {
    let mut buffer: Vec<RequestLogEntry> = Vec::with_capacity(REQUEST_LOG_BATCH_MAX);
    let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(Msg::Entry(entry)) => {
                    buffer.push(entry);
                    if buffer.len() >= REQUEST_LOG_BATCH_MAX {
                        write_batch(&db, &mut buffer).await;
                    }
                }
                Some(Msg::Flush(done)) => {
                    write_batch(&db, &mut buffer).await;
                    let _ = done.send(());
                }
                None => {
                    write_batch(&db, &mut buffer).await;
                    return;
                }
            },
            _ = ticker.tick() => {
                if !buffer.is_empty() {
                    write_batch(&db, &mut buffer).await;
                }
            }
        }
    }
}

async fn write_batch(db: &Db, buffer: &mut Vec<RequestLogEntry>) {
    if buffer.is_empty() {
        return;
    }
    let batch = std::mem::take(buffer);
    if let Err(e) = system::insert_request_logs(db, batch).await {
        tracing::warn!(error = %e, "failed to persist request log batch");
    }
}

/// Times the request and hands the entry to the background writer.
/// `CurrentAuth` resolved here is cached in the request extensions (see
/// `extract.rs`), so the handler's own `CurrentAuth` extractor is a map
/// lookup rather than a second token verification + DB round trip.
pub async fn log_requests(
    State(app): State<AppState>,
    CurrentAuth(auth): CurrentAuth,
    req: Request,
    next: Next,
) -> Response {
    if !app.request_logs.is_enabled() {
        return next.run(req).await;
    }

    let method = req.method().to_string();
    // `req.uri().path()` is relative to the `/api` nest this middleware is
    // layered inside (`build_app` applies it to the nested router, not the
    // whole app), so it reads e.g. `/health` — re-prefix with `/api` so a
    // logged path matches the URL a client actually requested.
    let path = format!("/api{}", req.uri().path());
    let started = Instant::now();

    let response = next.run(req).await;

    let duration_ms = started.elapsed().as_millis() as i64;
    let status = response.status().as_u16() as i64;
    let (auth_id, auth_collection_id) = match auth {
        Some(ctx) if ctx.is_superuser => (Some(ctx.id), None),
        Some(ctx) => (Some(ctx.id), Some(ctx.collection_id)),
        None => (None, None),
    };

    app.request_logs.record(RequestLogEntry {
        method,
        path,
        status,
        duration_ms,
        auth_id,
        auth_collection_id,
        created: cratebase_core::now(),
    });

    response
}
