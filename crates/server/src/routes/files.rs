use std::io::Cursor;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use cratebase_auth::{issue_file_token, verify_token, TokenKind};
use cratebase_core::AppError;
use cratebase_db::records;
use cratebase_db::resolver::{
    evaluate_rule, load_related_collections_for_rule, RequestContext, RuleOutcome,
};
use cratebase_db::{admins, collections, AuthContext};
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::extract::CurrentAuth;
use crate::helpers::{file_key, load_collection};
use crate::http_error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/files/{collection}/{record_id}/{filename}", get(download))
        .route("/files/token", post(issue_file_token_route))
}

#[derive(Debug, Deserialize)]
struct DownloadParams {
    /// A PocketBase-style thumb spec: `WxH` (cover, center-cropped),
    /// `WxHf` (fit inside the box, no cropping), or `WxHt`/`WxHb`
    /// (cover, cropped from the top/bottom instead of centered). Ignored
    /// unless the file's mime type is a raster image format we can
    /// decode/encode.
    thumb: Option<String>,
    /// A short-lived `FileToken` minted by `POST /api/files/token`,
    /// letting a context that can't send an `Authorization` header (an
    /// `<img src>`, a shared link) still pass an auth-gated `viewRule`.
    token: Option<String>,
}

/// Streams the file straight from the storage backend to the response body
/// rather than buffering it in memory, so a multi-hundred-MB upload doesn't
/// cost a multi-hundred-MB allocation per concurrent download.
async fn download(
    State(app): State<AppState>,
    Path((collection_name, record_id, filename)): Path<(String, String, String)>,
    Query(params): Query<DownloadParams>,
    CurrentAuth(auth): CurrentAuth,
) -> ApiResult<Response> {
    let collection = load_collection(&app, &collection_name).await?;

    // A `?token=` file token overrides the caller's live session for this
    // request only — it exists precisely so an unauthenticated `<img>` tag
    // can still satisfy an auth-required `viewRule`. An absent or invalid
    // token silently falls back to whatever the `Authorization` header
    // resolved to, same as today.
    let auth = match params.token.as_deref() {
        Some(token) => resolve_file_token(&app, token).await.or(auth),
        None => auth,
    };
    let ctx = RequestContext { auth, data: None };

    // A file is only downloadable if its owning record is currently
    // visible under the collection's viewRule — files piggyback on record
    // access control rather than having their own rule type.
    let related =
        load_related_collections_for_rule(&app.db, &collection, &collection.view_rule).await?;
    let outcome = evaluate_rule(
        &collection.view_rule,
        &collection,
        app.db.backend,
        &ctx,
        0,
        &related,
    )?;
    let rule_filter = match outcome {
        RuleOutcome::DenyAll => {
            return Err(ApiError(AppError::Forbidden(
                "you are not allowed to access this file".into(),
            )))
        }
        RuleOutcome::AllowAll => None,
        RuleOutcome::Filtered(f) => Some(f),
    };
    records::get_record(&app.db, &collection, &record_id, rule_filter).await?;

    let key = file_key(&collection.id, &record_id, &filename);
    let content_type = mime_guess::from_path(&filename).first_or_octet_stream();

    if let (Some(spec), Some(format)) = (
        params.thumb.as_deref().and_then(ThumbSpec::parse),
        image_format_for(content_type.essence_str()),
    ) {
        let thumb_key = thumb_key(&collection.id, &record_id, &filename, &spec.raw);
        return serve_thumbnail(
            &app,
            &thumb_key,
            &key,
            format,
            &spec,
            content_type.essence_str(),
        )
        .await;
    }

    let stream = app.storage.get_stream(&key).await?;
    let response = (
        [(header::CONTENT_TYPE, content_type.essence_str().to_string())],
        Body::from_stream(stream),
    );
    Ok(response.into_response())
}

/// Mints a `FileToken` carrying the current caller's identity. The caller
/// must already be authenticated (admin or auth record) — this endpoint
/// only re-packages an existing session into a short-lived, URL-embeddable
/// form, it never grants access beyond what the caller already has.
async fn issue_file_token_route(
    State(app): State<AppState>,
    CurrentAuth(auth): CurrentAuth,
) -> ApiResult<Json<Value>> {
    let ctx =
        auth.ok_or_else(|| ApiError(AppError::Unauthorized("missing or invalid token".into())))?;
    let token = issue_file_token(
        &ctx.id,
        &ctx.collection_id,
        ctx.is_superuser,
        &app.config.auth_secret,
        app.config.file_token_ttl_seconds,
    )
    .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
    Ok(Json(json!({ "token": token })))
}

/// Resolves a `?token=` query param into the `AuthContext` it carries.
/// Mirrors `CurrentAuth`'s `Admin`/`Auth` resolution but keyed off
/// `TokenKind::FileToken` instead of a `Bearer` header. Any failure
/// (expired, wrong kind, dangling reference to a deleted admin/record)
/// resolves to `None` rather than a hard error — the caller falls back to
/// its normal session, same as an absent token.
async fn resolve_file_token(app: &AppState, token: &str) -> Option<AuthContext> {
    let claims = verify_token(token, &app.config.auth_secret).ok()?;
    if claims.kind != TokenKind::FileToken {
        return None;
    }
    if claims.is_superuser {
        admins::get_admin_by_id(&app.db, &claims.sub).await.ok()?;
        return Some(AuthContext {
            id: claims.sub,
            collection_id: String::new(),
            is_superuser: true,
            record: Default::default(),
        });
    }
    let collection = collections::get_collection_by_id(&app.db, &claims.collection_id)
        .await
        .ok()?;
    let record = records::get_record(&app.db, &collection, &claims.sub, None)
        .await
        .ok()?;
    let record_map = record.as_object().cloned().unwrap_or_default();
    Some(AuthContext {
        id: claims.sub,
        collection_id: claims.collection_id,
        is_superuser: false,
        record: record_map,
    })
}

/// How a thumb spec's `WxH` box is applied to the source image, matching
/// PocketBase's `WxH`/`WxHf`/`WxHt`/`WxHb` suffix convention.
#[derive(Clone, Copy)]
enum ThumbMode {
    /// No suffix: scale to cover the box, cropping the overflow evenly
    /// from both edges (centered).
    Cover,
    /// `f`: scale to fit inside the box, preserving aspect ratio, never
    /// cropping — the result may be smaller than `WxH` in one dimension.
    Fit,
    /// `t`: like `Cover`, but the crop keeps the top edge.
    CropTop,
    /// `b`: like `Cover`, but the crop keeps the bottom edge.
    CropBottom,
}

struct ThumbSpec {
    width: u32,
    height: u32,
    mode: ThumbMode,
    /// The original spec string (e.g. `"100x100f"`), reused verbatim in
    /// the cache key so distinct specs never collide.
    raw: String,
}

impl ThumbSpec {
    fn parse(spec: &str) -> Option<Self> {
        let (dims, mode) = if let Some(d) = spec.strip_suffix('f') {
            (d, ThumbMode::Fit)
        } else if let Some(d) = spec.strip_suffix('t') {
            (d, ThumbMode::CropTop)
        } else if let Some(d) = spec.strip_suffix('b') {
            (d, ThumbMode::CropBottom)
        } else {
            (spec, ThumbMode::Cover)
        };
        let (w, h) = dims.split_once('x')?;
        let width: u32 = w.parse().ok()?;
        let height: u32 = h.parse().ok()?;
        if width == 0 || height == 0 {
            return None;
        }
        Some(Self {
            width,
            height,
            mode,
            raw: spec.to_string(),
        })
    }
}

/// Raster formats the `image` crate can both decode and re-encode. Any
/// other mime (svg, pdf, video, ...) falls back to serving the original
/// file untouched regardless of `?thumb=`.
fn image_format_for(mime_essence: &str) -> Option<ImageFormat> {
    match mime_essence {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" => Some(ImageFormat::Jpeg),
        "image/gif" => Some(ImageFormat::Gif),
        "image/webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

/// Derived cache key for a thumbnail, stored alongside the original under
/// a `thumbs/` prefix so it survives in the same storage backend (local
/// disk or S3) and is trivially found again on the next identical request.
fn thumb_key(collection_id: &str, record_id: &str, filename: &str, spec_raw: &str) -> String {
    format!("{collection_id}/{record_id}/thumbs/{filename}_{spec_raw}")
}

fn render_thumbnail(img: &DynamicImage, spec: &ThumbSpec) -> DynamicImage {
    match spec.mode {
        ThumbMode::Fit => img.resize(spec.width, spec.height, FilterType::Lanczos3),
        ThumbMode::Cover => img.resize_to_fill(spec.width, spec.height, FilterType::Lanczos3),
        ThumbMode::CropTop => resize_crop_anchored(img, spec.width, spec.height, true),
        ThumbMode::CropBottom => resize_crop_anchored(img, spec.width, spec.height, false),
    }
}

/// Scales the image to cover `width`x`height` (same as `resize_to_fill`)
/// but crops the overflow from one edge only, anchoring the opposite edge
/// instead of centering — PocketBase's `t`/`b` thumb suffixes.
fn resize_crop_anchored(
    img: &DynamicImage,
    width: u32,
    height: u32,
    anchor_top: bool,
) -> DynamicImage {
    let (src_w, src_h) = (img.width().max(1) as f64, img.height().max(1) as f64);
    let scale = (width as f64 / src_w).max(height as f64 / src_h);
    let scaled_w = ((src_w * scale).round() as u32).max(width);
    let scaled_h = ((src_h * scale).round() as u32).max(height);
    let scaled = img.resize_exact(scaled_w, scaled_h, FilterType::Lanczos3);
    let x = scaled_w.saturating_sub(width) / 2;
    let y = if anchor_top {
        0
    } else {
        scaled_h.saturating_sub(height)
    };
    scaled.crop_imm(x, y, width, height)
}

/// Serves a thumbnail, generating and caching it to storage on first
/// request; every subsequent request for the same `thumb_key` is a plain
/// storage read with no re-decode/re-encode.
async fn serve_thumbnail(
    app: &AppState,
    thumb_key: &str,
    original_key: &str,
    format: ImageFormat,
    spec: &ThumbSpec,
    content_type: &str,
) -> ApiResult<Response> {
    if !app.storage.exists(thumb_key).await? {
        let original = app.storage.get(original_key).await?;
        let img = image::load_from_memory(&original)
            .map_err(|e| ApiError(AppError::Internal(format!("failed to decode image: {e}"))))?;
        let resized = render_thumbnail(&img, spec);
        let mut buf = Vec::new();
        resized
            .write_to(&mut Cursor::new(&mut buf), format)
            .map_err(|e| {
                ApiError(AppError::Internal(format!(
                    "failed to encode thumbnail: {e}"
                )))
            })?;
        app.storage.put(thumb_key, Bytes::from(buf)).await?;
    }

    let stream = app.storage.get_stream(thumb_key).await?;
    let response = (
        [(header::CONTENT_TYPE, content_type.to_string())],
        Body::from_stream(stream),
    );
    Ok(response.into_response())
}
