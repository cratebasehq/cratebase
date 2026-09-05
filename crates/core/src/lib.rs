//! Shared domain types for Cratebase, shaped to be byte-compatible with
//! PocketBase v0.23+'s JSON API: collections, fields, records, settings,
//! errors, ids and dates. Deliberately free of any I/O so every other
//! crate can depend on it without pulling in database or HTTP machinery.
//!
//! The compatibility contract is the set of captured PocketBase responses
//! in `docs/superpowers/specs/pb-fixtures/`; when a shape here and a
//! fixture disagree, the fixture wins.

pub mod collection;
pub mod datetime;
pub mod error;
pub mod event;
pub mod field;
pub mod ids;
pub mod record;
pub mod settings;

pub use collection::{
    AuthAlert, Collection, CollectionType, EmailTemplate, Mfa, OAuth2, OAuth2MappedFields,
    OAuth2Provider, Otp, PasswordAuth, TokenConfig,
};
pub use datetime::DateTime;
pub use error::{codes, AppError, AppResult, ErrorBody, FieldError};
pub use event::{RecordAction, RecordChanged};
pub use field::{Field, FieldKind, FieldType};
pub use ids::{collection_id, field_id, record_id};
pub use record::{Record, SerializeOptions};
pub use settings::Settings;

/// Field/collection names reserved by the system and unavailable to
/// users as custom field names.
pub const RESERVED_FIELD_NAMES: &[&str] = &["collectionId", "collectionName", "expand"];

/// System-managed field names on every auth collection.
pub const AUTH_SYSTEM_FIELDS: &[&str] = &[
    "password",
    "tokenKey",
    "email",
    "emailVisibility",
    "verified",
];

/// Name of the built-in superusers auth collection.
pub const SUPERUSERS_COLLECTION: &str = "_superusers";
/// PocketBase's literal id for the built-in `users` collection.
pub const USERS_COLLECTION_ID: &str = "_pb_users_auth_";

/// `_superusers.role` values. `Owner` is the only role that may create
/// or delete another superuser account, or change anyone's role
/// (including its own) — see `crates/server/src/extract.rs`'s
/// `RequireOwner` extractor and `crates/server/src/routes/records.rs`'s
/// `_superusers`-specific write guards. Kept as two flat string
/// constants rather than a Rust enum so the value round-trips through
/// the same `Select` field machinery (and JSON wire shape) as any other
/// collection's dropdown field.
pub const SUPERUSER_ROLE_OWNER: &str = "owner";
pub const SUPERUSER_ROLE_ADMIN: &str = "admin";

/// Whether `name` is a valid identifier for a collection or field:
/// ASCII letters, digits and underscores, not starting with a digit, at
/// most 64 chars.
pub fn is_valid_identifier(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    let first = name.as_bytes()[0];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// Current time; kept as a function so tests can compare against it.
pub fn now() -> DateTime {
    DateTime::now()
}
