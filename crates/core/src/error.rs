//! Application errors rendered as PocketBase's
//! `{"status": 400, "message": "...", "data": {field: {code, message}}}`.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

/// A single field's validation failure: a stable machine-readable `code`
/// (PocketBase's `validation_*` vocabulary, see [`codes`]) and a
/// human-readable `message`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FieldError {
    pub code: String,
    pub message: String,
    /// Machine-readable detail behind the message, so a client can render
    /// its own wording. PocketBase emits this for the comparison
    /// constraints — a date bound carries
    /// `{"threshold": "2020-01-01T00:00:00Z"}` — and omits the key
    /// entirely otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Map<String, Value>>,
}

impl FieldError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            params: None,
        }
    }

    /// Attach a single `params` entry, e.g. `("threshold", json!("..."))`.
    pub fn with_param(mut self, key: &str, value: Value) -> Self {
        self.params
            .get_or_insert_with(serde_json::Map::new)
            .insert(key.to_string(), value);
        self
    }
}

/// PocketBase's validation error codes, so SDK users branching on `code`
/// get the values they expect.
pub mod codes {
    pub const REQUIRED: &str = "validation_required";
    pub const INVALID_EMAIL: &str = "validation_is_email";
    pub const INVALID_URL: &str = "validation_is_url";
    pub const INVALID_DATE: &str = "validation_invalid_date";
    pub const INVALID_NUMBER: &str = "validation_invalid_number";
    pub const INVALID_BOOL: &str = "validation_invalid_bool";
    pub const INVALID_JSON: &str = "validation_invalid_json";
    pub const INVALID_FORMAT: &str = "validation_invalid_format";
    pub const NOT_UNIQUE: &str = "validation_not_unique";
    pub const MIN_TEXT: &str = "validation_min_text_constraint";
    pub const MAX_TEXT: &str = "validation_max_text_constraint";
    pub const MIN_NUMBER: &str = "validation_min_number_constraint";
    pub const MAX_NUMBER: &str = "validation_max_number_constraint";
    pub const MIN_DATE: &str = "validation_min_date_constraint";
    pub const MAX_DATE: &str = "validation_max_date_constraint";
    pub const MIN_SELECT: &str = "validation_not_enough_values";
    pub const MAX_SELECT: &str = "validation_too_many_values";
    pub const NOT_IN_LIST: &str = "validation_invalid_value";
    pub const MISSING_REL: &str = "validation_missing_rel_records";
    pub const REL_COLLECTION: &str = "validation_invalid_relation_collection";
    pub const FILE_TOO_LARGE: &str = "validation_file_size_limit";
    pub const FILE_MIME: &str = "validation_invalid_mime_type";
    pub const VALUES_MISMATCH: &str = "validation_values_mismatch";
    pub const INVALID_LENGTH: &str = "validation_length_invalid";
    pub const ONLY_INT: &str = "validation_only_int_constraint";
    pub const INVALID_NAME: &str = "validation_invalid_name";
    pub const COLLECTION_NAME_EXISTS: &str = "validation_collection_name_exists";
    pub const INVALID_RULE: &str = "validation_invalid_rule";
    pub const INVALID_INDEX: &str = "validation_invalid_index_expression";
    pub const INVALID_PATTERN: &str = "validation_invalid_regex_pattern";
    pub const PATTERN_MISMATCH: &str = "validation_invalid_format";
    pub const NOT_ALLOWED_DOMAIN: &str = "validation_email_domain_not_allowed";
    pub const INVALID_TOKEN: &str = "validation_invalid_token";
    pub const INVALID_CREDENTIALS: &str = "validation_invalid_credentials";
    pub const INVALID_OTP: &str = "validation_invalid_otp";
    pub const INVALID_MFA: &str = "validation_invalid_mfa";
    pub const OLD_PASSWORD: &str = "validation_invalid_old_password";
    pub const INVALID_VIEW_QUERY: &str = "validation_invalid_view_query";
}

/// Top-level application error.
#[derive(Debug, Error, Clone)]
pub enum AppError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{message}")]
    Validation {
        message: String,
        fields: BTreeMap<String, FieldError>,
    },
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    TooManyRequests(String),
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    /// PocketBase's default messages, used when a caller has nothing more
    /// specific to say.
    pub const DEFAULT_BAD_REQUEST: &'static str =
        "Something went wrong while processing your request.";
    pub const DEFAULT_NOT_FOUND: &'static str = "The requested resource wasn't found.";
    pub const DEFAULT_UNAUTHORIZED: &'static str =
        "The request requires valid record authorization token.";
    pub const DEFAULT_FORBIDDEN: &'static str = "You are not allowed to perform this request.";
    pub const DEFAULT_TOO_MANY: &'static str = "Too Many Requests.";
    pub const DEFAULT_INTERNAL: &'static str =
        "Something went wrong while processing your request.";

    pub fn not_found(msg: impl Into<String>) -> Self {
        let m: String = msg.into();
        AppError::NotFound(if m.is_empty() {
            Self::DEFAULT_NOT_FOUND.into()
        } else {
            m
        })
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        let m: String = msg.into();
        AppError::BadRequest(if m.is_empty() {
            Self::DEFAULT_BAD_REQUEST.into()
        } else {
            m
        })
    }
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        let m: String = msg.into();
        AppError::Unauthorized(if m.is_empty() {
            Self::DEFAULT_UNAUTHORIZED.into()
        } else {
            m
        })
    }
    pub fn forbidden(msg: impl Into<String>) -> Self {
        let m: String = msg.into();
        AppError::Forbidden(if m.is_empty() {
            Self::DEFAULT_FORBIDDEN.into()
        } else {
            m
        })
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        AppError::Internal(msg.into())
    }
    pub fn too_many_requests() -> Self {
        AppError::TooManyRequests(Self::DEFAULT_TOO_MANY.into())
    }

    /// A validation error with PocketBase's per-operation message
    /// (`"Failed to create record."`, ...) and per-field details.
    pub fn validation(
        message: impl Into<String>,
        fields: impl IntoIterator<Item = (String, FieldError)>,
    ) -> Self {
        AppError::Validation {
            message: message.into(),
            fields: fields.into_iter().collect(),
        }
    }

    pub fn status(&self) -> u16 {
        match self {
            AppError::NotFound(_) => 404,
            AppError::BadRequest(_) | AppError::Validation { .. } => 400,
            AppError::Unauthorized(_) => 401,
            AppError::Forbidden(_) => 403,
            AppError::TooManyRequests(_) => 429,
            AppError::Internal(_) => 500,
        }
    }

    pub fn body(&self) -> ErrorBody {
        let (message, data) = match self {
            AppError::Validation { message, fields } => (
                message.clone(),
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or(Value::Null)))
                    .collect(),
            ),
            // Internal errors never leak their detail to clients.
            AppError::Internal(_) => (Self::DEFAULT_INTERNAL.to_string(), BTreeMap::new()),
            other => (other.to_string(), BTreeMap::new()),
        };
        ErrorBody {
            status: self.status(),
            message,
            data,
        }
    }

    pub fn is_validation(&self) -> bool {
        matches!(self, AppError::Validation { .. })
    }
}

/// The JSON error envelope. `data` is a map of field name → `{code,
/// message}` for validation errors and empty otherwise.
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub status: u16,
    pub message: String,
    pub data: BTreeMap<String, Value>,
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_body_matches_pocketbase_shape() {
        let err = AppError::validation(
            "Failed to create record.",
            [(
                "passwordConfirm".to_string(),
                FieldError::new(codes::VALUES_MISMATCH, "Values don't match."),
            )],
        );
        let body = serde_json::to_value(err.body()).unwrap();
        assert_eq!(body["status"], 400);
        assert_eq!(body["message"], "Failed to create record.");
        assert_eq!(
            body["data"]["passwordConfirm"]["code"],
            "validation_values_mismatch"
        );
    }

    #[test]
    fn internal_errors_are_redacted() {
        let body = AppError::internal("db exploded").body();
        assert_eq!(body.status, 500);
        assert_eq!(body.message, AppError::DEFAULT_INTERNAL);
    }
}
