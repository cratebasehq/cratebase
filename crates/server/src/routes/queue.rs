//! `POST /api/queue/enqueue`, `POST /api/queue/jobs/{id}/retry` and
//! `DELETE /api/queue/jobs/{id}` — the canonical, always-mounted surface
//! for `crate::queue` (superuser or API key; `crate::extract::RequireSuperuser`
//! already accepts both — an API key always resolves to a superuser
//! identity, see that extractor's doc). `POST /api/plugins/queue/enqueue`
//! (and the `/jobs/{id}/...` pair alongside it) keep working unchanged as
//! aliases, mounted by `QueuePlugin::routes` in `crate::queue` itself.
//!
//! Unconditional, unlike `crate::routes::llm`'s toggle-gated merge: the
//! Queue plugin is now registered at every boot regardless of
//! `settings.queue.enabled` (see `crate::app::App::bootstrap`'s doc for
//! why), so these routes are always reachable — enqueueing, retrying and
//! deleting jobs works even while the worker itself is toggled off; the
//! job just sits there until an operator flips the setting on, live, with
//! no restart (see `crate::queue::QueuePlugin::setup`).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, post};
use axum::{Json, Router};

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::ApiResult;
use crate::queue::{self, EnqueueBody, EnqueueResponse};

pub fn router() -> Router<App> {
    Router::new()
        .route("/queue/enqueue", post(enqueue))
        .route("/queue/jobs/{id}/retry", post(retry))
        .route("/queue/jobs/{id}", delete(remove))
}

async fn enqueue(
    State(app): State<App>,
    _su: RequireSuperuser,
    Json(body): Json<EnqueueBody>,
) -> ApiResult<Json<EnqueueResponse>> {
    queue::enqueue_from_body(&app, body).await.map(Json)
}

async fn retry(
    State(app): State<App>,
    _su: RequireSuperuser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    queue::retry_job(&app, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    State(app): State<App>,
    _su: RequireSuperuser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    queue::delete_job(&app, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
