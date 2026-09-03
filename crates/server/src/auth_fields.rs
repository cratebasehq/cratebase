use std::collections::HashMap;

use cratebase_core::{AppError, Collection, FieldError};
use serde_json::{Map, Value};

use crate::http_error::ApiError;

fn looks_like_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

fn min_password_length(collection: &Collection) -> usize {
    collection.auth_options.min_password_length.unwrap_or(8) as usize
}

/// Extract and hash the identity/`password` fields of a create request for
/// an `Auth`-typed collection, leaving every other field untouched for the
/// normal schema-driven validation path. No-op for non-auth collections.
pub async fn prepare_auth_create(
    collection: &Collection,
    mut fields: Map<String, Value>,
) -> Result<Map<String, Value>, ApiError> {
    if !collection.is_auth() {
        return Ok(fields);
    }
    let mut errors = HashMap::new();
    let identity_field = collection.auth_options.identity_field();

    let identity = fields
        .get(identity_field)
        .and_then(Value::as_str)
        .map(str::to_string);
    match &identity {
        Some(v) if collection.auth_options.identity_is_email() && !looks_like_email(v) => {
            errors.insert(
                identity_field.to_string(),
                FieldError::new("invalid_email", "not a valid email address"),
            );
        }
        Some(v) if v.trim().is_empty() => {
            errors.insert(
                identity_field.to_string(),
                FieldError::new("value_required", "value is required"),
            );
        }
        Some(_) => {}
        None => {
            errors.insert(
                identity_field.to_string(),
                FieldError::new("value_required", "value is required"),
            );
        }
    }

    let password = fields
        .get("password")
        .and_then(Value::as_str)
        .map(str::to_string);
    // `passwordConfirm` is optional: when present it must match, but admins
    // creating a record directly (unlike a public self-registration form)
    // don't need to retype the password.
    let confirm = fields
        .get("passwordConfirm")
        .and_then(Value::as_str)
        .map(str::to_string);
    let min_len = min_password_length(collection);
    match &password {
        None => {
            errors.insert(
                "password".to_string(),
                FieldError::new("value_required", "value is required"),
            );
        }
        Some(p) if p.chars().count() < min_len => {
            errors.insert(
                "password".to_string(),
                FieldError::new(
                    "value_too_short",
                    format!("must be at least {min_len} characters"),
                ),
            );
        }
        Some(p) if confirm.is_some() && confirm.as_deref() != Some(p.as_str()) => {
            errors.insert(
                "passwordConfirm".to_string(),
                FieldError::new("password_mismatch", "passwords do not match"),
            );
        }
        Some(_) => {}
    }

    if !errors.is_empty() {
        return Err(ApiError(AppError::Validation(errors)));
    }

    let password_hash = cratebase_auth::hash_password_async(&password.unwrap())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    fields.remove("password");
    fields.remove("passwordConfirm");
    fields.insert(identity_field.to_string(), Value::String(identity.unwrap()));
    fields.insert("password_hash".to_string(), Value::String(password_hash));
    Ok(fields)
}

/// Same as [`prepare_auth_create`] but every field is optional, matching
/// PATCH semantics: omit `password` to leave it unchanged, omit the
/// identity field to leave it unchanged.
pub async fn prepare_auth_update(
    collection: &Collection,
    mut fields: Map<String, Value>,
) -> Result<Map<String, Value>, ApiError> {
    if !collection.is_auth() {
        return Ok(fields);
    }
    let mut errors = HashMap::new();
    let identity_field = collection.auth_options.identity_field();

    if let Some(value) = fields.get(identity_field).and_then(Value::as_str) {
        if collection.auth_options.identity_is_email() && !looks_like_email(value) {
            errors.insert(
                identity_field.to_string(),
                FieldError::new("invalid_email", "not a valid email address"),
            );
        } else if value.trim().is_empty() {
            errors.insert(
                identity_field.to_string(),
                FieldError::new("value_required", "value is required"),
            );
        }
    }

    let password = fields
        .get("password")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(password) = &password {
        let confirm = fields.get("passwordConfirm").and_then(Value::as_str);
        let min_len = min_password_length(collection);
        if password.chars().count() < min_len {
            errors.insert(
                "password".to_string(),
                FieldError::new(
                    "value_too_short",
                    format!("must be at least {min_len} characters"),
                ),
            );
        } else if confirm.is_some() && confirm != Some(password.as_str()) {
            errors.insert(
                "passwordConfirm".to_string(),
                FieldError::new("password_mismatch", "passwords do not match"),
            );
        }
    }

    if !errors.is_empty() {
        return Err(ApiError(AppError::Validation(errors)));
    }

    if let Some(password) = password {
        let hash = cratebase_auth::hash_password_async(&password)
            .await
            .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
        fields.insert("password_hash".to_string(), Value::String(hash));
    }
    fields.remove("password");
    fields.remove("passwordConfirm");
    Ok(fields)
}
