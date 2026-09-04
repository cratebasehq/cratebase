//! `/api/backups` — superuser only (download also accepts a superuser
//! file token in the query string, because a browser download cannot set
//! an `Authorization` header).
//!
//! A backup is a ZIP of the data directory: a consistent snapshot of the
//! main database (`Engine::snapshot_to`, i.e. `VACUUM INTO`, not a raw
//! file copy that could catch a half-written page), the same for
//! `auxiliary.db`, and the `storage/` tree when files live on local disk.
//! The `backups/` directory itself is excluded, so a backup never
//! contains its own siblings.
//!
//! # Streaming, not buffering
//!
//! The ZIP is built into a temp file and then *streamed* into the backups
//! object store, and a download is streamed straight out of it. The
//! previous implementation read the whole archive into a `Vec<u8>`, which
//! meant a multi-GB database became a multi-GB allocation per request —
//! flagged by the audit.
//!
//! # Why a restore restarts the process
//!
//! Restoring swaps the files the open SQLite connections are mapped to.
//! There is no safe way to keep serving from those handles, so the
//! engines are closed, the directories swapped, and the current binary
//! `exec`s itself with the same arguments — the process id survives, so
//! systemd/Docker see no crash. PocketBase restarts for the same reason.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{Multipart, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cratebase_core::{codes, AppError, DateTime, FieldError};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiResult};

/// Directory (inside the data dir) used to stage a restore.
const RESTORE_STAGING: &str = ".cb_restore";
/// The main database's name inside the archive.
const MAIN_DB_ENTRY: &str = "data.db";

pub fn router() -> Router<App> {
    Router::new()
        .route("/backups", get(list).post(create))
        .route("/backups/upload", post(upload))
        // Ahead of `/backups/{key}` on purpose: axum prefers the literal
        // segment, and this is an extension rather than a PocketBase route.
        .route("/backups/storage-info", get(storage_info))
        .route("/backups/{key}", get(download).delete(delete))
        .route("/backups/{key}/restore", post(restore))
}

/// `GET /api/backups/storage-info` — a Cratebase extension, not a
/// PocketBase route.
///
/// Backups go wherever `settings.backups.s3` points, which is a different
/// store from the one serving uploaded files. Nothing on the backups
/// screen otherwise says which, so a superuser configuring S3 has no way
/// to confirm it took effect short of writing a backup and looking.
#[derive(Serialize)]
struct StorageInfo {
    /// `"local"` or `"s3"`.
    driver: &'static str,
    /// Where backups land: the on-disk directory, or `bucket[@endpoint]`
    /// for S3-compatible stores. Never includes credentials.
    location: String,
}

async fn storage_info(
    State(app): State<App>,
    _su: RequireSuperuser,
) -> ApiResult<Json<StorageInfo>> {
    let settings = app.settings();
    let s3 = &settings.backups.s3;
    let info = if s3.enabled {
        StorageInfo {
            driver: "s3",
            location: if s3.endpoint.is_empty() {
                s3.bucket.clone()
            } else {
                format!("{}@{}", s3.bucket, s3.endpoint)
            },
        }
    } else {
        // Ask the store itself rather than rebuilding the path here, so
        // this can't drift from where `backups_storage()` actually writes.
        let storage = app.backups_storage().map_err(ApiError)?;
        StorageInfo {
            driver: "local",
            location: storage
                .local_root()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        }
    };
    Ok(Json(info))
}

/// One row of `GET /api/backups`.
#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    pub key: String,
    pub size: u64,
    pub modified: DateTime,
}

async fn list(State(app): State<App>, _su: RequireSuperuser) -> ApiResult<Json<Vec<BackupInfo>>> {
    let storage = app.backups_storage().map_err(ApiError)?;
    let mut items: Vec<BackupInfo> = storage
        .list("")
        .await?
        .into_iter()
        .filter(|o| o.key.ends_with(".zip"))
        .map(|o| BackupInfo {
            key: o.key,
            size: o.size,
            modified: DateTime::from_utc(o.last_modified),
        })
        .collect();
    items.sort_by_key(|item| std::cmp::Reverse(item.modified.inner()));
    Ok(Json(items))
}

#[derive(Debug, Default, Deserialize)]
struct CreateBody {
    #[serde(default)]
    name: String,
}

/// The body is optional (`pb.backups.create()` sends none), so it is read
/// as raw bytes and parsed leniently rather than through a `Json`
/// extractor that would reject an empty payload.
async fn create(
    State(app): State<App>,
    _su: RequireSuperuser,
    body: bytes::Bytes,
) -> ApiResult<StatusCode> {
    let name = serde_json::from_slice::<CreateBody>(&body)
        .map(|b| b.name)
        .unwrap_or_default();
    let name = if name.trim().is_empty() {
        auto_name()
    } else {
        name
    };
    validate_key(&name)?;

    let storage = app.backups_storage().map_err(ApiError)?;
    if storage.exists(&name).await? {
        return Err(name_error(
            "validation_backup_name_exists",
            "A backup with this name already exists.",
        ));
    }

    let mut event = crate::events::BackupCreateEvent::new(app.clone(), name);
    let app_for_finalizer = app.clone();
    app.hooks()
        .on_backup_create
        .trigger(&mut event, move |e| {
            let app = app_for_finalizer.clone();
            let name = e.name.clone();
            Box::pin(async move { write_backup(&app, &name).await })
        })
        .await
        .map_err(ApiError)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Build the archive and stream it into the backups store.
async fn write_backup(app: &App, name: &str) -> Result<(), AppError> {
    let staging = tempfile::Builder::new()
        .prefix("cratebase-backup")
        .tempdir()
        .map_err(|e| AppError::internal(e.to_string()))?;
    let archive = staging.path().join("archive.zip");

    // Consistent copies first: `VACUUM INTO` on SQLite, an error on
    // Postgres (which the caller reports rather than writing a useless
    // archive).
    let main_copy = staging.path().join(MAIN_DB_ENTRY);
    app.db()
        .engine
        .snapshot_to(&main_copy.to_string_lossy())
        .await
        .map_err(|e| {
            AppError::bad_request(format!(
                "Failed to create a database snapshot.\nRaw error: {e}"
            ))
        })?;
    let logs_copy = staging.path().join(cratebase_db::db::AUXILIARY_DB);
    // The logs database is best-effort: an unreadable one must not stop a
    // backup of the real data.
    let has_logs = app
        .db()
        .logs
        .snapshot_to(&logs_copy.to_string_lossy())
        .await
        .is_ok();

    let data_dir = app.config().data_path().to_path_buf();
    let archive_path = archive.clone();
    let files_root = app.storage().local_root().map(Path::to_path_buf);
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let file = std::fs::File::create(&archive_path)?;
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        add_file(&mut zip, &main_copy, MAIN_DB_ENTRY, options)?;
        if has_logs {
            add_file(
                &mut zip,
                &logs_copy,
                cratebase_db::db::AUXILIARY_DB,
                options,
            )?;
        }
        if let Some(root) = files_root {
            // Only a local file store is part of the archive; an S3 store
            // is already durable elsewhere.
            if root.starts_with(&data_dir) {
                add_tree(
                    &mut zip,
                    &root,
                    cratebase_storage::LOCAL_STORAGE_DIR,
                    options,
                )?;
            }
        }
        zip.finish()?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
    .map_err(|e| AppError::internal(format!("failed to build the backup archive: {e}")))?;

    let size = tokio::fs::metadata(&archive).await.ok().map(|m| m.len());
    let file = tokio::fs::File::open(&archive)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let stream = tokio_util::io::ReaderStream::new(file);
    app.backups_storage()?
        .put_stream(name, stream, size)
        .await
        .map_err(|e| AppError::internal(format!("failed to store the backup: {e}")))?;
    Ok(())
}

fn add_file(
    zip: &mut zip::ZipWriter<std::fs::File>,
    source: &Path,
    entry: &str,
    options: zip::write::FileOptions<'_, ()>,
) -> std::io::Result<()> {
    if !source.exists() {
        return Ok(());
    }
    zip.start_file(entry, options)?;
    let mut reader = std::fs::File::open(source)?;
    std::io::copy(&mut reader, zip)?;
    Ok(())
}

fn add_tree(
    zip: &mut zip::ZipWriter<std::fs::File>,
    root: &Path,
    prefix: &str,
    options: zip::write::FileOptions<'_, ()>,
) -> std::io::Result<()> {
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let name = format!("{prefix}/{}", relative.to_string_lossy().replace('\\', "/"));
        zip.start_file(name, options)?;
        let mut reader = std::fs::File::open(entry.path())?;
        std::io::copy(&mut reader, zip)?;
    }
    Ok(())
}

async fn upload(
    State(app): State<App>,
    _su: RequireSuperuser,
    mut multipart: Multipart,
) -> ApiResult<StatusCode> {
    let mut name = String::new();
    let mut bytes: Option<bytes::Bytes> = None;
    while let Some(field) = multipart.next_field().await.map_err(|_| {
        ApiError::bad_request("Failed to load the submitted data due to invalid formatting.")
    })? {
        match field.name().unwrap_or_default() {
            "file" => {
                name = field.file_name().unwrap_or_default().to_string();
                bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::bad_request(e.to_string()))?,
                );
            }
            "name" => {
                let value = field.text().await.unwrap_or_default();
                if !value.trim().is_empty() {
                    name = value;
                }
            }
            _ => {}
        }
    }

    let Some(bytes) = bytes else {
        return Err(file_error(codes::REQUIRED, "Cannot be blank."));
    };
    // "PK\x03\x04" — anything else is not a ZIP, whatever it is named.
    if bytes.len() < 4 || &bytes[..2] != b"PK" {
        return Err(file_error(
            codes::FILE_MIME,
            "The uploaded file is not a valid backup archive.",
        ));
    }
    if name.trim().is_empty() {
        name = auto_name();
    }
    validate_key(&name)?;

    let storage = app.backups_storage().map_err(ApiError)?;
    if storage.exists(&name).await? {
        return Err(name_error(
            "validation_backup_name_exists",
            "A backup with this name already exists.",
        ));
    }
    storage.put(&name, bytes).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Default, Deserialize)]
struct DownloadQuery {
    #[serde(default)]
    token: String,
}

/// Streams the archive. Authenticated by a superuser **file token** in
/// the query string, since `<a download>` and `window.open` cannot send
/// headers. A missing or invalid token is a 403, matching PocketBase.
async fn download(
    State(app): State<App>,
    axum::extract::Path(key): axum::extract::Path<String>,
    axum::extract::Query(query): axum::extract::Query<DownloadQuery>,
) -> ApiResult<Response> {
    if !verify_superuser_file_token(&app, &query.token).await {
        return Err(ApiError::forbidden(
            "Insufficient permissions to access the resource.",
        ));
    }
    validate_key(&key)?;

    let storage = app.backups_storage().map_err(ApiError)?;
    let stream = storage
        .get_stream(&key)
        .await
        .map_err(|_| ApiError::not_found(""))?;
    let body = Body::from_stream(stream.map_err(std::io::Error::other));
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{key}\""),
            ),
        ],
        body,
    )
        .into_response())
}

async fn delete(
    State(app): State<App>,
    _su: RequireSuperuser,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> ApiResult<StatusCode> {
    validate_key(&key)?;
    let storage = app.backups_storage().map_err(ApiError)?;
    if !storage.exists(&key).await? {
        return Err(ApiError::bad_request(format!(
            "Invalid or already deleted backup file.\nRaw error: {key} does not exist."
        )));
    }
    storage.delete(&key).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn restore(
    State(app): State<App>,
    _su: RequireSuperuser,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> ApiResult<StatusCode> {
    validate_key(&key).map_err(|_| ApiError::bad_request("Missing or invalid backup file."))?;
    let storage = app.backups_storage().map_err(ApiError)?;
    if !storage.exists(&key).await? {
        return Err(ApiError::bad_request("Missing or invalid backup file."));
    }

    let mut event = crate::events::BackupRestoreEvent::new(app.clone(), key);
    let app_for_finalizer = app.clone();
    app.hooks()
        .on_backup_restore
        .trigger(&mut event, move |e| {
            let app = app_for_finalizer.clone();
            let key = e.name.clone();
            Box::pin(async move { stage_and_restart(app, key).await })
        })
        .await
        .map_err(ApiError)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Extract the archive next to the data dir, verify it really is one, and
/// schedule the swap + re-exec. The swap happens *after* this response
/// has been written, so the client gets its 204.
async fn stage_and_restart(app: App, key: String) -> Result<(), AppError> {
    let data_dir = app.config().data_path().to_path_buf();
    let staging = data_dir.join(RESTORE_STAGING);
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let archive = staging.join("archive.zip");
    let storage = app.backups_storage()?;
    let stream = storage
        .get_stream(&key)
        .await
        .map_err(|_| AppError::bad_request("Missing or invalid backup file."))?;
    let mut file = tokio::fs::File::create(&archive)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let mut stream = Box::pin(stream);
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream
        .try_next()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
    }
    file.flush()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    drop(file);

    let extracted = staging.join("extracted");
    let archive_path = archive.clone();
    let target = extracted.clone();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let reader = std::fs::File::open(&archive_path)?;
        let mut zip = zip::ZipArchive::new(reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        zip.extract(&target)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
    .map_err(|_| AppError::bad_request("Missing or invalid backup file."))?;

    if !extracted.join(MAIN_DB_ENTRY).exists() {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(AppError::bad_request("Missing or invalid backup file."));
    }

    tokio::spawn(async move {
        // Give the 204 time to reach the client before the process
        // disappears.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        app.terminate(true).await;
        if let Err(e) = swap_data_dir(&data_dir, &extracted) {
            tracing::error!(error = %e, "restore failed while swapping the data directory");
            return;
        }
        restart_process();
    });
    Ok(())
}

/// Replace the data directory's contents with `restored`, keeping the
/// `backups/` directory (which holds the archive being restored) and the
/// staging directory out of it.
fn swap_data_dir(data_dir: &Path, restored: &Path) -> std::io::Result<()> {
    let keep = [cratebase_storage::LOCAL_BACKUPS_DIR, RESTORE_STAGING];
    for entry in std::fs::read_dir(data_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if keep.iter().any(|k| *k == name) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    for entry in std::fs::read_dir(restored)? {
        let entry = entry?;
        std::fs::rename(entry.path(), data_dir.join(entry.file_name()))?;
    }
    let _ = std::fs::remove_dir_all(data_dir.join(RESTORE_STAGING));
    Ok(())
}

/// Re-exec the current binary with the same arguments. On success this
/// never returns; the process image is replaced, so the pid, open
/// listening socket ownership and supervisor state are all unchanged.
fn restart_process() {
    let Ok(exe) = std::env::current_exe() else {
        tracing::error!(
            "cannot locate the current executable; exiting so the supervisor restarts us"
        );
        std::process::exit(0);
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = std::process::Command::new(exe).args(args).exec();
        tracing::error!(error = %error, "re-exec after restore failed");
        std::process::exit(1);
    }
    #[cfg(not(unix))]
    {
        let _ = std::process::Command::new(exe).args(args).spawn();
        std::process::exit(0);
    }
}

/// The `settings.backups.cron` job: take a backup, then keep only the
/// newest `cronMaxKeep` archives.
pub async fn create_scheduled(app: &App) -> Result<(), AppError> {
    let name = auto_name();
    write_backup(app, &name).await?;

    let max_keep = app.settings().backups.cron_max_keep;
    if max_keep <= 0 {
        return Ok(());
    }
    let storage = app.backups_storage()?;
    let mut archives: Vec<_> = storage
        .list("")
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .into_iter()
        .filter(|o| o.key.ends_with(".zip"))
        .collect();
    archives.sort_by_key(|archive| std::cmp::Reverse(archive.last_modified));
    for stale in archives.into_iter().skip(max_keep as usize) {
        if let Err(e) = storage.delete(&stale.key).await {
            tracing::warn!(key = %stale.key, error = %e, "failed to rotate an old backup");
        }
    }
    Ok(())
}

/// PocketBase's generated name: `pb_backup_YYYYMMDDHHMMSS.zip`.
fn auto_name() -> String {
    format!(
        "pb_backup_{}.zip",
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    )
}

/// Backup keys become filenames in the object store, so they must not be
/// able to escape it. Anything but `[A-Za-z0-9_-]+.zip` is rejected.
fn validate_key(key: &str) -> Result<(), ApiError> {
    let valid = key.len() > 4
        && key.ends_with(".zip")
        && key[..key.len() - 4]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !key.starts_with('-');
    if valid {
        Ok(())
    } else {
        Err(name_error(
            codes::INVALID_FORMAT,
            "Must be a valid backup file name (letters, digits, '-', '_' and a .zip suffix).",
        ))
    }
}

fn name_error(code: &str, message: &str) -> ApiError {
    field_error("name", code, message)
}

fn file_error(code: &str, message: &str) -> ApiError {
    field_error("file", code, message)
}

fn field_error(field: &str, code: &str, message: &str) -> ApiError {
    let mut data = Map::new();
    data.insert(
        field.to_string(),
        serde_json::to_value(FieldError::new(code, message)).unwrap_or(Value::Null),
    );
    ApiError::nested_validation(
        "An error occurred while validating the submitted data.",
        data,
    )
}

/// Verify a `?token=` file token minted for a superuser.
///
/// Both the minting side (`POST /api/files/token`) and the verification
/// live in [`crate::routes::files`]; this only adds the extra condition
/// that a backup needs a *superuser's* token, not just any record's.
async fn verify_superuser_file_token(app: &App, token: &str) -> bool {
    crate::routes::files::file_token_context(app, token)
        .await
        .is_some_and(|auth| auth.is_superuser)
}

/// The path a restore stages into; exposed for tests.
pub fn staging_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(RESTORE_STAGING)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_cannot_escape_the_store() {
        validate_key("pb_backup_20260903120000.zip").unwrap();
        validate_key("my-backup.zip").unwrap();
        for bad in [
            "../escape.zip",
            "nested/dir.zip",
            "no-extension",
            ".zip",
            "with space.zip",
            "trailing.zip.txt",
        ] {
            assert!(validate_key(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn auto_names_follow_pocketbase() {
        let name = auto_name();
        assert!(name.starts_with("pb_backup_"), "{name}");
        assert!(name.ends_with(".zip"));
        validate_key(&name).unwrap();
    }

    #[test]
    fn a_name_error_is_a_nested_400() {
        let err = validate_key("../x.zip").unwrap_err();
        assert_eq!(err.error.status(), 400);
        assert!(err.data.unwrap().contains_key("name"));
    }
}
