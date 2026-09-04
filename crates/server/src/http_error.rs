//! Rendering [`AppError`] as PocketBase's error envelope, plus the
//! extractors that make axum's own rejections look the same.
//!
//! PocketBase answers *every* failure with
//! `{"status": <int>, "message": "...", "data": {...}}`. axum's built-in
//! rejections answer with `text/plain`, which breaks the official SDK's
//! `ClientResponseError` parsing — hence [`ApiJson`] and [`ApiQuery`],
//! thin wrappers whose rejection type is [`ApiError`].

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use cratebase_core::AppError;
use serde::de::DeserializeOwned;

/// Wraps [`AppError`] so handlers can `return Err(...)` directly.
///
/// `data` overrides the envelope's `data` object when set. PocketBase
/// nests validation errors for structured payloads — settings report
/// `data.meta.appName.code`, batch reports
/// `data.requests["0"].response.status` — and [`AppError::Validation`] is
/// deliberately flat, so those routes supply the tree here instead.
#[derive(Debug)]
pub struct ApiError {
    pub error: AppError,
    pub data: Option<std::collections::BTreeMap<String, serde_json::Value>>,
}

pub type ApiResult<T> = Result<T, ApiError>;

#[allow(non_snake_case)]
pub fn ApiError(error: AppError) -> ApiError {
    ApiError { error, data: None }
}

impl ApiError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        ApiError(AppError::bad_request(msg))
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        ApiError(AppError::not_found(msg))
    }
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        ApiError(AppError::unauthorized(msg))
    }
    pub fn forbidden(msg: impl Into<String>) -> Self {
        ApiError(AppError::forbidden(msg))
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        ApiError(AppError::internal(msg))
    }

    /// A 400 whose `data` is an arbitrary tree of per-field errors.
    pub fn nested_validation(
        message: impl Into<String>,
        data: serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        ApiError {
            error: AppError::BadRequest(message.into()),
            data: Some(data.into_iter().collect()),
        }
    }
}

/// The statuses PocketBase answers a failed API rule with. Verified
/// against v0.40.2 by `tests/conformance/errors.test.ts`; they are not
/// what you would guess, so `routes::records` uses these constructors
/// rather than inventing its own.
///
/// * a failing `listRule` is **not** an error — return an empty list;
/// * `viewRule` / `updateRule` / `deleteRule` → 404;
/// * `createRule` → 400 `"Failed to create record."` with `data: {}`;
/// * a `null` (superuser-only) rule → 403.
pub mod rule_errors {
    use super::*;

    /// A `null` rule: only superusers may do this.
    pub fn superusers_only() -> AppError {
        AppError::Forbidden("Only superusers can perform this action.".into())
    }

    /// A failing `viewRule` / `updateRule` / `deleteRule`: the record is
    /// reported as simply not there, so a rule cannot be used as an
    /// existence oracle.
    pub fn hidden_record() -> AppError {
        AppError::not_found("")
    }

    /// A failing `createRule`.
    pub fn create_denied() -> AppError {
        AppError::BadRequest("Failed to create record.".into())
    }

    /// An unknown collection in the URL.
    pub fn missing_collection() -> AppError {
        AppError::NotFound("Missing collection context.".into())
    }
}

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

impl From<cratebase_filter::FilterError> for ApiError {
    fn from(e: cratebase_filter::FilterError) -> Self {
        ApiError(AppError::BadRequest(e.to_string()))
    }
}

impl From<cratebase_storage::StorageError> for ApiError {
    fn from(e: cratebase_storage::StorageError) -> Self {
        ApiError(match e {
            cratebase_storage::StorageError::NotFound(m) => AppError::NotFound(m),
            other => AppError::Internal(other.to_string()),
        })
    }
}

impl From<cratebase_mailer::MailerError> for ApiError {
    fn from(e: cratebase_mailer::MailerError) -> Self {
        ApiError(AppError::BadRequest(e.to_string()))
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        match e.downcast::<AppError>() {
            Ok(app) => ApiError(app),
            Err(other) => ApiError(AppError::Internal(other.to_string())),
        }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError(AppError::Internal(e.to_string()))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = self.error.body();
        if let Some(data) = self.data {
            body.data = data;
        }
        let status = StatusCode::from_u16(body.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        if status.is_server_error() {
            // The client only ever sees the redacted message, so the real
            // one has to reach the operator here or nowhere.
            tracing::error!(error = %self.error, "request failed");
        }
        let message = body.message.clone();
        let mut response = (status, Json(body)).into_response();
        // The request-log middleware reads this instead of buffering the
        // body, so an error row can carry `data.error`.
        response
            .extensions_mut()
            .insert(crate::middleware::request_log::LoggedError(message));
        response
    }
}

/// `Json<T>` whose rejection is PocketBase-shaped.
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(ApiJson(value)),
            Err(rejection) => Err(json_rejection(rejection)),
        }
    }
}

/// PocketBase reports every malformed payload as a plain 400 with its
/// generic wording; the parser detail goes to the log, not the client.
fn json_rejection(rejection: JsonRejection) -> ApiError {
    tracing::debug!(detail = %rejection, "rejected request body");
    ApiError(AppError::bad_request(
        "Failed to load the submitted data due to invalid formatting.",
    ))
}

/// `Query<T>` whose rejection is PocketBase-shaped.
pub struct ApiQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(value)) => Ok(ApiQuery(value)),
            Err(rejection) => Err(query_rejection(rejection)),
        }
    }
}

fn query_rejection(rejection: QueryRejection) -> ApiError {
    tracing::debug!(detail = %rejection, "rejected query string");
    ApiError(AppError::bad_request(
        "Invalid or missing query parameters.",
    ))
}

/// Router fallback: an unknown path is a PocketBase 404, not axum's empty
/// body.
///
/// The same handler is wired as axum's *method-not-allowed* fallback:
/// PocketBase answers a wrong method on an existing route with 404 rather
/// than 405 (confirmed against v0.40.2), and the SDK's error handling
/// depends on it.
pub async fn not_found_fallback() -> Response {
    ApiError(AppError::not_found("")).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_of(err: ApiError) -> (u16, serde_json::Value) {
        let resp = err.into_response();
        let status = resp.status().as_u16();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn renders_pocketbase_shapes() {
        for (err, status, message) in [
            (
                AppError::bad_request(""),
                400,
                AppError::DEFAULT_BAD_REQUEST,
            ),
            (
                AppError::unauthorized(""),
                401,
                AppError::DEFAULT_UNAUTHORIZED,
            ),
            (AppError::forbidden(""), 403, AppError::DEFAULT_FORBIDDEN),
            (AppError::not_found(""), 404, AppError::DEFAULT_NOT_FOUND),
            (
                AppError::too_many_requests(),
                429,
                AppError::DEFAULT_TOO_MANY,
            ),
            (
                AppError::internal("driver exploded"),
                500,
                AppError::DEFAULT_INTERNAL,
            ),
        ] {
            let (code, body) = body_of(ApiError(err)).await;
            assert_eq!(code, status);
            assert_eq!(body["status"], status);
            assert_eq!(body["message"], message);
            assert_eq!(body["data"], serde_json::json!({}));
        }
    }

    #[tokio::test]
    async fn validation_errors_carry_per_field_codes() {
        let err = AppError::validation(
            "Failed to create record.",
            [(
                "email".to_string(),
                cratebase_core::FieldError::new("validation_required", "Cannot be blank."),
            )],
        );
        let (code, body) = body_of(ApiError(err)).await;
        assert_eq!(code, 400);
        assert_eq!(body["data"]["email"]["code"], "validation_required");
    }
}
