use std::collections::HashMap;

use serde::Serialize;
use thiserror::Error;

/// A single field's validation failure: a stable machine-readable `code`
/// (e.g. `"value_too_short"`) alongside a human-readable `message`. Callers
/// that only render text can ignore `code`; callers that branch on the
/// failure kind (an SDK, a form library) don't have to string-match
/// `message`.
#[derive(Debug, Clone, Serialize)]
pub struct FieldError {
    pub code: String,
    pub message: String,
}

impl FieldError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Top-level application error. Carries enough structure for the HTTP layer
/// to render structured JSON error bodies:
/// `{ "code": 400, "message": "...", "data": { field: { code, message } } }`.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("validation failed")]
    Validation(HashMap<String, FieldError>),
    #[error("unauthorized")]
    Unauthorized(String),

    #[error("forbidden")]
    Forbidden(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn status(&self) -> u16 {
        match self {
            AppError::NotFound(_) => 404,
            AppError::BadRequest(_) | AppError::Validation(_) => 400,
            AppError::Unauthorized(_) => 401,
            AppError::Forbidden(_) => 403,
            AppError::Conflict(_) => 409,
            AppError::Internal(_) => 500,
        }
    }

    pub fn body(&self) -> ErrorBody {
        let (message, data) = match self {
            AppError::NotFound(m) => (m.clone(), None),
            AppError::BadRequest(m) => (m.clone(), None),
            AppError::Validation(fields) => ("validation failed".to_string(), Some(fields.clone())),
            AppError::Unauthorized(m) => (m.clone(), None),
            AppError::Forbidden(m) => (m.clone(), None),
            AppError::Conflict(m) => (m.clone(), None),
            AppError::Internal(m) => (m.clone(), None),
        };
        ErrorBody {
            code: self.status(),
            message,
            data: data.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: u16,
    pub message: String,
    pub data: HashMap<String, FieldError>,
}

pub type AppResult<T> = Result<T, AppError>;
