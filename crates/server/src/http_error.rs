use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use cratebase_core::AppError;

/// Wraps `cratebase_core::AppError` so it can be returned directly from
/// axum handlers as `Result<T, ApiError>`.
pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        ApiError(e)
    }
}

impl From<cratebase_db::DbError> for ApiError {
    fn from(e: cratebase_db::DbError) -> Self {
        ApiError(e.into())
    }
}

impl From<cratebase_storage::StorageError> for ApiError {
    fn from(e: cratebase_storage::StorageError) -> Self {
        let app_err = match e {
            cratebase_storage::StorageError::NotFound(m) => AppError::NotFound(m),
            other => AppError::Internal(other.to_string()),
        };
        ApiError(app_err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = self.0.body();
        let status = StatusCode::from_u16(body.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        if status.is_server_error() {
            tracing::error!(message = %body.message, "request failed");
        }
        (status, Json(body)).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
