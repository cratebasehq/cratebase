//! `_zip_exports`: a generic "ZIP up every file matching a filter" job,
//! built on the compile-time [`crate::plugin::Plugin`] trait. Toggle-gated
//! the same way as `crate::queue`/`crate::teams`: `settings.zipExport.enabled`
//! defaults `false`, and `App::bootstrap` only provisions `_zip_exports` and
//! registers [`ZipExportPlugin`] when it is `true`.
//!
//! # Why this exists as a built-in plugin, not app-specific code
//!
//! "Let a user download every file matching some query as one ZIP" is a
//! common enough need (event photo galleries, per-tenant document bundles,
//! monthly report archives) that it earns a place next to the Queue plugin
//! rather than being reimplemented per downstream app. It is deliberately
//! generic: the job row names a *source collection*, a *filter expression*,
//! and a *file field* — it has no idea what a "photo" or a "guest" is.
//!
//! # Why a Rust plugin and not a `pb_hooks` script
//!
//! Building a ZIP archive is not itself something `$os.exec` is needed for
//! (unlike, say, video rendering), but a JS hook has no primitive for
//! streaming an arbitrary number of storage objects into a single archive
//! and attaching multi-megabyte output back onto a record — that plumbing
//! (temp files, the `zip` crate, `Storage::get`/`put`, `records::update_with_uploads`)
//! only exists on the Rust side.
//!
//! # Job lifecycle
//!
//! `pending` → `running` → `done`, or `failed` with `error` set. Unlike the
//! Queue plugin this has no retry/backoff: a ZIP export is a request for a
//! fresh snapshot, not a fire-and-forget background task with a business
//! reason to retry automatically — a failure surfaces immediately and the
//! caller decides whether to enqueue a new one.
//!
//! # Claim atomicity
//!
//! [`claim_next`] uses the exact same transactional claim as
//! `crate::queue::claim_next`: `SELECT ... FOR UPDATE SKIP LOCKED` on
//! Postgres, then an `UPDATE ... WHERE status = 'pending'` inside the same
//! transaction so a concurrent worker can never double-claim a row.
//!
//! # Entry naming and path-traversal
//!
//! `nameField`'s value (arbitrary end-user input in most real uses — a
//! guest's display name, a customer's company name) becomes a real
//! filesystem path component the moment someone unzips the result.
//! [`sanitize_zip_name`] strips anything outside letters/digits/space/dash
//! rather than escaping it, matching the same reasoning PocketBase-style
//! filename sanitizers use elsewhere in this codebase.

use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use cratebase_core::{AppError, Collection, CollectionType, DateTime, Field, FieldKind, Record};
use cratebase_db::engine::Sql;
use cratebase_db::{records, Executor, ListParams, RequestContext};
use cratebase_filter::Dialect;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiResult};
use crate::plugin::Plugin;

pub const COLLECTION: &str = "_zip_exports";

const STATUS_PENDING: &str = "pending";
const STATUS_RUNNING: &str = "running";
const STATUS_DONE: &str = "done";
const STATUS_FAILED: &str = "failed";

/// How long a `running` row can go without finishing before a fresh tick
/// assumes its worker crashed and reclaims it back to `pending` — the same
/// crash-recovery guarantee `crate::queue` gives, sized for "zipping files"
/// rather than "an arbitrary background job".
const STALE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct ZipExportPlugin {
    tick_interval: Duration,
}

impl ZipExportPlugin {
    pub fn new() -> Self {
        ZipExportPlugin {
            tick_interval: Duration::from_secs(2),
        }
    }

    pub fn with_tick_interval(mut self, d: Duration) -> Self {
        self.tick_interval = d;
        self
    }
}

impl Default for ZipExportPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for ZipExportPlugin {
    fn name(&self) -> &str {
        "zip-export"
    }

    fn setup(&self, app: &App) -> Result<(), AppError> {
        let app = app.clone();
        let tick_interval = self.tick_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tick_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                tick(&app).await;
            }
        });
        Ok(())
    }

    fn routes(&self) -> Option<Router<App>> {
        Some(Router::new().route("/enqueue", post(enqueue)))
    }
}

/// Create `_zip_exports` if it doesn't already exist. Idempotent — safe to
/// call every boot. Superuser-only end to end, the same trust tier as
/// `_queue_jobs`: `filter`/`sourceCollection` are operator/app-code-supplied
/// query parameters, not something a non-superuser record write should ever
/// reach (a filter is arbitrary read access into another collection, rules
/// or not).
pub async fn ensure_collection(app: &App) -> Result<(), AppError> {
    if app.db().collections.get_by_name(COLLECTION).is_some() {
        return Ok(());
    }
    let collection = build_collection();
    app.db()
        .collections
        .insert(app.db().engine.as_ref(), &collection)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

fn text_field(name: &str, required: bool) -> Field {
    let mut f = Field::new(
        name,
        FieldKind::Text {
            min: 0,
            max: 0,
            pattern: String::new(),
            autogenerate_pattern: String::new(),
            primary_key: false,
        },
    );
    f.system = true;
    f.required = required;
    f
}

fn build_collection() -> Collection {
    let mut c = Collection::new(COLLECTION, CollectionType::Base);
    c.system = true;

    let source_collection = text_field("sourceCollection", true);
    let filter = text_field("filter", false);
    let file_field = text_field("fileField", false);
    let name_field = text_field("nameField", false);
    let sort = text_field("sort", false);

    let status = text_field("status", true);

    let mut output_file = Field::new(
        "outputFile",
        FieldKind::File {
            max_select: 1,
            max_size: 0,
            mime_types: vec!["application/zip".to_string()],
            thumbs: vec![],
            protected: false,
        },
    );
    output_file.required = false;

    let mut bytes = Field::new(
        "bytes",
        FieldKind::Number {
            min: Some(0.0),
            max: None,
            only_int: true,
        },
    );
    bytes.system = true;

    let mut source_count = Field::new(
        "sourceCount",
        FieldKind::Number {
            min: Some(0.0),
            max: None,
            only_int: true,
        },
    );
    source_count.system = true;

    let mut error = text_field("error", false);
    error.required = false;

    let mut started_at = Field::new(
        "startedAt",
        FieldKind::Date {
            min: None,
            max: None,
        },
    );
    started_at.required = false;

    let mut completed_at = Field::new(
        "completedAt",
        FieldKind::Date {
            min: None,
            max: None,
        },
    );
    completed_at.required = false;

    let pos = c.fields.len() - 2;
    c.fields.splice(
        pos..pos,
        [
            source_collection,
            filter,
            file_field,
            name_field,
            sort,
            status,
            output_file,
            bytes,
            source_count,
            error,
            started_at,
            completed_at,
        ],
    );
    c.indexes = vec![format!(
        "CREATE INDEX `idx_zip_exports_status` ON `{COLLECTION}` (status)"
    )];
    c
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnqueueBody {
    source_collection: String,
    filter: String,
    #[serde(default)]
    file_field: Option<String>,
    #[serde(default)]
    name_field: Option<String>,
    #[serde(default)]
    sort: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnqueueResponse {
    id: String,
    status: String,
}

/// `POST /api/plugins/zip-export/enqueue` — superuser-only (see
/// [`ensure_collection`]'s doc comment for why).
async fn enqueue(
    State(app): State<App>,
    _auth: RequireSuperuser,
    Json(body): Json<EnqueueBody>,
) -> ApiResult<Json<EnqueueResponse>> {
    if body.source_collection.trim().is_empty() {
        return Err(ApiError::bad_request("sourceCollection must not be empty."));
    }
    if app
        .db()
        .collections
        .get_by_name(&body.source_collection)
        .is_none()
    {
        return Err(ApiError::bad_request("sourceCollection does not exist."));
    }
    let Some(collection) = app.db().collections.get_by_name(COLLECTION) else {
        return Err(ApiError::internal(
            "the zip-export plugin's collection is missing; is settings.zipExport.enabled set?",
        ));
    };

    let mut record = Record::new(collection);
    record.set("sourceCollection", Value::String(body.source_collection));
    record.set("filter", Value::String(body.filter));
    record.set(
        "fileField",
        Value::String(body.file_field.unwrap_or_else(|| "file".to_string())),
    );
    record.set(
        "nameField",
        Value::String(body.name_field.unwrap_or_default()),
    );
    record.set("sort", Value::String(body.sort.unwrap_or_default()));
    record.set("status", Value::String(STATUS_PENDING.to_string()));

    records::create(app.db(), &app.db().collections, &mut record)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(EnqueueResponse {
        id: record.id().to_string(),
        status: STATUS_PENDING.to_string(),
    }))
}

/// One claimed row, enough to run the render and write the outcome back.
struct ClaimedExport {
    id: String,
    source_collection: String,
    filter: String,
    file_field: String,
    name_field: String,
    sort: String,
}

async fn tick(app: &App) {
    reclaim_stale(app).await;
    match claim_next(app).await {
        Ok(Some(job)) => run_export(app, job).await,
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "zip-export: failed to claim next job"),
    }
}

/// Put every `running` row whose `startedAt` is older than [`STALE_TIMEOUT`]
/// back to `pending` — a crashed worker's export is not orphaned forever.
async fn reclaim_stale(app: &App) {
    let cutoff = DateTime::from_utc(
        DateTime::now().inner() - chrono::Duration::from_std(STALE_TIMEOUT).unwrap_or_default(),
    );
    let result = app
        .db()
        .execute(
            &format!(
                r#"UPDATE "{COLLECTION}" SET "status" = $1 WHERE "status" = $2 AND "startedAt" < $3"#
            ),
            &[
                Sql::from(STATUS_PENDING),
                Sql::from(STATUS_RUNNING),
                Sql::from(cutoff.to_pb_string()),
            ],
        )
        .await;
    if let Err(e) = result {
        tracing::warn!(error = %e, "zip-export: failed to reclaim stale rows");
    }
}

/// Atomically claim the earliest `pending` row, if any. Same
/// `SELECT ... FOR UPDATE SKIP LOCKED` + re-checked `UPDATE` pattern as
/// `crate::queue::claim_next` — see this module's doc comment.
async fn claim_next(app: &App) -> Result<Option<ClaimedExport>, AppError> {
    app.run_in_transaction(|tx| async move {
        let now = DateTime::now().to_pb_string();
        let select_sql = match tx.dialect() {
            Dialect::Postgres => format!(
                r#"SELECT "id", "sourceCollection", "filter", "fileField", "nameField", "sort"
                   FROM "{COLLECTION}" WHERE "status" = $1
                   ORDER BY "created" ASC LIMIT 1 FOR UPDATE SKIP LOCKED"#
            ),
            Dialect::Sqlite => format!(
                r#"SELECT "id", "sourceCollection", "filter", "fileField", "nameField", "sort"
                   FROM "{COLLECTION}" WHERE "status" = $1
                   ORDER BY "created" ASC LIMIT 1"#
            ),
        };
        let Some(row) = tx
            .query_one(&select_sql, &[Sql::from(STATUS_PENDING)])
            .await
            .map_err(AppError::from)?
        else {
            return Ok(None);
        };
        let id = row.get_str("id").unwrap_or_default().to_string();

        let updated = tx
            .execute(
                &format!(
                    r#"UPDATE "{COLLECTION}" SET "status" = $1, "startedAt" = $2
                       WHERE "id" = $3 AND "status" = $4"#
                ),
                &[
                    Sql::from(STATUS_RUNNING),
                    Sql::from(now),
                    Sql::from(id.clone()),
                    Sql::from(STATUS_PENDING),
                ],
            )
            .await
            .map_err(AppError::from)?;
        if updated == 0 {
            return Ok(None);
        }
        Ok(Some(ClaimedExport {
            id,
            source_collection: row
                .get_str("sourceCollection")
                .unwrap_or_default()
                .to_string(),
            filter: row.get_str("filter").unwrap_or_default().to_string(),
            file_field: row.get_str("fileField").unwrap_or_default().to_string(),
            name_field: row.get_str("nameField").unwrap_or_default().to_string(),
            sort: row.get_str("sort").unwrap_or_default().to_string(),
        }))
    })
    .await
}

async fn run_export(app: &App, job: ClaimedExport) {
    let id = job.id.clone();
    match render_export(app, &job).await {
        Ok((bytes, count)) => {
            if let Err(e) = mark_done(app, &id, bytes, count).await {
                tracing::warn!(id = %id, error = %e, "zip-export: failed to mark done");
            }
        }
        Err(message) => {
            if let Err(e) = mark_failed(app, &id, &message).await {
                tracing::warn!(id = %id, error = %e, "zip-export: failed to mark failed");
            }
        }
    }
}

/// Strip anything outside letters/digits/space/dash and truncate — the
/// same "delete, don't escape" reasoning GetMoment's own `zipEntryName`
/// TypeScript port uses: a ZIP entry name becomes a real filesystem path
/// the moment someone unzips the result, and this value usually comes
/// from untrusted end-user input (a display name, a company name, ...).
fn sanitize_zip_name(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .collect();
    let trimmed = cleaned.trim();
    let collapsed = trimmed.split_whitespace().collect::<Vec<_>>().join("-");
    collapsed.chars().take(60).collect()
}

fn extension_of(filename: &str) -> &str {
    Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
}

async fn render_export(app: &App, job: &ClaimedExport) -> Result<(i64, i64), String> {
    let db = app.db();
    let store = &db.collections;

    let source = store
        .get(&job.source_collection)
        .ok_or_else(|| format!("source collection '{}' not found", job.source_collection))?;

    let ctx = RequestContext::superuser();
    let list = records::list(
        db,
        store,
        &ctx,
        &source,
        ListParams {
            page: 1,
            per_page: 10_000,
            sort: (!job.sort.is_empty()).then_some(job.sort.as_str()),
            filter: (!job.filter.is_empty()).then_some(job.filter.as_str()),
            expand: None,
            skip_total: true,
        },
    )
    .await
    .map_err(|e| format!("failed to query '{}': {e}", job.source_collection))?;

    if list.items.is_empty() {
        return Err("no records matched the export filter".to_string());
    }

    let work_dir = std::env::temp_dir().join(format!("cratebase-zip-export-{}", job.id));
    tokio::fs::create_dir_all(&work_dir)
        .await
        .map_err(|e| format!("failed to create temp dir: {e}"))?;
    let cleanup = TempDirGuard(work_dir.clone());

    let storage = app.storage();
    let zip_path = work_dir.join("export.zip");
    let field = job.file_field.clone();
    let name_field = job.name_field.clone();
    let source_id = source.id.clone();

    let mut downloaded: Vec<(PathBuf, String)> = Vec::with_capacity(list.items.len());
    for (index, record) in list.items.iter().enumerate() {
        let filename = record.get_string(&field);
        if filename.is_empty() {
            continue;
        }
        let key = format!("{}/{}/{}", source_id, record.id(), filename);
        let bytes = storage
            .get(&key)
            .await
            .map_err(|e| format!("failed to read '{key}': {e}"))?;
        let ext = extension_of(&filename);
        let base_name = if name_field.is_empty() {
            String::new()
        } else {
            sanitize_zip_name(&record.get_string(&name_field))
        };
        let short_id: String = record.id().chars().take(8).collect();
        let entry_name = if base_name.is_empty() {
            format!("{:04}-{short_id}.{ext}", index + 1)
        } else {
            format!("{:04}-{base_name}-{short_id}.{ext}", index + 1)
        };
        let local_path = work_dir.join(format!("src-{index:05}"));
        tokio::fs::write(&local_path, &bytes)
            .await
            .map_err(|e| format!("failed to write temp file: {e}"))?;
        downloaded.push((local_path, entry_name));
    }

    if downloaded.is_empty() {
        return Err("matched records had no file in the configured field".to_string());
    }
    let source_count = downloaded.len() as i64;

    let zip_path_for_blocking = zip_path.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let file = std::fs::File::create(&zip_path_for_blocking)
            .map_err(|e| format!("failed to create zip file: {e}"))?;
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (path, entry_name) in &downloaded {
            zip.start_file(entry_name.as_str(), options)
                .map_err(|e| format!("failed to start zip entry '{entry_name}': {e}"))?;
            let mut reader = std::fs::File::open(path)
                .map_err(|e| format!("failed to reopen temp file: {e}"))?;
            std::io::copy(&mut reader, &mut zip)
                .map_err(|e| format!("failed to write zip entry '{entry_name}': {e}"))?;
        }
        zip.finish()
            .map_err(|e| format!("failed to finalize zip: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("zip task panicked: {e}"))??;

    let zip_bytes = tokio::fs::read(&zip_path)
        .await
        .map_err(|e| format!("failed to read finished zip: {e}"))?;
    let byte_count = zip_bytes.len() as i64;

    let filename = format!("export-{}.zip", DateTime::now().inner().timestamp_millis());
    let export_collection = store
        .get(COLLECTION)
        .ok_or("_zip_exports collection missing")?;
    let storage_key = format!("{}/{}/{}", export_collection.id, job.id, filename);
    storage
        .put(&storage_key, zip_bytes.clone().into())
        .await
        .map_err(|e| format!("failed to store zip: {e}"))?;

    let mut record = records::find_by_id_raw(db, &export_collection, &job.id)
        .await
        .map_err(|e| format!("failed to reload export row: {e}"))?;
    let previous_file = record.get_string("outputFile");

    record.set("status", json!(STATUS_DONE));
    record.set("outputFile", json!(filename.clone()));
    record.set("bytes", json!(byte_count));
    record.set("sourceCount", json!(source_count));
    record.set("error", json!(""));
    record.set("completedAt", json!(DateTime::now().to_pb_string()));

    let upload = cratebase_db::UploadMeta {
        field: "outputFile".to_string(),
        name: filename.clone(),
        size: byte_count,
        mime: "application/zip".to_string(),
    };
    records::update_with_uploads(db, store, &mut record, std::slice::from_ref(&upload))
        .await
        .map_err(|e| format!("failed to save export row: {e}"))?;

    if !previous_file.is_empty() && previous_file != filename {
        let old_key = format!("{}/{}/{}", export_collection.id, job.id, previous_file);
        if let Err(e) = storage.delete(&old_key).await {
            tracing::warn!(key = %old_key, error = %e, "zip-export: failed to remove previous output");
        }
    }

    drop(cleanup);
    Ok((byte_count, source_count))
}

async fn mark_done(app: &App, id: &str, _bytes: i64, _count: i64) -> Result<(), AppError> {
    // Terminal state is already written by `render_export` (it needs the
    // Records API for the file upload, unlike `mark_failed`'s plain SQL
    // below) — this only exists so `run_export`'s match arms stay
    // symmetric and any future post-success hook has one place to land.
    let _ = (app, id);
    Ok(())
}

async fn mark_failed(app: &App, id: &str, message: &str) -> Result<(), AppError> {
    let truncated: String = message.chars().take(2_000).collect();
    app.db()
        .execute(
            &format!(r#"UPDATE "{COLLECTION}" SET "status" = $1, "error" = $2 WHERE "id" = $3"#),
            &[
                Sql::from(STATUS_FAILED),
                Sql::from(truncated),
                Sql::from(id),
            ],
        )
        .await
        .map_err(AppError::from)?;
    Ok(())
}

/// Best-effort recursive temp-dir cleanup on drop.
struct TempDirGuard(PathBuf);
impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        ensure_collection(&app).await.expect("ensure collection");
        (app, dir)
    }

    #[test]
    fn sanitize_zip_name_strips_path_traversal() {
        assert_eq!(sanitize_zip_name("../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_zip_name("  Jane   Doe  "), "Jane-Doe");
        assert_eq!(sanitize_zip_name("Müller & Co."), "Mller-Co");
        assert_eq!(sanitize_zip_name(""), "");
    }

    #[test]
    fn sanitize_zip_name_truncates() {
        let long = "a".repeat(200);
        assert_eq!(sanitize_zip_name(&long).len(), 60);
    }

    #[tokio::test]
    async fn ensure_collection_is_idempotent() {
        let (app, _dir) = test_app().await;
        ensure_collection(&app).await.expect("second call");
        assert!(app.db().collections.get_by_name(COLLECTION).is_some());
    }

    #[tokio::test]
    async fn claim_next_returns_none_when_empty() {
        let (app, _dir) = test_app().await;
        let claimed = claim_next(&app).await.expect("claim");
        assert!(claimed.is_none());
    }

    #[tokio::test]
    async fn claim_next_claims_and_marks_running() {
        let (app, _dir) = test_app().await;
        let collection = app.db().collections.get_by_name(COLLECTION).unwrap();
        let mut record = Record::new(collection);
        record.set("sourceCollection", Value::String("media".to_string()));
        record.set("filter", Value::String(String::new()));
        record.set("fileField", Value::String("file".to_string()));
        record.set("nameField", Value::String(String::new()));
        record.set("sort", Value::String(String::new()));
        record.set("status", Value::String(STATUS_PENDING.to_string()));
        records::create(app.db(), &app.db().collections, &mut record)
            .await
            .expect("insert");

        let claimed = claim_next(&app).await.expect("claim").expect("some job");
        assert_eq!(claimed.source_collection, "media");

        // A second claim attempt must find nothing else pending.
        let second = claim_next(&app).await.expect("claim");
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn render_export_fails_clearly_on_missing_source_collection() {
        let (app, _dir) = test_app().await;
        let job = ClaimedExport {
            id: "does-not-matter".to_string(),
            source_collection: "does_not_exist".to_string(),
            filter: String::new(),
            file_field: "file".to_string(),
            name_field: String::new(),
            sort: String::new(),
        };
        let err = render_export(&app, &job).await.unwrap_err();
        assert!(err.contains("not found"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn render_export_fails_clearly_on_empty_result_set() {
        let (app, _dir) = test_app().await;
        // `users` is a real system collection with no rows in a fresh app.
        let job = ClaimedExport {
            id: "does-not-matter".to_string(),
            source_collection: "users".to_string(),
            filter: String::new(),
            file_field: "avatar".to_string(),
            name_field: String::new(),
            sort: String::new(),
        };
        let err = render_export(&app, &job).await.unwrap_err();
        assert!(
            err.contains("no records matched"),
            "unexpected error: {err}"
        );
    }
}
