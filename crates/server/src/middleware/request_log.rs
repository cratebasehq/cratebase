//! Request logging into `_logs` (the auxiliary SQLite database).
//!
//! # Why a background batching writer
//!
//! The first version of this middleware spawned a task per request that
//! ran its own `INSERT`. On SQLite that turned every *read-only* API call
//! into a write transaction competing for the single writer lock with
//! user traffic — measured in `benchmarks/` as the largest single
//! contributor to the read gap against PocketBase. Pushing onto an
//! unbounded channel and letting one task drain it into a multi-row
//! `INSERT` per batch made reads roughly 3x faster, so that design is
//! kept here verbatim; only the row shape changed (PocketBase's
//! `_logs`, not the old `_request_logs`).
//!
//! [`LogWriter::flush`] exists so `GET /api/logs` can show a request that
//! happened milliseconds ago instead of waiting out the flush interval.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;
use cratebase_db::logs::{self, LogEntry, BATCH_MAX};
use cratebase_db::Db;
use serde_json::{json, Map, Value};
use tokio::sync::{mpsc, oneshot};

use crate::app::App;
use crate::middleware::client_ip;

/// Longest an entry sits in the buffer before being written.
const FLUSH_INTERVAL: Duration = Duration::from_millis(250);

/// PocketBase's numeric levels (`slog`): debug -4, info 0, warn 4,
/// error 8. Request rows are always `info`.
pub const LEVEL_DEBUG: i64 = -4;
pub const LEVEL_INFO: i64 = 0;
pub const LEVEL_WARN: i64 = 4;
pub const LEVEL_ERROR: i64 = 8;

enum Msg {
    Entry(Box<LogEntry>),
    /// Write everything buffered so far, then signal.
    Flush(oneshot::Sender<()>),
}

/// Handle to the background log writer. Cheap to clone; `None` inside
/// means logging is disabled (`LOG_REQUESTS=false`).
#[derive(Clone, Default)]
pub struct LogWriter {
    tx: Option<mpsc::UnboundedSender<Msg>>,
}

impl LogWriter {
    /// A writer that drops everything.
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

    /// Queue an already-built entry.
    pub fn record(&self, entry: LogEntry) {
        if let Some(tx) = &self.tx {
            // Only fails when the writer task is gone, in which case
            // losing a diagnostic row is the right outcome.
            let _ = tx.send(Msg::Entry(Box::new(entry)));
        }
    }

    /// Queue an application log line, honouring `settings.logs.minLevel`.
    pub fn write(
        &self,
        settings: &cratebase_core::settings::Logs,
        level: i64,
        message: impl Into<String>,
        data: Value,
    ) {
        if level < settings.min_level {
            return;
        }
        self.record(LogEntry::new(
            level,
            message,
            truncate(data, settings.max_data_size),
        ));
    }

    /// Wait until every entry queued before this call has been written.
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
    let mut buffer: Vec<LogEntry> = Vec::with_capacity(BATCH_MAX);
    let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(Msg::Entry(entry)) => {
                    buffer.push(*entry);
                    if buffer.len() >= BATCH_MAX {
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

async fn write_batch(db: &Db, buffer: &mut Vec<LogEntry>) {
    if buffer.is_empty() {
        return;
    }
    let batch = std::mem::take(buffer);
    if let Err(e) = logs::insert_batch(&*db.logs, batch).await {
        tracing::warn!(error = %e, "failed to persist a log batch");
    }
}

/// Drop the bulky, optional keys once the serialised payload exceeds
/// `max_data_size` bytes (0 = unlimited), so one enormous request cannot
/// bloat the logs database.
fn truncate(data: Value, max_data_size: i64) -> Value {
    if max_data_size <= 0 {
        return data;
    }
    let limit = max_data_size as usize;
    if data.to_string().len() <= limit {
        return data;
    }
    let Value::Object(mut map) = data else {
        return json!({ "truncated": true });
    };
    for key in ["details", "error", "userAgent", "referer"] {
        map.remove(key);
        if Value::Object(map.clone()).to_string().len() <= limit {
            break;
        }
    }
    map.insert("truncated".into(), Value::Bool(true));
    Value::Object(map)
}

/// Times the request, resolves the caller once (cached in the request
/// extensions so the handler's own `Auth` extractor is a map lookup) and
/// hands the row to the background writer.
pub async fn log_requests(State(app): State<App>, req: Request, next: Next) -> Response {
    let logger = app.logger();
    // `LOG_REQUESTS=false` must cost nothing: no settings load, no auth
    // resolution, no allocation. Bail before touching the request.
    if !logger.is_enabled() {
        return next.run(req).await;
    }
    let settings = app.settings();
    // A `minLevel` above `error` silences even failed requests, so there
    // is nothing to resolve or time.
    if settings.logs.min_level > LEVEL_ERROR {
        return next.run(req).await;
    }

    let (mut parts, body) = req.into_parts();
    // Resolved exactly once per request: the result is cached in the
    // request extensions, so the rate limiter and the handler's own
    // extractors are map lookups rather than a second JWT verify plus DB
    // round trip.
    let auth = crate::extract::resolve_and_cache(&mut parts, &app).await;

    let method = parts.method.to_string();
    let uri = crate::middleware::original_uri(&parts);
    let url = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| uri.path().to_string());
    let header_str = |name: header::HeaderName| {
        parts
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    let referer = header_str(header::REFERER);
    let user_agent = header_str(header::USER_AGENT);
    let peer = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0);
    let user_ip = client_ip::client_ip(&parts.headers, peer, &settings.trusted_proxy);
    let remote_ip = client_ip::remote_ip(peer);

    let started = Instant::now();
    let response = next.run(Request::from_parts(parts, body)).await;
    let exec_ms = started.elapsed().as_secs_f64() * 1000.0;

    // PocketBase logs a failed request at `error` level and records the
    // message. `ApiError` stamps the message on the response extensions on
    // its way out, so nothing has to buffer the response body to get it.
    let status = response.status();
    let error = response
        .extensions()
        .get::<LoggedError>()
        .map(|e| e.0.clone())
        .or_else(|| {
            status
                .is_client_error()
                .then(|| status.canonical_reason().unwrap_or("Error").to_string())
        })
        .filter(|_| !status.is_success() && !status.is_redirection());
    let level = if error.is_some() {
        LEVEL_ERROR
    } else {
        LEVEL_INFO
    };
    if level < settings.logs.min_level {
        return response;
    }

    let mut data = Map::new();
    data.insert("type".into(), json!("request"));
    data.insert("method".into(), json!(method));
    data.insert("url".into(), json!(url));
    data.insert("status".into(), json!(status.as_u16()));
    data.insert("execTime".into(), json!(exec_ms));
    data.insert(
        "auth".into(),
        json!(auth
            .as_ref()
            .map(|a| a.collection_name.as_str())
            .unwrap_or("")),
    );
    if settings.logs.log_ip {
        data.insert("userIP".into(), json!(user_ip));
        data.insert("remoteIP".into(), json!(remote_ip));
    } else {
        data.insert("userIP".into(), json!(""));
        data.insert("remoteIP".into(), json!(""));
    }
    data.insert("referer".into(), json!(referer));
    data.insert("userAgent".into(), json!(user_agent));
    if settings.logs.log_auth_id {
        data.insert(
            "authId".into(),
            json!(auth.as_ref().map(|a| a.id.as_str()).unwrap_or("")),
        );
    }
    // Present only on failures — the conformance suite asserts a
    // successful row has no `error` key at all.
    if let Some(error) = error {
        data.insert("error".into(), json!(error));
    }

    logger.record(LogEntry::new(
        level,
        format!("{method} {}", strip_query(&data)),
        truncate(Value::Object(data.clone()), settings.logs.max_data_size),
    ));

    response
}

/// The message an [`crate::http_error::ApiError`] rendered, carried on the
/// response so the log middleware can record it without reading the body.
#[derive(Clone, Debug)]
pub struct LoggedError(pub String);

/// PocketBase's `message` is `"<METHOD> <path>"` without the query
/// string, while `data.url` keeps it.
fn strip_query(data: &Map<String, Value>) -> String {
    let url = data.get("url").and_then(Value::as_str).unwrap_or("");
    url.split('?').next().unwrap_or(url).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_only_kicks_in_past_the_limit() {
        let data = json!({"type": "request", "userAgent": "x".repeat(200)});
        assert_eq!(truncate(data.clone(), 0), data);
        let cut = truncate(data, 64);
        assert_eq!(cut["type"], "request");
        assert!(cut.get("userAgent").is_none());
        assert_eq!(cut["truncated"], true);
    }

    #[test]
    fn message_drops_the_query_string() {
        let mut m = Map::new();
        m.insert("url".into(), json!("/api/collections/posts/records?page=2"));
        assert_eq!(strip_query(&m), "/api/collections/posts/records");
    }
}
