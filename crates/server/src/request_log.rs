//! Persists a bounded history of API requests to `_request_logs`, feeding
//! the superuser-only `GET /api/logs` dashboard page (see
//! `routes::logs`). Applied as an outermost layer in `build_app` so it
//! sees every `/api/*` call, not just ones a particular route module opts
//! into.

use std::time::Instant;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use cratebase_db::system::{self, RequestLogEntry};

use crate::extract::CurrentAuth;
use crate::state::AppState;

/// Times the request, then fires the DB insert off on its own task rather
/// than awaiting it inline — logging a request should never add latency to
/// (or, worse, fail) the response it's describing.
pub async fn log_requests(
    State(app): State<AppState>,
    CurrentAuth(auth): CurrentAuth,
    req: Request,
    next: Next,
) -> Response {
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

    tokio::spawn(async move {
        let entry = RequestLogEntry {
            method,
            path,
            status,
            duration_ms,
            auth_id,
            auth_collection_id,
        };
        if let Err(e) = system::insert_request_log(&app.db, entry).await {
            tracing::warn!(error = %e, "failed to persist request log");
        }
    });

    response
}
