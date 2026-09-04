//! Storage-layer errors and their mapping onto the HTTP-facing
//! [`AppError`]. Anything that reaches a client goes through
//! `From<DbError> for AppError`, which deliberately redacts driver
//! messages (PocketBase's generic "Something went wrong..." wording)
//! while keeping the machine-readable validation codes.

use std::collections::BTreeMap;

use cratebase_core::{codes, AppError, FieldError};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("not found")]
    NotFound,
    /// A unique index or constraint was violated. The payload is the
    /// most specific thing the driver told us: `table.column` (SQLite's
    /// "UNIQUE constraint failed: posts.slug") or the constraint/index
    /// name (Postgres).
    #[error("unique constraint violated: {0}")]
    UniqueViolation(String),
    /// Any other constraint failure (NOT NULL, CHECK, FOREIGN KEY).
    #[error("constraint violated: {0}")]
    Constraint(String),
    #[error("validation failed")]
    Validation(BTreeMap<String, FieldError>),
    #[error("invalid identifier: {0}")]
    InvalidIdentifier(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Postgres(#[from] tokio_postgres::Error),
    #[error("connection pool: {0}")]
    Pool(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Filter(#[from] cratebase_filter::FilterError),
    #[error("{0}")]
    Other(String),
}

impl DbError {
    pub fn other(msg: impl Into<String>) -> Self {
        DbError::Other(msg.into())
    }

    /// A single-field validation error.
    pub fn field(name: impl Into<String>, code: &str, message: impl Into<String>) -> Self {
        let mut m = BTreeMap::new();
        m.insert(name.into(), FieldError::new(code, message));
        DbError::Validation(m)
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, DbError::NotFound)
    }
}

impl From<tokio::task::JoinError> for DbError {
    fn from(e: tokio::task::JoinError) -> Self {
        DbError::Other(format!("blocking task failed: {e}"))
    }
}

/// Best-effort field name for a unique violation, from what the driver
/// reported:
///
/// - SQLite: `posts.slug` or `posts.a, posts.b` → `slug` (first column).
/// - Postgres: the index name. PocketBase names its indexes
///   `idx_<field>_<collectionId>` (`idx_email_pbc_123`), so the segment
///   after `idx_` is the field; otherwise the segment after the last `_`.
pub fn unique_violation_field(detail: &str) -> String {
    let first = detail.split(',').next().unwrap_or(detail).trim();
    if let Some((_, col)) = first.rsplit_once('.') {
        return col.trim_matches('"').to_string();
    }
    let name = first.trim_matches('"').trim_matches('`');
    if let Some(rest) = name.strip_prefix("idx_") {
        if let Some((field, _)) = rest.split_once('_') {
            if !field.is_empty() {
                return field.to_string();
            }
        }
        return rest.to_string();
    }
    match name.rsplit_once('_') {
        Some((_, tail)) if !tail.is_empty() => tail.to_string(),
        _ => name.to_string(),
    }
}

impl From<DbError> for AppError {
    fn from(e: DbError) -> Self {
        match e {
            DbError::NotFound => AppError::not_found(""),
            DbError::UniqueViolation(detail) => {
                let field = unique_violation_field(&detail);
                AppError::validation(
                    AppError::DEFAULT_BAD_REQUEST,
                    [(
                        field,
                        FieldError::new(codes::NOT_UNIQUE, "Value must be unique."),
                    )],
                )
            }
            DbError::Validation(fields) => {
                AppError::validation("Failed to validate the submitted data.", fields)
            }
            DbError::Constraint(_) => AppError::bad_request(""),
            DbError::InvalidIdentifier(msg) => AppError::bad_request(msg),
            DbError::Unsupported(msg) => AppError::bad_request(msg),
            DbError::Filter(e) => AppError::bad_request(e.to_string()),
            DbError::Sqlite(e) => AppError::internal(e.to_string()),
            DbError::Postgres(e) => AppError::internal(e.to_string()),
            DbError::Pool(msg) => AppError::internal(msg),
            DbError::Io(e) => AppError::internal(e.to_string()),
            DbError::Json(e) => AppError::internal(e.to_string()),
            DbError::Other(msg) => AppError::internal(msg),
        }
    }
}

pub type DbResult<T> = Result<T, DbError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_field_from_sqlite_and_postgres_details() {
        assert_eq!(unique_violation_field("posts.slug"), "slug");
        assert_eq!(unique_violation_field("posts.a, posts.b"), "a");
        assert_eq!(unique_violation_field("idx_email_pbc_3142635823"), "email");
        assert_eq!(
            unique_violation_field("idx_tokenKey__pb_users_auth_"),
            "tokenKey"
        );
        assert_eq!(unique_violation_field("_collections.name"), "name");
        assert_eq!(unique_violation_field("some_custom_slug"), "slug");
    }

    #[test]
    fn unique_violation_maps_to_validation_not_unique() {
        let err: AppError = DbError::UniqueViolation("posts.slug".into()).into();
        let body = err.body();
        assert_eq!(body.status, 400);
        assert_eq!(body.data["slug"]["code"], codes::NOT_UNIQUE);
    }

    #[test]
    fn internal_errors_are_redacted() {
        let err: AppError = DbError::Other("boom".into()).into();
        assert_eq!(err.status(), 500);
        assert_eq!(err.body().message, AppError::DEFAULT_INTERNAL);
    }
}
