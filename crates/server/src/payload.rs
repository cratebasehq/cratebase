use axum::body::Body;
use axum::extract::{FromRequest, Multipart, Request};
use bytes::Bytes;
use cratebase_core::field::FieldType;
use cratebase_core::{AppError, Collection};
use serde_json::{Map, Value};

use crate::http_error::ApiError;
use crate::state::AppState;

/// A file part submitted alongside a create/update record request.
pub struct Upload {
    pub field: String,
    pub filename: String,
    pub bytes: Bytes,
}

pub struct ParsedPayload {
    pub fields: Map<String, Value>,
    pub uploads: Vec<Upload>,
}

/// Parse a record create/update body. Collections with no `file` fields
/// typically send plain JSON; collections with file fields send
/// `multipart/form-data` (exactly how every PocketBase SDK — and by
/// extension anything already written against it — behaves), with
/// non-file fields submitted as individual form values. Both are accepted
/// unconditionally based on `Content-Type` so either style always works.
pub async fn parse_payload(
    collection: &Collection,
    app: &AppState,
    req: Request<Body>,
) -> Result<ParsedPayload, ApiError> {
    let content_type = req
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if content_type.starts_with("multipart/form-data") {
        let multipart = Multipart::from_request(req, app)
            .await
            .map_err(|e| ApiError(AppError::BadRequest(e.to_string())))?;
        parse_multipart(collection, multipart).await
    } else {
        let bytes = axum::body::to_bytes(req.into_body(), 20 * 1024 * 1024)
            .await
            .map_err(|e| ApiError(AppError::BadRequest(e.to_string())))?;
        let fields = if bytes.is_empty() {
            Map::new()
        } else {
            serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|v| v.as_object().cloned())
                .ok_or_else(|| ApiError(AppError::BadRequest("body must be a JSON object".into())))?
        };
        Ok(ParsedPayload {
            fields,
            uploads: Vec::new(),
        })
    }
}

async fn parse_multipart(collection: &Collection, mut multipart: Multipart) -> Result<ParsedPayload, ApiError> {
    let mut fields = Map::new();
    let mut multi_text: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut uploads = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError(AppError::BadRequest(e.to_string())))?
    {
        let Some(name) = field.name().map(str::to_string) else {
            continue;
        };
        if let Some(filename) = field.file_name().map(str::to_string) {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError(AppError::BadRequest(e.to_string())))?;
            uploads.push(Upload {
                field: name,
                filename,
                bytes,
            });
            continue;
        }

        let text = field
            .text()
            .await
            .map_err(|e| ApiError(AppError::BadRequest(e.to_string())))?;

        let schema_field = collection.field(&name);
        let multiple = schema_field
            .map(|f| f.field_type.supports_multiple() && f.options.multiple.unwrap_or(false))
            .unwrap_or(false);
        if multiple {
            multi_text.entry(name).or_default().push(text);
            continue;
        }

        let value = match schema_field.map(|f| f.field_type) {
            Some(FieldType::Number) => text
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            Some(FieldType::Bool) => Value::Bool(text == "true" || text == "1"),
            Some(FieldType::Json) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
            _ if text.is_empty() => Value::Null,
            _ => Value::String(text),
        };
        fields.insert(name, value);
    }

    for (name, values) in multi_text {
        fields.insert(name, Value::Array(values.into_iter().map(Value::String).collect()));
    }

    Ok(ParsedPayload { fields, uploads })
}
