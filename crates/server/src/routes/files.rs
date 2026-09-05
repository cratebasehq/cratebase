//! `/api/files` — serving record files, thumbnails and file tokens.
//!
//! # `protected` is not "always needs a token"
//!
//! This is the single most surprising thing in the file API, measured
//! against PocketBase v0.40.2 (KNOWN_DIVERGENCES §29): marking a file
//! field `protected` does **not** make it private. What it does is switch
//! on a `viewRule` check for the *token owner* — and an empty `viewRule`
//! is public, so a protected file in a public collection is served to
//! anyone, with or without a token. Only a restricted `viewRule` actually
//! gates the file, and an unauthorized request is then a `404`, never a
//! `403`, so the URL cannot confirm that the record exists.
//!
//! # Thumbs
//!
//! `?thumb=WxH[t|b|f]`, `?thumb=0xH`, `?thumb=Wx0`. A thumb is generated
//! on first request and cached under
//! `{collectionId}/{recordId}/thumbs_{filename}/{spec}_{filename}`, so
//! the second request is a plain object-store read. A thumb of something
//! that is not an image falls back to the original file with a `200`.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use cratebase_core::{Collection, Field, FieldKind, FieldType, Record};
use cratebase_db::context::{AuthContext, RequestContext};
use cratebase_db::records;
use futures::TryStreamExt;
use serde::Deserialize;
use serde_json::json;

use crate::app::App;
use crate::events::{collection_tags, FileDownloadEvent, FileTokenEvent};
use crate::extract::{Auth, RequestInfo};
use crate::http_error::{ApiError, ApiQuery, ApiResult};
use crate::routes::common;

/// PocketBase's `@request.context` value while a protected file's
/// `viewRule` is evaluated.
const PROTECTED_FILE_CONTEXT: &str = "protectedFile";

pub fn router() -> Router<App> {
    Router::new()
        .route("/files/token", post(token))
        .route("/files/{collection}/{recordId}/{filename}", get(download))
}

/// `POST /api/files/token`. Any authenticated record may mint one; what
/// it unlocks is decided per file by the owning collection's `viewRule`.
async fn token(State(app): State<App>, auth: Auth) -> ApiResult<Json<serde_json::Value>> {
    let config = &auth.collection.auth.file_token;
    let claims =
        cratebase_auth::new_file_claims(&auth.id, &auth.collection_id, config.duration.max(1));
    let token = cratebase_auth::sign(
        &claims,
        &app.token_signing_key(&auth.record.token_key(), &config.secret),
    )
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut event = FileTokenEvent::new(app.clone(), Some(auth), token);
    app.hooks()
        .on_file_token_request
        .trigger_bare(&mut event)
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({ "token": event.token })))
}

/// Resolve a `?token=` file token into the rule-evaluation context of its
/// owner. Anything wrong with the token is simply "no auth", which then
/// fails a restricted `viewRule` and becomes a 404.
pub async fn file_token_context(app: &App, token: &str) -> Option<AuthContext> {
    if token.is_empty() {
        return None;
    }
    let unverified = cratebase_auth::decode_unverified(token).ok()?;
    if unverified.token_type != cratebase_auth::TokenType::File {
        return None;
    }
    let collection = app.db().collections.get_by_id(&unverified.collection_id)?;
    if !collection.is_auth() {
        return None;
    }
    let record = records::find_by_id_raw(app.db(), &collection, &unverified.id)
        .await
        .ok()?;
    let key = app.token_signing_key(&record.token_key(), &collection.auth.file_token.secret);
    cratebase_auth::verify(token, &key).ok()?;
    Some(AuthContext::new(record))
}

#[derive(Debug, Default, Deserialize)]
struct FileQuery {
    #[serde(default)]
    thumb: Option<String>,
    #[serde(default)]
    download: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

async fn download(
    State(app): State<App>,
    Path((collection_ref, record_id, filename)): Path<(String, String, String)>,
    ApiQuery(query): ApiQuery<FileQuery>,
    info: RequestInfo,
) -> ApiResult<Response> {
    let not_found = || ApiError::not_found("");

    let collection = app
        .db()
        .collections
        .get(&collection_ref)
        .ok_or_else(not_found)?;
    let record = records::find_by_id_raw(app.db(), &collection, &record_id)
        .await
        .map_err(|_| not_found())?;
    let field = owning_field(&record, &filename).ok_or_else(not_found)?;

    if is_protected(&field) && !may_view(&app, &collection, &record, &query, &info).await? {
        return Err(not_found());
    }

    let storage = app.storage();
    let key = common::file_key(&collection.id, &record_id, &filename);
    let mime = mime_for(&filename);

    // A thumb request that cannot be honoured (an unparsable spec, a
    // non-image file, a decode failure) falls back to the original with a
    // 200 — that is what PocketBase does.
    let mut served_key = key.clone();
    let mut inline_bytes: Option<Bytes> = None;
    if let Some(spec) = query
        .thumb
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(ThumbSpec::parse)
    {
        if let Some((thumb_key, bytes)) =
            thumb(&storage, &collection.id, &record_id, &filename, &key, spec).await
        {
            served_key = thumb_key;
            inline_bytes = bytes;
        }
    }

    let mut event = FileDownloadEvent::new(
        app.clone(),
        collection.clone(),
        Some(record),
        served_key.clone(),
        filename.clone(),
        collection_tags(&collection),
    );
    app.hooks()
        .on_file_download_request
        .trigger_bare(&mut event)
        .await
        .map_err(ApiError)?;
    let served_key = event.key.clone();
    let served_name = event.served_name.clone();

    let disposition = if is_truthy(query.download.as_deref()) {
        format!("attachment; filename=\"{served_name}\"")
    } else {
        format!("inline; filename=\"{served_name}\"")
    };

    let body = match inline_bytes {
        Some(bytes) => Body::from(bytes),
        None => {
            let stream = storage
                .get_stream(&served_key)
                .await
                .map_err(|_| not_found())?;
            Body::from_stream(stream.map_err(std::io::Error::other))
        }
    };

    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

/// The file field a name belongs to, or `None` when the record does not
/// actually hold that file (which is a 404, not a storage miss).
fn owning_field(record: &Record, filename: &str) -> Option<Field> {
    record
        .collection
        .fields_of_type(FieldType::File)
        .find(|f| {
            record
                .get_string_list(&f.name)
                .iter()
                .any(|name| name == filename)
        })
        .cloned()
}

fn is_protected(field: &Field) -> bool {
    matches!(
        field.kind,
        FieldKind::File {
            protected: true,
            ..
        }
    )
}

/// The `viewRule` gate a protected file goes through, evaluated for the
/// `?token=` owner (or for a guest when there is no token).
async fn may_view(
    app: &App,
    collection: &Arc<Collection>,
    record: &Record,
    query: &FileQuery,
    info: &RequestInfo,
) -> ApiResult<bool> {
    let auth = match query.token.as_deref() {
        Some(token) => file_token_context(app, token).await,
        // A regular `Authorization` header works too; PocketBase accepts
        // either, and the query token is only needed because a browser
        // download cannot set a header.
        None => info.auth.as_ref().map(Auth::to_auth_context),
    };
    let ctx = RequestContext {
        auth,
        context: PROTECTED_FILE_CONTEXT.to_string(),
        ..Default::default()
    };
    common::record_matches_rule(
        app.db(),
        &app.db().collections,
        &ctx,
        collection,
        &collection.view_rule,
        record.id(),
    )
    .await
    .map_err(|e| ApiError(e.into()))
}

fn is_truthy(raw: Option<&str>) -> bool {
    matches!(raw, Some("1" | "true" | "TRUE" | "True"))
}

fn mime_for(filename: &str) -> String {
    mime_guess::from_path(filename)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThumbMode {
    /// `WxH` / `WxHt` / `WxHb`: cover then crop to the anchor.
    Crop(Anchor),
    /// `WxHf`: fit inside the box, no cropping.
    Fit,
    /// `0xH` or `Wx0`: one dimension, aspect preserved.
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Center,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThumbSpec {
    width: u32,
    height: u32,
    mode: ThumbMode,
}

impl ThumbSpec {
    /// `WxH`, `WxHt`, `WxHb`, `WxHf`, `0xH`, `Wx0`. Anything else is not
    /// a thumb request and the original file is served instead.
    fn parse(raw: &str) -> Option<ThumbSpec> {
        let (head, mode) = match raw.chars().last() {
            Some('t') => (&raw[..raw.len() - 1], ThumbMode::Crop(Anchor::Top)),
            Some('b') => (&raw[..raw.len() - 1], ThumbMode::Crop(Anchor::Bottom)),
            Some('f') => (&raw[..raw.len() - 1], ThumbMode::Fit),
            _ => (raw, ThumbMode::Crop(Anchor::Center)),
        };
        let (w, h) = head.split_once('x')?;
        let width: u32 = w.parse().ok()?;
        let height: u32 = h.parse().ok()?;
        if width == 0 && height == 0 {
            return None;
        }
        // A dimension of zero always means "keep the aspect ratio",
        // whatever suffix came with it.
        let mode = if width == 0 || height == 0 {
            ThumbMode::Scale
        } else {
            mode
        };
        Some(ThumbSpec {
            width,
            height,
            mode,
        })
    }

    fn key(&self) -> String {
        let suffix = match self.mode {
            ThumbMode::Crop(Anchor::Top) => "t",
            ThumbMode::Crop(Anchor::Bottom) => "b",
            ThumbMode::Fit => "f",
            _ => "",
        };
        format!("{}x{}{suffix}", self.width, self.height)
    }
}

/// Serve a cached thumb, or generate and cache one. `None` means "not a
/// thumbnailable file" and the caller falls back to the original.
async fn thumb(
    storage: &cratebase_storage::Storage,
    collection_id: &str,
    record_id: &str,
    filename: &str,
    original_key: &str,
    spec: ThumbSpec,
) -> Option<(String, Option<Bytes>)> {
    let key = format!(
        "{}/{}_{filename}",
        common::thumbs_prefix(collection_id, record_id, filename),
        spec.key()
    );
    if storage.exists(&key).await.unwrap_or(false) {
        return Some((key, None));
    }

    let source = storage.get(original_key).await.ok()?;
    let format = image::ImageFormat::from_path(filename).ok()?;
    let generated = tokio::task::spawn_blocking(move || render(&source, format, spec))
        .await
        .ok()??;
    if let Err(e) = storage.put(&key, generated.clone()).await {
        // A cache write failure must not fail the request.
        tracing::warn!(key = %key, error = %e, "failed to cache a thumbnail");
    }
    Some((key, Some(generated)))
}

/// Decode, resize and re-encode. CPU-bound, so it runs on the blocking
/// pool.
fn render(source: &[u8], format: image::ImageFormat, spec: ThumbSpec) -> Option<Bytes> {
    use image::imageops::FilterType;

    let image = image::load_from_memory_with_format(source, format).ok()?;
    let (width, height) = (image.width().max(1), image.height().max(1));

    let resized = match spec.mode {
        ThumbMode::Scale => {
            let (w, h) = if spec.width == 0 {
                let scaled = (width as f64 * spec.height as f64 / height as f64).round();
                (scaled.max(1.0) as u32, spec.height)
            } else {
                let scaled = (height as f64 * spec.width as f64 / width as f64).round();
                (spec.width, scaled.max(1.0) as u32)
            };
            image.resize_exact(w, h, FilterType::Lanczos3)
        }
        ThumbMode::Fit => image.resize(spec.width, spec.height, FilterType::Lanczos3),
        ThumbMode::Crop(anchor) => {
            // Cover the box, then take the slice the anchor asks for.
            let scale = (spec.width as f64 / width as f64).max(spec.height as f64 / height as f64);
            let covered = image.resize_exact(
                ((width as f64 * scale).round() as u32).max(spec.width),
                ((height as f64 * scale).round() as u32).max(spec.height),
                FilterType::Lanczos3,
            );
            let x = covered.width().saturating_sub(spec.width) / 2;
            let y = match anchor {
                Anchor::Top => 0,
                Anchor::Bottom => covered.height().saturating_sub(spec.height),
                Anchor::Center => covered.height().saturating_sub(spec.height) / 2,
            };
            let mut covered = covered;
            image::imageops::crop(&mut covered, x, y, spec.width, spec.height)
                .to_image()
                .into()
        }
    };

    let mut out = std::io::Cursor::new(Vec::new());
    resized.write_to(&mut out, format).ok()?;
    Some(Bytes::from(out.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_specs_cover_every_pocketbase_form() {
        assert_eq!(
            ThumbSpec::parse("100x100"),
            Some(ThumbSpec {
                width: 100,
                height: 100,
                mode: ThumbMode::Crop(Anchor::Center)
            })
        );
        assert_eq!(
            ThumbSpec::parse("50x50t").map(|s| s.mode),
            Some(ThumbMode::Crop(Anchor::Top))
        );
        assert_eq!(
            ThumbSpec::parse("50x50b").map(|s| s.mode),
            Some(ThumbMode::Crop(Anchor::Bottom))
        );
        assert_eq!(
            ThumbSpec::parse("50x50f").map(|s| s.mode),
            Some(ThumbMode::Fit)
        );
        assert_eq!(
            ThumbSpec::parse("0x200").map(|s| s.mode),
            Some(ThumbMode::Scale)
        );
        assert_eq!(
            ThumbSpec::parse("200x0").map(|s| s.mode),
            Some(ThumbMode::Scale)
        );
        assert_eq!(ThumbSpec::parse("0x0"), None);
        assert_eq!(ThumbSpec::parse("nonsense"), None);
        assert_eq!(ThumbSpec::parse("100"), None);
        assert_eq!(ThumbSpec::parse("100x100").unwrap().key(), "100x100");
        assert_eq!(ThumbSpec::parse("50x50t").unwrap().key(), "50x50t");
    }

    #[test]
    fn a_real_png_is_resized_and_re_encoded() {
        let mut source = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(8, 4)
            .write_to(&mut source, image::ImageFormat::Png)
            .unwrap();
        let bytes = source.into_inner();

        let cropped = render(
            &bytes,
            image::ImageFormat::Png,
            ThumbSpec::parse("4x4").unwrap(),
        )
        .unwrap();
        let decoded = image::load_from_memory(&cropped).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (4, 4));

        let scaled = render(
            &bytes,
            image::ImageFormat::Png,
            ThumbSpec::parse("0x2").unwrap(),
        )
        .unwrap();
        let decoded = image::load_from_memory(&scaled).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (4, 2));

        assert!(render(
            b"not an image",
            image::ImageFormat::Png,
            ThumbSpec::parse("4x4").unwrap()
        )
        .is_none());
    }

    #[test]
    fn download_flag_is_explicit() {
        assert!(is_truthy(Some("1")));
        assert!(is_truthy(Some("true")));
        assert!(!is_truthy(Some("0")));
        assert!(!is_truthy(None));
        assert_eq!(mime_for("a.txt"), "text/plain");
        assert_eq!(mime_for("a.png"), "image/png");
    }
}
