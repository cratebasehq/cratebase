//! Superuser-only read access to the bounded request-log table the
//! `request_log` middleware fills in. This module only ever reads
//! `_request_logs` — inserts happen exclusively from the middleware layer
//! wrapping every `/api/*` call (see `crate::request_log`).

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use cratebase_db::system;
use serde::Deserialize;
use serde_json::Value;

use crate::extract::RequireAdmin;
use crate::http_error::ApiResult;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/logs", get(list))
}

#[derive(Deserialize)]
struct ListQuery {
    page: Option<i64>,
    #[serde(rename = "perPage")]
    per_page: Option<i64>,
    /// Plain substring match against the request path — see
    /// `system::list_request_logs`'s doc comment for why this isn't the
    /// full filter grammar.
    filter: Option<String>,
}

async fn list(
    State(app): State<AppState>,
    _admin: RequireAdmin,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    // Entries are written in batches by a background task; make sure
    // everything recorded so far is visible before reading.
    app.request_logs.flush().await;
    let result = system::list_request_logs(
        &app.db,
        q.page.unwrap_or(1),
        q.per_page.unwrap_or(30),
        q.filter.as_deref(),
    )
    .await?;

    Ok(Json(serde_json::json!({
        "page": result.page,
        "perPage": result.per_page,
        "totalItems": result.total_items,
        "totalPages": result.total_pages,
        "items": result.items,
    })))
}
