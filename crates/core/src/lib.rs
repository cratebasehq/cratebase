//! Shared domain types for Cratebase: collections, fields and the
//! application error type. Deliberately free of any I/O so it can be
//! depended on by every other crate without pulling in database or HTTP
//! machinery.

pub mod collection;
pub mod error;
pub mod field;

pub use collection::{AuthOptions, Collection, CollectionType};
pub use error::{AppError, AppResult, ErrorBody, FieldError};
pub use field::{Field, FieldOptions, FieldType, RESERVED_FIELD_NAMES};

use uuid::Uuid;

/// Generate a new lowercase, hyphen-free unique id (id-friendly for use as a
/// SQL primary key across both SQLite and Postgres).
pub fn new_id() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Current UTC timestamp formatted as RFC3339 (millisecond precision), used
/// for `created` / `updated` record fields so it sorts and compares
/// correctly as plain text on both backends.
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
