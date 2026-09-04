//! `/api/storage/objects` — superuser only. A bucket/file manager over the
//! record-file object store (`App::storage()`), independent of any record.
//!
//! # Why this exists next to `routes::files`
//!
//! Every other file endpoint is reached *through* a record: a filename is
//! meaningless without the collection and record that own it, and deleting
//! a record's file also clears the record's field. This module is the
//! escape hatch for the operator, not the API consumer — it lets a
//! superuser see and fix what is actually sitting in the bucket, which
//! matters when storage and the database have drifted (a failed upload, a
//! migration between local disk and S3, orphaned files left behind by a
//! bug). Deleting through here is therefore explicitly documented as
//! **not** touching any record — see the dashboard's confirmation dialog.
//!
//! # Prefix-based "folders"
//!
//! Object keys are flat strings; there is no real directory tree, on S3 or
//! on local disk (`object_store`'s local driver just mirrors the key as a
//! path). [`list`] fakes one the way any S3 console does: given a prefix,
//! it groups every key under it by the next `/`, returning that segment as
//! a "folder" and everything with no further `/` as a file at this level.
//! It is one level of `list`, not a recursive walk — a bucket with a
//! million objects under a shallow prefix stays cheap to browse.

use std::collections::BTreeSet;

use axum::body::Body;
use axum::extract::{Multipart, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use cratebase_core::DateTime;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiQuery, ApiResult};

pub fn router() -> Router<App> {
    Router::new()
        .route("/storage/objects", get(list).post(upload).delete(delete))
        .route("/storage/objects/download", get(download))
}

/// A key must not be empty, must not carry a `..` path-traversal segment,
/// and must not start with `/` — the same shape every driver already
/// treats as a single flat string, just rejected up front with a normal
/// field error instead of surfacing as a confusing storage-layer failure.
fn validate_key(key: &str) -> Result<(), ApiError> {
    let trimmed = key.trim_matches('/');
    let valid = !trimmed.is_empty()
        && !trimmed
            .split('/')
            .any(|segment| segment.is_empty() || segment == "..");
    if valid {
        Ok(())
    } else {
        Err(ApiError::bad_request("Missing or invalid object key."))
    }
}

// ------------------------------------------------------------------- list

#[derive(Debug, Default, Deserialize)]
struct ListQuery {
    #[serde(default)]
    prefix: String,
}

#[derive(Debug, Clone, Serialize)]
struct FileEntry {
    key: String,
    size: u64,
    #[serde(rename = "lastModified")]
    last_modified: DateTime,
}

#[derive(Debug, Serialize)]
struct ListResponse {
    /// Full prefixes (each ending in `/`) a caller can pass straight back
    /// as the next `prefix` to descend into.
    folders: Vec<String>,
    files: Vec<FileEntry>,
}

async fn list(
    State(app): State<App>,
    _su: RequireSuperuser,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> ApiResult<Json<ListResponse>> {
    let prefix = query.prefix.trim().trim_matches('/');
    if !prefix.is_empty() && prefix.split('/').any(|segment| segment == "..") {
        return Err(ApiError::bad_request("Missing or invalid prefix."));
    }
    let search_prefix = if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}/")
    };

    let storage = app.storage();
    let objects = storage.list(prefix).await?;

    let mut folders = BTreeSet::new();
    let mut files = Vec::new();
    for object in objects {
        // Defensive: a driver whose prefix match is a raw string compare
        // (rather than path-component aware) could otherwise fold a
        // sibling like `a/bc.txt` into a `prefix = "a/b"` listing.
        let Some(rest) = object.key.strip_prefix(&search_prefix) else {
            continue;
        };
        if rest.is_empty() {
            // The prefix itself exists as an object (a folder marker or a
            // file that happens to share the folder's name) — not
            // representable as a leaf here, so it's skipped rather than
            // shown as a nameless entry.
            continue;
        }
        match rest.split_once('/') {
            Some((folder, _)) => {
                folders.insert(format!("{search_prefix}{folder}/"));
            }
            None => files.push(FileEntry {
                key: object.key,
                size: object.size,
                last_modified: DateTime::from_utc(object.last_modified),
            }),
        }
    }

    Ok(Json(ListResponse {
        folders: folders.into_iter().collect(),
        files,
    }))
}

// --------------------------------------------------------------- download

#[derive(Debug, Default, Deserialize)]
struct KeyQuery {
    #[serde(default)]
    key: String,
}

/// `GET /api/storage/objects/download?key=...`. The key is a query
/// parameter rather than a path segment: keys routinely contain `/`, and
/// axum's path matching has no ambiguity-free way to capture an arbitrary
/// number of segments as one extractor value here.
async fn download(
    State(app): State<App>,
    _su: RequireSuperuser,
    ApiQuery(query): ApiQuery<KeyQuery>,
) -> ApiResult<Response> {
    validate_key(&query.key)?;
    let key = query.key.trim_matches('/');

    let storage = app.storage();
    let stream = storage
        .get_stream(key)
        .await
        .map_err(|_| ApiError::not_found(""))?;
    let body = Body::from_stream(stream.map_err(std::io::Error::other));

    let filename = key.rsplit('/').next().unwrap_or(key);
    let mime = mime_guess::from_path(filename)
        .first_or_octet_stream()
        .essence_str()
        .to_string();

    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

// ----------------------------------------------------------------- upload

/// `POST /api/storage/objects`, multipart: a `key` field with the
/// destination key and a `file` field with the bytes. Overwrites whatever
/// is already at `key`, same as [`cratebase_storage::Storage::put`].
async fn upload(
    State(app): State<App>,
    _su: RequireSuperuser,
    mut multipart: Multipart,
) -> ApiResult<StatusCode> {
    let mut key = String::new();
    let mut bytes: Option<bytes::Bytes> = None;
    while let Some(field) = multipart.next_field().await.map_err(|_| {
        ApiError::bad_request("Failed to load the submitted data due to invalid formatting.")
    })? {
        match field.name().unwrap_or_default() {
            "key" => {
                key = field.text().await.unwrap_or_default();
            }
            "file" => {
                if key.trim().is_empty() {
                    key = field.file_name().unwrap_or_default().to_string();
                }
                bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::bad_request(e.to_string()))?,
                );
            }
            _ => {}
        }
    }

    let Some(bytes) = bytes else {
        return Err(ApiError::bad_request("Missing or invalid file."));
    };
    validate_key(&key)?;
    let key = key.trim_matches('/');

    let storage = app.storage();
    storage.put(key, bytes).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----------------------------------------------------------------- delete

async fn delete(
    State(app): State<App>,
    _su: RequireSuperuser,
    ApiQuery(query): ApiQuery<KeyQuery>,
) -> ApiResult<StatusCode> {
    validate_key(&query.key)?;
    let key = query.key.trim_matches('/');

    let storage = app.storage();
    if storage.size(key).await?.is_none() {
        return Err(ApiError::not_found(""));
    }
    storage.delete(key).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_key_rejects_traversal_and_empty() {
        assert!(validate_key("a/b.txt").is_ok());
        assert!(validate_key("").is_err());
        assert!(validate_key("/").is_err());
        assert!(validate_key("a/../b").is_err());
        assert!(validate_key("a//b").is_err());
    }
}
