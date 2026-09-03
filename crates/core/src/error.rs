use std::collections::HashMap;

use serde::Serialize;
use thiserror::Error;

/// Top-level application error. Carries enough structure for the HTTP layer
/// to render PocketBase-style JSON error bodies:
/// `{ "code": 400, "message": "...", "data": { field: reason } }`.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("validation failed")]
    Validation(HashMap<String, String>),

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
    pub data: HashMap<String, String>,
}

pub type AppResult<T> = Result<T, AppError>;
