//! `/api/backups` — superuser only (download also accepts a superuser
//! file token in the query string, because a browser download cannot set
//! an `Authorization` header).
//!
//! A backup is a ZIP of the data directory: a consistent snapshot of the
//! main database (`Engine::snapshot_to` — `VACUUM INTO` on SQLite, `pg_dump
//! --format=custom` on Postgres, never a raw file copy that could catch a
//! half-written page), the same for `auxiliary.db`, and the `storage/`
//! tree when files live on local disk. The `backups/` directory itself is
//! excluded, so a backup never contains its own siblings.
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
//! On SQLite, restoring swaps the files the open connections are mapped
//! to — there is no safe way to keep serving from those handles, so the
//! engines are closed, the directories swapped, and the current binary
//! `exec`s itself with the same arguments. On Postgres there is no file
//! to swap (`pg_restore` runs against the live connection), but the
//! process still restarts anyway: every schema/settings cache loaded at
//! boot (`Db::bootstrap`'s `CollectionStore`, `App::settings`) has to be
//! rebuilt from whatever the restore just put in `_collections`/
//! `_params`, and re-exec'ing is the same "start from a clean slate"
//! mechanism SQLite already needs, reused rather than duplicated with an
//! in-place reload path. Either way the process id survives, so
//! systemd/Docker see no crash. PocketBase restarts for the same reason.
//!
//! # What actually gets swapped
//!
//! The archive always names the main database `data.db` (SQLite) or
//! `data.pgdump` (Postgres) and the logs database `auxiliary.db`
//! (`MAIN_DB_ENTRY`/`MAIN_DB_ENTRY_PG`/`AUXILIARY_DB`; see
//! `main_db_entry`) — an archive's main-database entry name is also how
//! `validate_archive_for_backend` tells a SQLite backup apart from a
//! Postgres one, and rejects the wrong kind with a clear 400 instead of
//! a confusing failure partway through. The *live* SQLite main database
//! can be named anything `DATABASE_URL` says and need not even live
//! under the data directory. `swap_data_dir` maps the archive's fixed
//! names onto the live paths derived from config, rather than assuming
//! the data directory's own layout — and replaces only what
//! `write_backup` actually captured (the main db, the logs db, a local
//! `storage/` tree), leaving `.secret`, `plugins/`, `backups/` and
//! anything else untouched. Each file is swapped by parking the live
//! version next to it instead of deleting it outright, so a failure
//! partway through rolls back instead of leaving a half-restored data
//! directory. A Postgres restore only ever swaps the logs
//! database/storage tree this way (`swap_side_files_only`) — the main
//! database was already restored in place by `pg_restore` itself.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{Multipart, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cratebase_core::{codes, AppError, DateTime, FieldError};
use cratebase_db::Backend;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiResult};

/// Directory (inside the data dir) used to stage a restore.
const RESTORE_STAGING: &str = ".cb_restore";
/// The main database's name inside the archive, for a SQLite main
/// database.
const MAIN_DB_ENTRY: &str = "data.db";
/// As `MAIN_DB_ENTRY`, for a Postgres main database — a `pg_dump
/// --format=custom` archive rather than a raw database file. The two
/// names double as how a restore tells which engine an archive was
/// taken from (see `validate_archive_for_backend`): an archive only ever
/// has one or the other, never both.
const MAIN_DB_ENTRY_PG: &str = "data.pgdump";
/// Suffix a live file/directory gets while it is parked during a restore
/// swap, so a crash between moving it out and moving the restored one in
/// can be recovered by hand. Removed on success, renamed back on failure.
const RESTORE_OLD_SUFFIX: &str = ".cb_restore_old";
/// SQLite's file header signature ("SQLite format 3\0"). The only other
/// check an archive gets is that it's a well-formed ZIP, so nothing else
/// stops one whose `data.db` entry isn't actually a database.
const SQLITE_HEADER: [u8; 16] = *b"SQLite format 3\0";
/// `pg_dump --format=custom`'s own magic header
/// (`pg_backup_archiver.c`'s `K_MAGIC`), checked the same way
/// `SQLITE_HEADER` is for a SQLite archive.
const PGDUMP_MAGIC: [u8; 5] = *b"PGDMP";
/// The `_params` key `create_scheduled` persists its outcome under, read
/// back by `storage_info` so the dashboard's Backups page can show
/// whether the last `settings.backups.cron` run succeeded — otherwise a
/// failure is only ever a `tracing::error!` nobody but an operator
/// tailing logs would see.
const LAST_SCHEDULED_KEY: &str = "backups_last_scheduled";

/// The name of the archive's main-database entry for `backend`, and (via
/// [`validate_archive_for_backend`]) how a restore recognizes which
/// engine produced a given archive.
fn main_db_entry(backend: Backend) -> &'static str {
    match backend {
        Backend::Sqlite => MAIN_DB_ENTRY,
        Backend::Postgres => MAIN_DB_ENTRY_PG,
    }
}

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
#[serde(rename_all = "camelCase")]
struct StorageInfo {
    /// `"local"` or `"s3"`.
    driver: &'static str,
    /// Where backups land: the on-disk directory, or `bucket[@endpoint]`
    /// for S3-compatible stores. Never includes credentials.
    location: String,
    /// The outcome of the most recent `settings.backups.cron` run, if
    /// any has happened yet. `None` before the cron job has ever fired
    /// (including when it isn't configured at all).
    #[serde(skip_serializing_if = "Option::is_none")]
    last_scheduled: Option<LastScheduledBackup>,
}

/// What `create_scheduled` recorded about its most recent run — see
/// [`LAST_SCHEDULED_KEY`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastScheduledBackup {
    pub at: DateTime,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

async fn storage_info(
    State(app): State<App>,
    _su: RequireSuperuser,
) -> ApiResult<Json<StorageInfo>> {
    let settings = app.settings();
    let s3 = &settings.backups.s3;
    let last_scheduled = load_last_scheduled_backup(&app).await;
    let info = if s3.enabled {
        StorageInfo {
            driver: "s3",
            location: if s3.endpoint.is_empty() {
                s3.bucket.clone()
            } else {
                format!("{}@{}", s3.bucket, s3.endpoint)
            },
            last_scheduled,
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
            last_scheduled,
        }
    };
    Ok(Json(info))
}

/// Best-effort read of [`LAST_SCHEDULED_KEY`] — a missing or corrupt
/// value (an older Cratebase never wrote one, say) just means the
/// dashboard shows nothing yet, not an error.
async fn load_last_scheduled_backup(app: &App) -> Option<LastScheduledBackup> {
    let raw = cratebase_db::params::get(&*app.db().engine, LAST_SCHEDULED_KEY)
        .await
        .ok()??;
    serde_json::from_str(&raw).ok()
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

    // Consistent copies first: `VACUUM INTO` on SQLite, `pg_dump
    // --format=custom` on Postgres (`Engine::snapshot_to`) — either way
    // the entry name records which one, so a restore can tell them apart
    // (see `validate_archive_for_backend`).
    let backend = app.db().backend;
    let main_entry_name = main_db_entry(backend);
    let main_copy = staging.path().join(main_entry_name);
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

        add_file(&mut zip, &main_copy, main_entry_name, options)?;
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

/// Extract the archive next to the data dir, verify it really is one
/// *for this deployment's backend*, and schedule the swap/`pg_restore` +
/// re-exec. That work happens *after* this response has been written, so
/// the client gets its 204.
async fn stage_and_restart(app: App, key: String) -> Result<(), AppError> {
    let backend = app.db().backend;
    // SQLite restores by swapping the live database *file*, which needs
    // to know its real path up front (a Postgres restore has no such
    // file — `pg_restore` talks to the live database directly).
    let main_db_path = match backend {
        Backend::Sqlite => Some(app.config().sqlite_main_path().ok_or_else(|| {
            AppError::bad_request("Restoring a backup requires a SQLite main database.")
        })?),
        Backend::Postgres => None,
    };
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
    tokio::task::spawn_blocking(move || -> Result<(), ExtractError> {
        let reader = std::fs::File::open(&archive_path)?;
        let mut zip = zip::ZipArchive::new(reader)?;
        zip.extract(&target)?;
        // Reject a corrupt, tampered, or wrong-engine archive now, before
        // the response goes out and the app commits to tearing itself
        // down — `swap_data_dir` checks the SQLite case again, but only
        // after that point.
        validate_archive_for_backend(&target, backend)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
    .map_err(ExtractError::into_app_error)?;

    tokio::spawn(async move {
        // Give the 204 time to reach the client before the process
        // disappears (SQLite) or the database starts getting
        // `pg_restore`d out from under any request still in flight
        // (Postgres).
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        app.terminate(true).await;
        match backend {
            Backend::Sqlite => {
                let main_db_path =
                    main_db_path.expect("Backend::Sqlite always has a resolved sqlite_main_path");
                if let Err(e) = swap_data_dir(&data_dir, &extracted, &main_db_path) {
                    tracing::error!(error = %e, "restore failed while swapping the data directory");
                    return;
                }
            }
            Backend::Postgres => {
                let pgdump = extracted.join(MAIN_DB_ENTRY_PG);
                if let Err(e) = app
                    .db()
                    .engine
                    .restore_from(&pgdump.to_string_lossy())
                    .await
                {
                    tracing::error!(error = %e, "restore failed while running pg_restore");
                    return;
                }
                if let Err(e) = swap_side_files_only(&data_dir, &extracted) {
                    tracing::error!(error = %e, "restore failed while swapping the logs database/storage tree");
                    return;
                }
            }
        }
        restart_process();
    });
    Ok(())
}

/// Replace exactly the files a backup captures (see `write_backup`) with
/// the ones extracted from the archive at `restored`, leaving everything
/// else in `data_dir` alone — most importantly `.secret`
/// (`config::SECRET_FILE`; regenerating it would invalidate every issued
/// token) and `plugins/` (installed WASM plugins), neither of which the
/// archive contains. `backups/` and the restore staging directory were
/// never part of `restored` either, so they need no special-casing.
///
/// `main_db_path` is the *live* main database's real path
/// (`Config::sqlite_main_path`), not assumed to be `<data_dir>/data.db`:
/// the archive's main database is always named `data.db`
/// (`MAIN_DB_ENTRY`), but a deployment can point `DATABASE_URL` at any
/// filename — `docker-compose.yml` uses `cratebase.db` — and it need not
/// even live under `data_dir`. Deleting everything but a fixed allowlist
/// used to delete that file and never replace it, so a restore came back
/// with an empty database.
///
/// The restored main database is validated before anything is touched,
/// then each live file/directory is replaced one at a time by parking it
/// next to itself (`<name>.cb_restore_old`) rather than deleting it
/// outright. If anything fails partway, every change already made is
/// undone before the error is returned, so a failed restore leaves the
/// data directory exactly as it was.
fn swap_data_dir(data_dir: &Path, restored: &Path, main_db_path: &Path) -> std::io::Result<()> {
    let main_entry = restored.join(MAIN_DB_ENTRY);
    validate_sqlite_file(&main_entry)?;

    // (archive source, live target) — only the files `write_backup` puts
    // in an archive are ever swapped; an older archive without a logs
    // database or a local storage tree just skips those entries.
    let mut swaps = vec![(main_entry, main_db_path.to_path_buf())];
    swaps.extend(side_file_swaps(data_dir, restored));

    let result = apply_swaps(&swaps);
    // The staged download/extraction is disposable either way — a retry
    // re-downloads and re-extracts the archive from scratch.
    let _ = std::fs::remove_dir_all(data_dir.join(RESTORE_STAGING));
    result
}

/// The logs-database and local-storage-tree swaps `swap_data_dir` and
/// [`swap_side_files_only`] share — everything a backup archive can
/// contain besides the main database itself.
fn side_file_swaps(data_dir: &Path, restored: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut swaps = Vec::new();
    let logs_entry = restored.join(cratebase_db::db::AUXILIARY_DB);
    if logs_entry.exists() {
        swaps.push((logs_entry, data_dir.join(cratebase_db::db::AUXILIARY_DB)));
    }
    let storage_entry = restored.join(cratebase_storage::LOCAL_STORAGE_DIR);
    if storage_entry.exists() {
        swaps.push((
            storage_entry,
            data_dir.join(cratebase_storage::LOCAL_STORAGE_DIR),
        ));
    }
    swaps
}

/// As [`swap_data_dir`], but for a Postgres restore: the main database
/// was already restored in place by `pg_restore` against the live
/// connection (there is no file for this to swap), so only the logs
/// database and local storage tree — everything else `write_backup`
/// captures — are swapped here.
fn swap_side_files_only(data_dir: &Path, restored: &Path) -> std::io::Result<()> {
    let swaps = side_file_swaps(data_dir, restored);
    let result = apply_swaps(&swaps);
    let _ = std::fs::remove_dir_all(data_dir.join(RESTORE_STAGING));
    result
}

/// Move every restored file/directory into place, parking what it
/// replaces first. On the first failure, everything already swapped in is
/// rolled back, in reverse order, before the error is returned.
fn apply_swaps(swaps: &[(PathBuf, PathBuf)]) -> std::io::Result<()> {
    let mut moved_aside = Vec::new();
    let mut swapped_in = Vec::new();
    for (source, target) in swaps {
        if let Err(e) = swap_one(source, target, &mut moved_aside, &mut swapped_in) {
            for target in swapped_in.iter().rev() {
                let _ = remove_path(target);
            }
            for (aside, original) in moved_aside.iter().rev() {
                let _ = std::fs::rename(aside, original);
            }
            return Err(e);
        }
    }
    for (aside, _) in &moved_aside {
        let _ = remove_path(aside);
    }
    Ok(())
}

/// Park `target` (see [`live_paths`]) if it exists, then move `source`
/// into its place.
fn swap_one(
    source: &Path,
    target: &Path,
    moved_aside: &mut Vec<(PathBuf, PathBuf)>,
    swapped_in: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    for live in live_paths(source, target) {
        if live.exists() {
            let aside = aside_path(&live);
            std::fs::rename(&live, &aside)?;
            moved_aside.push((aside, live));
        }
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    move_path(source, target)?;
    swapped_in.push(target.to_path_buf());
    Ok(())
}

/// `target` itself, plus its `-wal`/`-shm` sidecars when `source` (and so
/// `target`, once swapped) is a database file rather than a directory
/// (`storage/` has no sidecars) — stale ones left from the live database
/// must not survive next to a freshly restored file.
fn live_paths(source: &Path, target: &Path) -> Vec<PathBuf> {
    if !source.is_file() {
        return vec![target.to_path_buf()];
    }
    let mut wal = target.as_os_str().to_owned();
    wal.push("-wal");
    let mut shm = target.as_os_str().to_owned();
    shm.push("-shm");
    vec![target.to_path_buf(), PathBuf::from(wal), PathBuf::from(shm)]
}

fn aside_path(live: &Path) -> PathBuf {
    let mut name = live.file_name().unwrap_or_default().to_os_string();
    name.push(RESTORE_OLD_SUFFIX);
    live.with_file_name(name)
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// `std::fs::rename`, falling back to a copy-then-remove when it fails —
/// `main_db_path` can point outside `data_dir` entirely, so the two sides
/// of a swap aren't guaranteed to share a filesystem, and a plain rename
/// always fails across one.
fn move_path(source: &Path, target: &Path) -> std::io::Result<()> {
    if std::fs::rename(source, target).is_ok() {
        return Ok(());
    }
    if source.is_dir() {
        copy_dir_all(source, target)?;
        std::fs::remove_dir_all(source)
    } else {
        std::fs::copy(source, target)?;
        std::fs::remove_file(source)
    }
}

/// Recursive copy for the [`move_path`] cross-filesystem fallback.
fn copy_dir_all(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in walkdir::WalkDir::new(source) {
        let entry = entry.map_err(std::io::Error::other)?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(std::io::Error::other)?;
        let dest = target.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else if entry.file_type().is_file() {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

/// Reject an archive whose main database entry isn't really SQLite.
fn validate_sqlite_file(path: &Path) -> std::io::Result<()> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut header = [0u8; 16];
    file.read_exact(&mut header)?;
    if header == SQLITE_HEADER {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the archive's main database is not a SQLite file",
        ))
    }
}

/// As [`validate_sqlite_file`], for a `pg_dump --format=custom` entry.
fn validate_pgdump_file(path: &Path) -> std::io::Result<()> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut header = [0u8; PGDUMP_MAGIC.len()];
    file.read_exact(&mut header)?;
    if header == PGDUMP_MAGIC {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the archive's main database is not a pg_dump custom-format file",
        ))
    }
}

/// What can go wrong extracting/validating a restore archive
/// (`stage_and_restart`'s `spawn_blocking`): either it's simply invalid
/// (corrupt ZIP, tampered/truncated main database, ...), or — the case
/// worth a specific message — it's a perfectly good archive taken from
/// the *other* engine.
#[derive(Debug)]
enum ExtractError {
    Invalid,
    Mismatch(String),
}

impl ExtractError {
    fn into_app_error(self) -> AppError {
        match self {
            ExtractError::Invalid => AppError::bad_request("Missing or invalid backup file."),
            ExtractError::Mismatch(message) => AppError::bad_request(message),
        }
    }
}

impl From<std::io::Error> for ExtractError {
    fn from(_: std::io::Error) -> Self {
        ExtractError::Invalid
    }
}

impl From<zip::result::ZipError> for ExtractError {
    fn from(_: zip::result::ZipError) -> Self {
        ExtractError::Invalid
    }
}

/// Confirm `extracted` (an already-unzipped archive) actually matches
/// `backend`: exactly one of [`MAIN_DB_ENTRY`]/[`MAIN_DB_ENTRY_PG`]
/// should be present (see [`main_db_entry`]), and it should pass its
/// backend's own header check. An archive from the *other* engine is a
/// [`ExtractError::Mismatch`] with a message naming both engines, not
/// the generic "Missing or invalid backup file." every other failure
/// here gets — restoring a SQLite backup onto Postgres (or vice versa)
/// isn't corruption, it's an operator pointing a restore at a backup
/// from a different deployment, and the fix is "restore it onto the
/// right kind of server", not "try a different file".
fn validate_archive_for_backend(extracted: &Path, backend: Backend) -> Result<(), ExtractError> {
    let has_sqlite = extracted.join(MAIN_DB_ENTRY).exists();
    let has_pgdump = extracted.join(MAIN_DB_ENTRY_PG).exists();
    match backend {
        Backend::Sqlite if has_sqlite => {
            validate_sqlite_file(&extracted.join(MAIN_DB_ENTRY))?;
            Ok(())
        }
        Backend::Sqlite if has_pgdump => Err(ExtractError::Mismatch(
            "This backup was taken from a Postgres database, but the live database is \
             SQLite. Restore it onto a Postgres deployment instead."
                .to_string(),
        )),
        Backend::Postgres if has_pgdump => {
            validate_pgdump_file(&extracted.join(MAIN_DB_ENTRY_PG))?;
            Ok(())
        }
        Backend::Postgres if has_sqlite => Err(ExtractError::Mismatch(
            "This backup was taken from a SQLite database, but the live database is \
             Postgres. Restore it onto a SQLite deployment instead."
                .to_string(),
        )),
        _ => Err(ExtractError::Invalid),
    }
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
    let outcome = write_backup(app, &name).await;
    record_scheduled_backup_result(app, outcome.as_ref().err()).await;
    outcome?;

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

/// Record `settings.backups.cron`'s outcome somewhere an operator will
/// actually see it — before this, a scheduled backup failure was only
/// ever a `tracing::error!` in `App::sync_backup_cron`, invisible unless
/// someone was already tailing logs at the right moment. Two places, so
/// two different audiences can find it:
///
/// - [`LAST_SCHEDULED_KEY`] in `_params`, read back by [`storage_info`]
///   so the dashboard's Backups page can show "last run: ok/failed,
///   when, why" without anyone having to go looking.
/// - `_audit_log`, *only* on failure — a healthy backup running on
///   schedule isn't an audit-worthy event (nobody needs to know "who"
///   restored what if it happens automatically and works), but a
///   failure is exactly the kind of thing `_audit_log` exists to
///   surface for an operator scanning "what needs my attention".
///   `actor: None` because a cron job has no caller behind it — see
///   `Collection::default_system_collections`' doc on `_audit_log`'s
///   `actor` field.
///
/// Best-effort like `crate::audit::write` itself: a failure to record
/// the failure must not additionally break the cron job's own error
/// propagation (`create_scheduled` still returns the original `Err`).
async fn record_scheduled_backup_result(app: &App, error: Option<&AppError>) {
    let status = LastScheduledBackup {
        at: DateTime::now(),
        ok: error.is_none(),
        message: error.map(|e| e.to_string()),
    };
    match serde_json::to_string(&status) {
        Ok(raw) => {
            if let Err(e) =
                cratebase_db::params::set(&*app.db().engine, LAST_SCHEDULED_KEY, &raw).await
            {
                tracing::warn!(error = %e, "failed to persist scheduled backup status");
            }
        }
        Err(e) => tracing::warn!(error = %e, "failed to serialize scheduled backup status"),
    }

    if let Some(err) = error {
        crate::audit::write(
            app,
            None,
            "backup.scheduled_failed",
            "settings.backups.cron",
            json!({ "error": err.to_string() }),
        )
        .await;
    }
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

    // ----------------------------------------------------- swap_data_dir

    use cratebase_db::engine::{Engine, Executor, Sql};
    use cratebase_db::sqlite::SqliteEngine;

    /// A real, openable SQLite file at `path` — enough to pass
    /// `validate_sqlite_file` and to prove a restore actually swapped the
    /// bytes rather than just moving an opaque blob around.
    async fn make_sqlite_file(path: &Path, marker: &str) {
        let engine = SqliteEngine::open(&path.to_string_lossy(), 0).unwrap();
        engine
            .execute("CREATE TABLE marker (val TEXT)", &[])
            .await
            .unwrap();
        engine
            .execute(
                "INSERT INTO marker (val) VALUES ($1)",
                &[Sql::Text(marker.to_string())],
            )
            .await
            .unwrap();
        engine.close().await.unwrap();
    }

    async fn read_marker(path: &Path) -> String {
        let engine = SqliteEngine::open(&path.to_string_lossy(), 0).unwrap();
        let row = engine
            .query_one("SELECT val FROM marker", &[])
            .await
            .unwrap()
            .expect("a marker row");
        let value = row.get_str("val").unwrap().to_string();
        engine.close().await.unwrap();
        value
    }

    /// The bug the audit flagged: the archive's main database is always
    /// named `data.db`, but `DATABASE_URL` (`docker-compose.yml` uses
    /// `cratebase.db`) can name the live file anything. Restoring must
    /// write into that real path, not a `data.db` the app never opens.
    #[tokio::test]
    async fn restore_brings_back_rows_when_the_main_db_has_a_custom_name() {
        let data_dir = tempfile::tempdir().unwrap();
        let main_db_path = data_dir.path().join("cratebase.db");
        make_sqlite_file(&main_db_path, "stale").await;

        // What `stage_and_restart` hands `swap_data_dir`: the archive
        // extracted into a staging dir, its main database always named
        // `data.db` regardless of the live file's real name.
        let restored = tempfile::tempdir().unwrap();
        make_sqlite_file(&restored.path().join(MAIN_DB_ENTRY), "restored").await;

        swap_data_dir(data_dir.path(), restored.path(), &main_db_path).unwrap();

        assert_eq!(read_marker(&main_db_path).await, "restored");
    }

    #[tokio::test]
    async fn restore_swaps_the_logs_database_and_local_storage_tree_too() {
        let data_dir = tempfile::tempdir().unwrap();
        let main_db_path = data_dir.path().join("data.db");
        make_sqlite_file(&main_db_path, "stale-main").await;
        make_sqlite_file(
            &data_dir.path().join(cratebase_db::db::AUXILIARY_DB),
            "stale-logs",
        )
        .await;
        let storage_dir = data_dir.path().join(cratebase_storage::LOCAL_STORAGE_DIR);
        std::fs::create_dir_all(&storage_dir).unwrap();
        std::fs::write(storage_dir.join("old.txt"), b"old").unwrap();

        let restored = tempfile::tempdir().unwrap();
        make_sqlite_file(&restored.path().join(MAIN_DB_ENTRY), "new-main").await;
        make_sqlite_file(
            &restored.path().join(cratebase_db::db::AUXILIARY_DB),
            "new-logs",
        )
        .await;
        let restored_storage = restored.path().join(cratebase_storage::LOCAL_STORAGE_DIR);
        std::fs::create_dir_all(&restored_storage).unwrap();
        std::fs::write(restored_storage.join("new.txt"), b"new").unwrap();

        swap_data_dir(data_dir.path(), restored.path(), &main_db_path).unwrap();

        assert_eq!(read_marker(&main_db_path).await, "new-main");
        assert_eq!(
            read_marker(&data_dir.path().join(cratebase_db::db::AUXILIARY_DB)).await,
            "new-logs"
        );
        assert!(!storage_dir.join("old.txt").exists());
        assert_eq!(std::fs::read(storage_dir.join("new.txt")).unwrap(), b"new");
    }

    #[tokio::test]
    async fn restore_preserves_the_secret_file_and_installed_plugins() {
        let data_dir = tempfile::tempdir().unwrap();
        std::fs::write(data_dir.path().join(crate::config::SECRET_FILE), "shh").unwrap();
        let plugins_dir = data_dir.path().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        std::fs::write(plugins_dir.join("example.wasm"), b"wasm").unwrap();
        let backups_dir = data_dir.path().join(cratebase_storage::LOCAL_BACKUPS_DIR);
        std::fs::create_dir_all(&backups_dir).unwrap();
        std::fs::write(backups_dir.join("some.zip"), b"zip").unwrap();

        let main_db_path = data_dir.path().join("data.db");
        make_sqlite_file(&main_db_path, "stale").await;
        let restored = tempfile::tempdir().unwrap();
        make_sqlite_file(&restored.path().join(MAIN_DB_ENTRY), "restored").await;

        swap_data_dir(data_dir.path(), restored.path(), &main_db_path).unwrap();

        assert_eq!(
            std::fs::read_to_string(data_dir.path().join(crate::config::SECRET_FILE)).unwrap(),
            "shh"
        );
        assert_eq!(
            std::fs::read(plugins_dir.join("example.wasm")).unwrap(),
            b"wasm"
        );
        assert_eq!(std::fs::read(backups_dir.join("some.zip")).unwrap(), b"zip");
    }

    #[test]
    fn a_non_sqlite_archive_is_rejected_before_anything_is_touched() {
        let data_dir = tempfile::tempdir().unwrap();
        let main_db_path = data_dir.path().join("data.db");
        std::fs::write(&main_db_path, b"OLD").unwrap();

        let restored = tempfile::tempdir().unwrap();
        std::fs::write(
            restored.path().join(MAIN_DB_ENTRY),
            b"not a sqlite database, just plain text",
        )
        .unwrap();

        let err = swap_data_dir(data_dir.path(), restored.path(), &main_db_path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(&main_db_path).unwrap(), b"OLD");
    }

    // ------------------------------------------------- Postgres restore

    use cratebase_db::Backend;

    #[test]
    fn main_db_entry_picks_the_right_name_per_backend() {
        assert_eq!(main_db_entry(Backend::Sqlite), MAIN_DB_ENTRY);
        assert_eq!(main_db_entry(Backend::Postgres), MAIN_DB_ENTRY_PG);
        assert_ne!(MAIN_DB_ENTRY, MAIN_DB_ENTRY_PG);
    }

    fn write_pgdump_like_file(path: &Path) {
        // Real magic header, then arbitrary bytes — enough to pass
        // `validate_pgdump_file` without needing a real `pg_dump`.
        let mut bytes = PGDUMP_MAGIC.to_vec();
        bytes.extend_from_slice(b"fake custom-format body");
        std::fs::write(path, bytes).unwrap();
    }

    #[tokio::test]
    async fn a_sqlite_archive_matches_a_sqlite_backend() {
        let extracted = tempfile::tempdir().unwrap();
        make_sqlite_file(&extracted.path().join(MAIN_DB_ENTRY), "ok").await;
        validate_archive_for_backend(extracted.path(), Backend::Sqlite).unwrap();
    }

    #[test]
    fn a_pgdump_archive_matches_a_postgres_backend() {
        let extracted = tempfile::tempdir().unwrap();
        write_pgdump_like_file(&extracted.path().join(MAIN_DB_ENTRY_PG));
        validate_archive_for_backend(extracted.path(), Backend::Postgres).unwrap();
    }

    #[tokio::test]
    async fn a_postgres_archive_restored_onto_sqlite_is_a_clear_mismatch_not_a_generic_error() {
        let extracted = tempfile::tempdir().unwrap();
        write_pgdump_like_file(&extracted.path().join(MAIN_DB_ENTRY_PG));
        let err = validate_archive_for_backend(extracted.path(), Backend::Sqlite).unwrap_err();
        match err {
            ExtractError::Mismatch(msg) => {
                assert!(msg.contains("Postgres"), "{msg}");
                assert!(msg.contains("SQLite"), "{msg}");
            }
            ExtractError::Invalid => panic!("expected a Mismatch, got the generic Invalid"),
        }
    }

    #[tokio::test]
    async fn a_sqlite_archive_restored_onto_postgres_is_a_clear_mismatch_not_a_generic_error() {
        let extracted = tempfile::tempdir().unwrap();
        make_sqlite_file(&extracted.path().join(MAIN_DB_ENTRY), "ok").await;
        let err = validate_archive_for_backend(extracted.path(), Backend::Postgres).unwrap_err();
        match err {
            ExtractError::Mismatch(msg) => {
                assert!(msg.contains("Postgres"), "{msg}");
                assert!(msg.contains("SQLite"), "{msg}");
            }
            ExtractError::Invalid => panic!("expected a Mismatch, got the generic Invalid"),
        }
    }

    #[test]
    fn an_archive_with_neither_entry_is_the_generic_invalid_error() {
        let extracted = tempfile::tempdir().unwrap();
        assert!(matches!(
            validate_archive_for_backend(extracted.path(), Backend::Sqlite).unwrap_err(),
            ExtractError::Invalid
        ));
        assert!(matches!(
            validate_archive_for_backend(extracted.path(), Backend::Postgres).unwrap_err(),
            ExtractError::Invalid
        ));
    }

    #[test]
    fn a_pgdump_entry_without_the_real_magic_header_is_rejected() {
        let extracted = tempfile::tempdir().unwrap();
        std::fs::write(extracted.path().join(MAIN_DB_ENTRY_PG), b"not a pgdump").unwrap();
        assert!(matches!(
            validate_archive_for_backend(extracted.path(), Backend::Postgres).unwrap_err(),
            ExtractError::Invalid
        ));
    }

    /// A Postgres restore doesn't swap the main database file (it was
    /// already restored in place by `pg_restore`) — only the logs
    /// database and local storage tree, exactly like `swap_data_dir`
    /// minus the main entry.
    #[tokio::test]
    async fn postgres_restore_swaps_only_the_logs_database_and_storage_tree() {
        let data_dir = tempfile::tempdir().unwrap();
        make_sqlite_file(
            &data_dir.path().join(cratebase_db::db::AUXILIARY_DB),
            "stale-logs",
        )
        .await;
        let storage_dir = data_dir.path().join(cratebase_storage::LOCAL_STORAGE_DIR);
        std::fs::create_dir_all(&storage_dir).unwrap();
        std::fs::write(storage_dir.join("old.txt"), b"old").unwrap();
        // A file that a main-db swap would touch but a side-files-only
        // swap must leave completely alone.
        let untouched = data_dir.path().join("cratebase.db");
        std::fs::write(&untouched, b"UNTOUCHED").unwrap();

        let restored = tempfile::tempdir().unwrap();
        write_pgdump_like_file(&restored.path().join(MAIN_DB_ENTRY_PG));
        make_sqlite_file(
            &restored.path().join(cratebase_db::db::AUXILIARY_DB),
            "new-logs",
        )
        .await;
        let restored_storage = restored.path().join(cratebase_storage::LOCAL_STORAGE_DIR);
        std::fs::create_dir_all(&restored_storage).unwrap();
        std::fs::write(restored_storage.join("new.txt"), b"new").unwrap();

        swap_side_files_only(data_dir.path(), restored.path()).unwrap();

        assert_eq!(
            read_marker(&data_dir.path().join(cratebase_db::db::AUXILIARY_DB)).await,
            "new-logs"
        );
        assert!(!storage_dir.join("old.txt").exists());
        assert_eq!(std::fs::read(storage_dir.join("new.txt")).unwrap(), b"new");
        assert_eq!(std::fs::read(&untouched).unwrap(), b"UNTOUCHED");
    }

    /// If a later file in the swap can't be written, every file already
    /// swapped in by an earlier one must be rolled back too — not just
    /// left in its new state.
    #[test]
    fn a_failed_swap_rolls_back_everything_already_swapped_in() {
        let live_dir = tempfile::tempdir().unwrap();
        let blocked_dir = tempfile::tempdir().unwrap();
        let source_dir = tempfile::tempdir().unwrap();

        let target0 = live_dir.path().join("live_a.db");
        std::fs::write(&target0, b"OLD_A").unwrap();
        let source0 = source_dir.path().join("a.db");
        std::fs::write(&source0, b"NEW_A").unwrap();

        // `nested` is a plain file, not a directory, so creating it as the
        // second target's parent is guaranteed to fail — a deterministic
        // stand-in for "the second file in a multi-file swap can't be
        // written", without relying on filesystem permissions (which
        // root, or a container running as root, can simply ignore).
        std::fs::write(blocked_dir.path().join("nested"), b"blocker").unwrap();
        let target1 = blocked_dir.path().join("nested").join("live_b.db");
        let source1 = source_dir.path().join("b.db");
        std::fs::write(&source1, b"NEW_B").unwrap();

        let swaps = vec![
            (source0.clone(), target0.clone()),
            (source1, target1.clone()),
        ];
        assert!(apply_swaps(&swaps).is_err());

        // The first file was fully swapped in before the second failed;
        // rolling back must undo it, not just stop where it broke.
        assert_eq!(std::fs::read(&target0).unwrap(), b"OLD_A");
        assert!(!target1.exists());
        assert!(!aside_path(&target0).exists(), "no leftover park file");
    }
}
