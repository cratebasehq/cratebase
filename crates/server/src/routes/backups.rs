//! Full-database backups: SQLite-only (matching PocketBase — a
//! self-contained single-file database has a trivial "back it up" story
//! that a multi-process Postgres cluster doesn't), stored through the same
//! [`cratebase_storage::Storage`] backend as user-uploaded files under a
//! `backups/` key prefix so there's no second storage location to
//! provision or reason about.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use bytes::Bytes;
use cratebase_core::{new_id, now, AppError};
use cratebase_db::{system, Backend};
use serde::{Deserialize, Serialize};

use crate::extract::RequireAdmin;
use crate::http_error::{ApiError, ApiResult};
use crate::state::AppState;

const BACKUP_PREFIX: &str = "backups/";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/backups", get(list).post(create))
        .route("/backups/{name}", axum::routing::delete(remove))
        .route("/backups/{name}/download", get(download))
}

#[derive(Deserialize, Default)]
struct CreateInput {
    /// Optional caller-chosen base name (without the `.db` extension,
    /// though one is tolerated). Defaults to a timestamped name so hitting
    /// the endpoint with no body still produces something sensible.
    name: Option<String>,
}

#[derive(Serialize)]
struct BackupInfo {
    name: String,
    size: u64,
    created: String,
}

/// Backup names double as storage keys and download-URL path segments, so
/// this is deliberately stricter than a general filename: no `/`, no `..`,
/// no leading dot — closes off path traversal into other storage prefixes
/// without needing a second, storage-level check.
fn validate_name(name: &str) -> ApiResult<()> {
    let valid = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(ApiError(AppError::BadRequest(
            "backup name must be alphanumeric (with '-', '_', '.') and not start with a dot".into(),
        )))
    }
}

fn sqlite_only(app: &AppState) -> ApiResult<()> {
    if app.db.backend != Backend::Sqlite {
        return Err(ApiError(AppError::BadRequest(
            "backups are only supported on SQLite; back up a Postgres database with your own tooling (e.g. pg_dump)".into(),
        )));
    }
    Ok(())
}

async fn create(
    State(app): State<AppState>,
    _admin: RequireAdmin,
    Json(input): Json<CreateInput>,
) -> ApiResult<Json<BackupInfo>> {
    sqlite_only(&app)?;

    let name = match input.name {
        Some(n) => {
            validate_name(&n)?;
            if n.ends_with(".db") {
                n
            } else {
                format!("{n}.db")
            }
        }
        // `now()` is RFC3339 with `:`/`.` — neither is a valid backup-name
        // character, so swap them for `-` instead of writing a second
        // timestamp formatter just for this.
        None => format!("backup_{}.db", now().replace([':', '.'], "-")),
    };

    // `VACUUM INTO` is SQLite's own online-backup path: it reads a
    // consistent snapshot through the WAL the same way an ordinary reader
    // transaction would, so a concurrent writer can never produce a
    // torn/corrupt copy the way copying the raw `.db` file could while WAL
    // mode is active.
    let tmp_path = std::env::temp_dir().join(format!("cratebase-backup-{}.db", new_id()));
    let tmp_path_str = tmp_path.to_string_lossy().to_string();
    system::vacuum_into(&app.db, &tmp_path_str)
        .await
        .map_err(|e| ApiError(AppError::Internal(format!("backup failed: {e}"))))?;

    let bytes = tokio::fs::read(&tmp_path)
        .await
        .map_err(|e| ApiError(AppError::Internal(format!("reading backup snapshot: {e}"))))?;
    tokio::fs::remove_file(&tmp_path).await.ok();

    let size = bytes.len() as u64;
    app.storage
        .put(&format!("{BACKUP_PREFIX}{name}"), Bytes::from(bytes))
        .await?;

    Ok(Json(BackupInfo {
        name,
        size,
        created: now(),
    }))
}

async fn list(
    State(app): State<AppState>,
    _admin: RequireAdmin,
) -> ApiResult<Json<Vec<BackupInfo>>> {
    let objects = app.storage.list(BACKUP_PREFIX).await?;
    let mut backups: Vec<BackupInfo> = objects
        .into_iter()
        .map(|o| BackupInfo {
            name: o
                .key
                .strip_prefix(BACKUP_PREFIX)
                .unwrap_or(&o.key)
                .to_string(),
            size: o.size,
            created: o.last_modified.to_rfc3339(),
        })
        .collect();
    backups.sort_by(|a, b| b.created.cmp(&a.created));
    Ok(Json(backups))
}

async fn download(
    State(app): State<AppState>,
    _admin: RequireAdmin,
    Path(name): Path<String>,
) -> ApiResult<Response> {
    validate_name(&name)?;
    let stream = app
        .storage
        .get_stream(&format!("{BACKUP_PREFIX}{name}"))
        .await?;
    let response = (
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        Body::from_stream(stream),
    );
    Ok(response.into_response())
}

async fn remove(
    State(app): State<AppState>,
    _admin: RequireAdmin,
    Path(name): Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    validate_name(&name)?;
    app.storage
        .delete(&format!("{BACKUP_PREFIX}{name}"))
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
