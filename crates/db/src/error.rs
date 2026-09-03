use std::collections::HashMap;

use cratebase_core::AppError;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("record not found")]
    NotFound,
    #[error("unique constraint violated on field '{0}'")]
    UniqueViolation(String),
    #[error("invalid identifier: {0}")]
    InvalidIdentifier(String),
    #[error("validation failed")]
    Validation(HashMap<String, String>),
    #[error(transparent)]
    Filter(#[from] cratebase_filter::FilterError),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

impl From<DbError> for AppError {
    fn from(e: DbError) -> Self {
        match e {
            DbError::NotFound => AppError::NotFound("record not found".into()),
            DbError::UniqueViolation(field) => {
                AppError::Conflict(format!("value for '{field}' must be unique"))
            }
            DbError::InvalidIdentifier(msg) => AppError::BadRequest(msg),
            DbError::Validation(fields) => AppError::Validation(fields),
            DbError::Filter(e) => AppError::BadRequest(e.to_string()),
            DbError::Sqlx(e) => AppError::Internal(e.to_string()),
        }
    }
}

pub type DbResult<T> = Result<T, DbError>;
