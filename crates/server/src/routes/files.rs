use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use cratebase_core::AppError;
use cratebase_db::records;
use cratebase_db::resolver::{evaluate_rule, RequestContext, RuleOutcome};

use crate::extract::CurrentAuth;
use crate::helpers::{file_key, load_collection};
use crate::http_error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/files/{collection}/{record_id}/{filename}", get(download))
}

/// Streams the file straight from the storage backend to the response body
/// rather than buffering it in memory, so a multi-hundred-MB upload doesn't
/// cost a multi-hundred-MB allocation per concurrent download.
async fn download(
    State(app): State<AppState>,
    Path((collection_name, record_id, filename)): Path<(String, String, String)>,
    CurrentAuth(auth): CurrentAuth,
) -> ApiResult<Response> {
    let collection = load_collection(&app, &collection_name).await?;
    let ctx = RequestContext { auth, data: None };

    // A file is only downloadable if its owning record is currently
    // visible under the collection's viewRule — files piggyback on record
    // access control rather than having their own rule type.
    let outcome = evaluate_rule(&collection.view_rule, &collection, app.db.backend, &ctx, 0)?;
    let rule_filter = match outcome {
        RuleOutcome::DenyAll => {
            return Err(ApiError(AppError::Forbidden(
                "you are not allowed to access this file".into(),
            )))
        }
        RuleOutcome::AllowAll => None,
        RuleOutcome::Filtered(f) => Some(f),
    };
    records::get_record(&app.db, &collection, &record_id, rule_filter).await?;

    let key = file_key(&collection.id, &record_id, &filename);
    let stream = app.storage.get_stream(&key).await?;

    let content_type = mime_guess::from_path(&filename).first_or_octet_stream();
    let response = (
        [(header::CONTENT_TYPE, content_type.essence_str().to_string())],
        Body::from_stream(stream),
    );
    Ok(response.into_response())
}
