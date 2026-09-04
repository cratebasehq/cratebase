//! `GET /api/crons` and `POST /api/crons/{id}` — superuser only.
//!
//! The list is `[{id, expression}]` in registration order, which is what
//! the dashboard renders and what the captured fixture holds. Running a
//! job is synchronous (so the caller learns it finished) and answers
//! `204 No Content`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::app::App;
use crate::cron::CronJob;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiResult};

pub fn router() -> Router<App> {
    Router::new()
        .route("/crons", get(list))
        .route("/crons/{id}", post(run))
}

async fn list(State(app): State<App>, _su: RequireSuperuser) -> Json<Vec<CronJob>> {
    Json(app.cron().list())
}

async fn run(
    State(app): State<App>,
    _su: RequireSuperuser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    app.cron()
        .run(&id)
        .await
        .map_err(|_| ApiError::not_found("Missing or invalid cron job."))?;
    Ok(StatusCode::NO_CONTENT)
}
