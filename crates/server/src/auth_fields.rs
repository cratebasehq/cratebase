use std::collections::HashMap;

use cratebase_core::{AppError, Collection};
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

/// Extract and hash the `password`/`passwordConfirm` fields of a create
/// request for an `Auth`-typed collection, leaving every other field
/// untouched for the normal schema-driven validation path. No-op for
/// non-auth collections.
pub fn prepare_auth_create(collection: &Collection, mut fields: Map<String, Value>) -> Result<Map<String, Value>, ApiError> {
    if !collection.is_auth() {
        return Ok(fields);
    }
    let mut errors = HashMap::new();

    let email = fields.get("email").and_then(Value::as_str).map(str::to_string);
    match &email {
        Some(e) if looks_like_email(e) => {}
        Some(_) => {
            errors.insert("email".to_string(), "not a valid email address".to_string());
        }
        None => {
            errors.insert("email".to_string(), "value is required".to_string());
        }
    }

    let password = fields.get("password").and_then(Value::as_str).map(str::to_string);
    let confirm = fields.get("passwordConfirm").and_then(Value::as_str).map(str::to_string);
    let min_len = min_password_length(collection);
    match &password {
        None => {
            errors.insert("password".to_string(), "value is required".to_string());
        }
        Some(p) if p.chars().count() < min_len => {
            errors.insert("password".to_string(), format!("must be at least {min_len} characters"));
        }
        Some(p) if confirm.as_deref() != Some(p.as_str()) => {
            errors.insert("passwordConfirm".to_string(), "passwords do not match".to_string());
        }
        Some(_) => {}
    }

    if !errors.is_empty() {
        return Err(ApiError(AppError::Validation(errors)));
    }

    let password_hash =
        cratebase_auth::hash_password(&password.unwrap()).map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    fields.remove("password");
    fields.remove("passwordConfirm");
    fields.insert("email".to_string(), Value::String(email.unwrap()));
    fields.insert("password_hash".to_string(), Value::String(password_hash));
    Ok(fields)
}

/// Same as [`prepare_auth_create`] but every field is optional, matching
/// PATCH semantics: omit `password` to leave it unchanged, omit `email` to
/// leave it unchanged.
pub fn prepare_auth_update(collection: &Collection, mut fields: Map<String, Value>) -> Result<Map<String, Value>, ApiError> {
    if !collection.is_auth() {
        return Ok(fields);
    }
    let mut errors = HashMap::new();

    if let Some(email) = fields.get("email").and_then(Value::as_str) {
        if !looks_like_email(email) {
            errors.insert("email".to_string(), "not a valid email address".to_string());
        }
    }

    let password = fields.get("password").and_then(Value::as_str).map(str::to_string);
    if let Some(password) = &password {
        let confirm = fields.get("passwordConfirm").and_then(Value::as_str);
        let min_len = min_password_length(collection);
        if password.chars().count() < min_len {
            errors.insert("password".to_string(), format!("must be at least {min_len} characters"));
        } else if confirm != Some(password.as_str()) {
            errors.insert("passwordConfirm".to_string(), "passwords do not match".to_string());
        }
    }

    if !errors.is_empty() {
        return Err(ApiError(AppError::Validation(errors)));
    }

    if let Some(password) = password {
        let hash = cratebase_auth::hash_password(&password).map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
        fields.insert("password_hash".to_string(), Value::String(hash));
    }
    fields.remove("password");
    fields.remove("passwordConfirm");
    Ok(fields)
}
