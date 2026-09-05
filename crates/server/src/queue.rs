//! `_queue_jobs`: a durable, retrying job queue built on the compile-time
//! [`crate::plugin::Plugin`] trait — a pg_boss-style worker, not a hosted
//! service. Toggle-gated the same way as `crate::teams`/`routes::llm`:
//! `settings.queue.enabled` defaults `false`, and `App::bootstrap` only
//! provisions `_queue_jobs` and registers [`QueuePlugin`] when it is
//! `true` (see that module's own gate for the exact wording).
//!
//! # Job lifecycle
//!
//! `pending` → `in_progress` → `completed`, or back to `pending` with a
//! bumped `attempts` and a pushed-out `runAfter` on failure, until
//! `attempts` reaches `maxAttempts`, at which point the job becomes
//! `failed` and is never retried again. [`tick`] — spawned on a fixed
//! interval from [`QueuePlugin::setup`] — does both halves of pg_boss's
//! own core guarantee every pass: [`reclaim_stale`] first (an
//! `in_progress` job whose `startedAt` is older than the stale timeout
//! means its worker crashed mid-run; it is put back to `pending` rather
//! than orphaned forever), then [`claim_next`] atomically claims and runs
//! one due job.
//!
//! # Claim atomicity
//!
//! [`claim_next`] runs inside [`crate::app::App::run_in_transaction`]:
//! select the earliest due `pending` row (`SELECT ... FOR UPDATE SKIP
//! LOCKED` on Postgres, so concurrent workers never race for the same
//! row and skip past ones another worker already holds; SQLite needs
//! nothing extra — `Engine::begin` already takes the single writer lock
//! up front, so the whole transaction is serialized against every other
//! writer), then an `UPDATE ... WHERE id = ? AND status = 'pending'` that
//! re-checks the status inside the same transaction rather than trusting
//! the row the `SELECT` just read. This is the exact defect a stale audit
//! of an earlier version of this queue flagged (a read-then-write claim
//! with no `WHERE status = 'pending'` on the write) — fixed here, for
//! real, rather than left as a known gap.
//!
//! # Handlers are compile-time, not dynamic
//!
//! There is no bytecode or script execution plane here (that is the
//! separate, sandboxed WASM plugin system) — a job's `queue` name is
//! looked up in an in-process handler registry
//! ([`QueueHandle::register_handler`]), populated by whatever Rust code
//! registered this plugin. A job whose `queue` has no registered handler
//! fails immediately with a descriptive `lastError`, retried and
//! eventually given up on like any other failure, rather than panicking
//! the worker loop.
//!
//! # Why raw SQL, not the Records API, for state transitions
//!
//! Claiming, completing and failing a job are all system-internal
//! bookkeeping with no caller-facing rule to enforce and no hook that
//! should refire on every retry — the same reasoning
//! `crate::cron_jobs`'s and `crate::webhooks`'s status write-backs give
//! for bypassing `records::update`. `POST /api/plugins/queue/enqueue`
//! *does* go through `cratebase_db::records::create` (see [`enqueue`]),
//! since that is the one write here that benefits from the Records
//! layer's id generation, timestamps and field validation.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use cratebase_core::{AppError, Collection, CollectionType, DateTime, Field, FieldKind, Record};
use cratebase_db::engine::Sql;
use cratebase_db::Executor;
use cratebase_filter::Dialect;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiResult};
use crate::plugin::Plugin;

pub const COLLECTION: &str = "_queue_jobs";

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_IN_PROGRESS: &str = "in_progress";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_FAILED: &str = "failed";

/// Used by [`enqueue`] when the caller omits `maxAttempts`.
const DEFAULT_MAX_ATTEMPTS: i64 = 5;

/// A job handler: takes the job's `payload` and resolves to `Ok(())` on
/// success or `Err(message)` on failure — a plain `String` rather than
/// `AppError`, since a handler has no HTTP response to shape and its
/// message is stored verbatim in `lastError`.
pub type HandlerFn =
    Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;

type HandlerMap = Arc<RwLock<HashMap<String, HandlerFn>>>;

/// A cheap, cloneable handle to a [`QueuePlugin`]'s handler registry,
/// usable after the plugin itself has been moved into
/// [`crate::app::App::register_plugin`] (which takes ownership). Obtain
/// one with [`QueuePlugin::handle`] *before* registering.
#[derive(Clone)]
pub struct QueueHandle(HandlerMap);

impl QueueHandle {
    /// Register (or replace) the handler for `queue`. Call this for every
    /// job name a binary's own code wants this queue to actually run —
    /// an unregistered name still enqueues fine, it just fails (and
    /// eventually gives up) every time [`tick`] tries to run it.
    pub fn register_handler<F, Fut>(&self, queue: impl Into<String>, handler: F)
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let wrapped: HandlerFn = Arc::new(move |payload| Box::pin(handler(payload)));
        self.0.write().insert(queue.into(), wrapped);
    }
}

/// Backoff/timing knobs, `Copy` so a reference to the owning
/// [`QueuePlugin`] doesn't need to outlive the spawned tick loop.
#[derive(Clone, Copy)]
struct TickConfig {
    base_delay: Duration,
    max_delay: Duration,
    stale_timeout: Duration,
}

pub struct QueuePlugin {
    handlers: HandlerMap,
    tick_interval: Duration,
    config: TickConfig,
}

impl Default for QueuePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl QueuePlugin {
    pub fn new() -> Self {
        QueuePlugin {
            handlers: Arc::new(RwLock::new(HashMap::new())),
            tick_interval: Duration::from_secs(1),
            config: TickConfig {
                base_delay: Duration::from_secs(1),
                max_delay: Duration::from_secs(300),
                stale_timeout: Duration::from_secs(300),
            },
        }
    }

    /// How often [`tick`] runs. Defaults to 1 second, matching
    /// `crate::cron::CronService`'s own ticker.
    pub fn with_tick_interval(mut self, d: Duration) -> Self {
        self.tick_interval = d;
        self
    }

    /// `base`/`max` for the exponential backoff a failed job's next
    /// `runAfter` is scheduled with: `min(2^attempts * base, max)`.
    pub fn with_backoff(mut self, base: Duration, max: Duration) -> Self {
        self.config.base_delay = base;
        self.config.max_delay = max;
        self
    }

    /// How long an `in_progress` job can run before [`reclaim_stale`]
    /// assumes its worker crashed and puts it back to `pending`.
    pub fn with_stale_timeout(mut self, d: Duration) -> Self {
        self.config.stale_timeout = d;
        self
    }

    /// A cloneable handle for registering handlers, safe to keep after
    /// this plugin is moved into `App::register_plugin`.
    pub fn handle(&self) -> QueueHandle {
        QueueHandle(self.handlers.clone())
    }
}

impl Plugin for QueuePlugin {
    fn name(&self) -> &str {
        "queue"
    }

    fn setup(&self, app: &App) -> Result<(), AppError> {
        // `_queue_jobs` is provisioned by `queue::ensure_collection`
        // *before* this plugin is registered (see `App::bootstrap_inner`)
        // rather than here: `setup()` is synchronous and provisioning a
        // collection is an async DB round trip, so there is nowhere to
        // `.await` it from this signature. All that's left to do here is
        // start the worker loop.
        let handlers = self.handlers.clone();
        let config = self.config;
        let tick_interval = self.tick_interval;
        let app = app.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tick_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                tick(&app, &handlers, config).await;
            }
        });
        Ok(())
    }

    fn routes(&self) -> Option<Router<App>> {
        Some(Router::new().route("/enqueue", post(enqueue)))
    }
}

/// Create `_queue_jobs` if it doesn't already exist. Idempotent — safe to
/// call every boot. Superuser-only end to end (every rule stays at
/// `Collection::new`'s default `None`), the same trust tier as
/// `_cron_jobs`/`_webhooks`: a queued job's payload is handed verbatim to
/// whatever handler its `queue` name resolves to, with no rule
/// enforcement in between.
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

fn text_field(name: &str) -> Field {
    Field::new(
        name,
        FieldKind::Text {
            min: 0,
            max: 0,
            pattern: String::new(),
            autogenerate_pattern: String::new(),
            primary_key: false,
        },
    )
}

fn build_collection() -> Collection {
    let mut c = Collection::new(COLLECTION, CollectionType::Base);
    c.system = true;

    let mut queue = text_field("queue");
    queue.system = true;
    queue.required = true;

    let mut payload = Field::new("payload", FieldKind::Json { max_size: 0 });
    payload.system = true;

    let mut status = text_field("status");
    status.system = true;
    status.required = true;

    let mut attempts = Field::new(
        "attempts",
        FieldKind::Number {
            min: Some(0.0),
            max: None,
            only_int: true,
        },
    );
    attempts.system = true;

    let mut max_attempts = Field::new(
        "maxAttempts",
        FieldKind::Number {
            min: Some(1.0),
            max: None,
            only_int: true,
        },
    );
    max_attempts.system = true;

    let mut run_after = Field::new(
        "runAfter",
        FieldKind::Date {
            min: None,
            max: None,
        },
    );
    run_after.system = true;

    let mut started_at = Field::new(
        "startedAt",
        FieldKind::Date {
            min: None,
            max: None,
        },
    );
    started_at.system = true;
    started_at.required = false;

    let mut last_error = text_field("lastError");
    last_error.required = false;

    let pos = c.fields.len() - 2;
    c.fields.splice(
        pos..pos,
        [
            queue,
            payload,
            status,
            attempts,
            max_attempts,
            run_after,
            started_at,
            last_error,
        ],
    );
    c.indexes = vec![
        "CREATE INDEX `idx_queue_jobs_status_runAfter` ON `_queue_jobs` (status, runAfter)".into(),
    ];
    c
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnqueueBody {
    queue: String,
    #[serde(default)]
    payload: Value,
    #[serde(default)]
    max_attempts: Option<i64>,
    #[serde(default)]
    run_after: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnqueueResponse {
    id: String,
    queue: String,
    status: String,
    run_after: String,
}

/// `POST /api/plugins/queue/enqueue` — superuser-only, same trust tier as
/// the collection itself (see [`ensure_collection`]'s doc comment).
async fn enqueue(
    State(app): State<App>,
    _auth: RequireSuperuser,
    Json(body): Json<EnqueueBody>,
) -> ApiResult<Json<EnqueueResponse>> {
    if body.queue.trim().is_empty() {
        return Err(ApiError::bad_request("queue must not be empty."));
    }
    let Some(collection) = app.db().collections.get_by_name(COLLECTION) else {
        return Err(ApiError::internal(
            "the queue plugin's collection is missing; is settings.queue.enabled set?",
        ));
    };
    let run_after = match body.run_after.as_deref() {
        Some(s) if !s.is_empty() => DateTime::parse(s)
            .ok_or_else(|| ApiError::bad_request("runAfter is not a valid date."))?,
        _ => DateTime::now(),
    };
    let max_attempts = body.max_attempts.unwrap_or(DEFAULT_MAX_ATTEMPTS).max(1);

    let mut record = Record::new(collection);
    record.set("queue", Value::String(body.queue.clone()));
    record.set("payload", body.payload);
    record.set("status", Value::String(STATUS_PENDING.to_string()));
    record.set("attempts", serde_json::json!(0));
    record.set("maxAttempts", serde_json::json!(max_attempts));
    record.set("runAfter", Value::String(run_after.to_pb_string()));

    cratebase_db::records::create(app.db(), &app.db().collections, &mut record)
        .await
        .map_err(AppError::from)
        .map_err(ApiError)?;

    Ok(Json(EnqueueResponse {
        id: record.id().to_string(),
        queue: body.queue,
        status: STATUS_PENDING.to_string(),
        run_after: run_after.to_pb_string(),
    }))
}

/// One claimed row, enough to run its handler and write the outcome back.
struct ClaimedJob {
    id: String,
    queue: String,
    payload: Value,
    attempts: i64,
    max_attempts: i64,
}

/// [`reclaim_stale`] then [`claim_next`] + run. Called on every tick of
/// the loop [`QueuePlugin::setup`] spawns.
async fn tick(app: &App, handlers: &HandlerMap, config: TickConfig) {
    reclaim_stale(app, config.stale_timeout).await;
    match claim_next(app).await {
        Ok(Some(job)) => run_job(app, job, handlers, config).await,
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "queue: failed to claim next job"),
    }
}

/// Put every `in_progress` job whose `startedAt` is older than
/// `stale_timeout` back to `pending` — a crashed worker's job is not
/// orphaned forever. Best-effort: an error here is logged, not
/// propagated, since the next tick tries again regardless.
async fn reclaim_stale(app: &App, stale_timeout: Duration) {
    let cutoff = DateTime::from_utc(
        DateTime::now().inner()
            - chrono::Duration::from_std(stale_timeout).unwrap_or(chrono::Duration::zero()),
    );
    let reclaimed = app
        .db()
        .execute(
            &format!(
                r#"UPDATE "{COLLECTION}" SET "status" = $1 WHERE "status" = $2 AND "startedAt" <= $3"#
            ),
            &[
                Sql::from(STATUS_PENDING),
                Sql::from(STATUS_IN_PROGRESS),
                Sql::from(cutoff.to_pb_string()),
            ],
        )
        .await;
    match reclaimed {
        Ok(0) => {}
        Ok(n) => tracing::info!(count = n, "queue: reclaimed stale in-progress job(s)"),
        Err(e) => tracing::warn!(error = %e, "queue: failed to reclaim stale jobs"),
    }
}

/// Atomically claim the earliest due `pending` job, if any. See this
/// module's doc comment for why the claim is race-free on both engines.
async fn claim_next(app: &App) -> Result<Option<ClaimedJob>, AppError> {
    app.run_in_transaction(|tx| async move {
        let now = DateTime::now().to_pb_string();
        let select_sql = match tx.dialect() {
            Dialect::Postgres => format!(
                r#"SELECT "id", "queue", "payload", "attempts", "maxAttempts" FROM "{COLLECTION}"
                   WHERE "status" = $1 AND "runAfter" <= $2
                   ORDER BY "runAfter" ASC LIMIT 1 FOR UPDATE SKIP LOCKED"#
            ),
            Dialect::Sqlite => format!(
                r#"SELECT "id", "queue", "payload", "attempts", "maxAttempts" FROM "{COLLECTION}"
                   WHERE "status" = $1 AND "runAfter" <= $2
                   ORDER BY "runAfter" ASC LIMIT 1"#
            ),
        };
        let Some(row) = tx
            .query_one(
                &select_sql,
                &[Sql::from(STATUS_PENDING), Sql::from(now.clone())],
            )
            .await
            .map_err(AppError::from)?
        else {
            return Ok(None);
        };
        let id = row.get_str("id").unwrap_or_default().to_string();
        let queue = row.get_str("queue").unwrap_or_default().to_string();
        let payload: Value = row
            .get_str("payload")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
        let attempts = row.get_i64("attempts").unwrap_or(0);
        let max_attempts = row.get_i64("maxAttempts").unwrap_or(DEFAULT_MAX_ATTEMPTS);

        let updated = tx
            .execute(
                &format!(
                    r#"UPDATE "{COLLECTION}" SET "status" = $1, "startedAt" = $2
                       WHERE "id" = $3 AND "status" = $4"#
                ),
                &[
                    Sql::from(STATUS_IN_PROGRESS),
                    Sql::from(now),
                    Sql::from(id.clone()),
                    Sql::from(STATUS_PENDING),
                ],
            )
            .await
            .map_err(AppError::from)?;
        if updated == 0 {
            // Lost a race with another worker between the select and the
            // update (only reachable on Postgres without the lock ever
            // being contended — e.g. a concurrent direct SQL write).
            return Ok(None);
        }
        Ok(Some(ClaimedJob {
            id,
            queue,
            payload,
            attempts,
            max_attempts,
        }))
    })
    .await
}

async fn run_job(app: &App, job: ClaimedJob, handlers: &HandlerMap, config: TickConfig) {
    let handler = handlers.read().get(&job.queue).cloned();
    let result = match handler {
        Some(h) => h(job.payload.clone()).await,
        None => Err(format!("no handler registered for queue '{}'", job.queue)),
    };
    match result {
        Ok(()) => mark_completed(app, &job.id).await,
        Err(err) => mark_failed_or_retry(app, &job, &err, config).await,
    }
}

async fn mark_completed(app: &App, id: &str) {
    if let Err(e) = app
        .db()
        .execute(
            &format!(r#"UPDATE "{COLLECTION}" SET "status" = $1 WHERE "id" = $2"#),
            &[Sql::from(STATUS_COMPLETED), Sql::from(id)],
        )
        .await
    {
        tracing::warn!(error = %e, job = %id, "queue: failed to mark job completed");
    }
}

/// `min(2^attempts * base, max)`, clamping the exponent so a very large
/// `attempts` can never overflow the multiplication.
fn backoff_delay(attempts: i64, base: Duration, max: Duration) -> Duration {
    let exp = attempts.clamp(0, 32) as u32;
    let factor = 2u64.saturating_pow(exp);
    let millis = (base.as_millis() as u64)
        .saturating_mul(factor)
        .min(max.as_millis() as u64);
    Duration::from_millis(millis)
}

async fn mark_failed_or_retry(app: &App, job: &ClaimedJob, error: &str, config: TickConfig) {
    let attempts = job.attempts + 1;
    let result = if attempts >= job.max_attempts {
        app.db()
            .execute(
                &format!(
                    r#"UPDATE "{COLLECTION}" SET "status" = $1, "attempts" = $2, "lastError" = $3
                       WHERE "id" = $4"#
                ),
                &[
                    Sql::from(STATUS_FAILED),
                    Sql::from(attempts),
                    Sql::from(error),
                    Sql::from(job.id.clone()),
                ],
            )
            .await
    } else {
        let delay = backoff_delay(attempts, config.base_delay, config.max_delay);
        let run_after = DateTime::from_utc(
            DateTime::now().inner()
                + chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::zero()),
        );
        app.db()
            .execute(
                &format!(
                    r#"UPDATE "{COLLECTION}" SET "status" = $1, "attempts" = $2, "lastError" = $3,
                       "runAfter" = $4 WHERE "id" = $5"#
                ),
                &[
                    Sql::from(STATUS_PENDING),
                    Sql::from(attempts),
                    Sql::from(error),
                    Sql::from(run_after.to_pb_string()),
                    Sql::from(job.id.clone()),
                ],
            )
            .await
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, job = %job.id, "queue: failed to write back job outcome");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::config::Config;

    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        ensure_collection(&app).await.expect("ensure collection");
        (app, dir)
    }

    async fn enqueue_job(app: &App, queue: &str, max_attempts: i64) -> String {
        let collection = app.db().collections.get_by_name(COLLECTION).unwrap();
        let mut record = Record::new(collection);
        record.set("queue", Value::String(queue.to_string()));
        record.set("payload", serde_json::json!({"n": 1}));
        record.set("status", Value::String(STATUS_PENDING.to_string()));
        record.set("attempts", serde_json::json!(0));
        record.set("maxAttempts", serde_json::json!(max_attempts));
        record.set("runAfter", Value::String(DateTime::now().to_pb_string()));
        cratebase_db::records::create(app.db(), &app.db().collections, &mut record)
            .await
            .expect("insert job");
        record.id().to_string()
    }

    async fn job_row(app: &App, id: &str) -> cratebase_db::engine::Row {
        app.db()
            .query_one(
                &format!(r#"SELECT * FROM "{COLLECTION}" WHERE "id" = $1"#),
                &[Sql::from(id)],
            )
            .await
            .expect("query job")
            .expect("job row present")
    }

    #[tokio::test]
    async fn a_pending_job_is_claimed_and_run_to_completion() {
        let (app, _dir) = test_app().await;
        let id = enqueue_job(&app, "greet", 5).await;

        let calls = Arc::new(AtomicUsize::new(0));
        let handlers: HandlerMap = Arc::new(RwLock::new(HashMap::new()));
        let counter = calls.clone();
        handlers.write().insert(
            "greet".into(),
            Arc::new(move |_payload| {
                let counter = counter.clone();
                Box::pin(async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }),
        );

        let config = TickConfig {
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
            stale_timeout: Duration::from_secs(300),
        };
        tick(&app, &handlers, config).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let row = job_row(&app, &id).await;
        assert_eq!(row.get_str("status"), Some(STATUS_COMPLETED));
    }

    #[tokio::test]
    async fn a_failing_job_retries_with_growing_backoff_then_gives_up() {
        let (app, _dir) = test_app().await;
        let id = enqueue_job(&app, "flaky", 3).await;

        let handlers: HandlerMap = Arc::new(RwLock::new(HashMap::new()));
        handlers.write().insert(
            "flaky".into(),
            Arc::new(
                |_payload: Value| -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
                    Box::pin(async move { Err("boom".to_string()) })
                },
            ) as HandlerFn,
        );

        let config = TickConfig {
            base_delay: Duration::from_millis(15),
            max_delay: Duration::from_secs(5),
            stale_timeout: Duration::from_secs(300),
        };

        // Attempt 1: fails, scheduled ~30ms out (2^1 * 15ms), still pending.
        tick(&app, &handlers, config).await;
        let row = job_row(&app, &id).await;
        assert_eq!(row.get_str("status"), Some(STATUS_PENDING));
        assert_eq!(row.get_i64("attempts"), Some(1));
        assert_eq!(row.get_str("lastError"), Some("boom"));
        let run_after_1 = row.get_str("runAfter").unwrap().to_string();

        // Immediately retrying finds nothing due yet — respects backoff.
        tick(&app, &handlers, config).await;
        let row = job_row(&app, &id).await;
        assert_eq!(row.get_i64("attempts"), Some(1));

        tokio::time::sleep(Duration::from_millis(40)).await;

        // Attempt 2: fails again, scheduled further out (2^2 * 15ms = 60ms).
        tick(&app, &handlers, config).await;
        let row = job_row(&app, &id).await;
        assert_eq!(row.get_str("status"), Some(STATUS_PENDING));
        assert_eq!(row.get_i64("attempts"), Some(2));
        let run_after_2 = row.get_str("runAfter").unwrap().to_string();
        assert!(
            run_after_2 > run_after_1,
            "backoff should push runAfter further out on each failure"
        );

        tokio::time::sleep(Duration::from_millis(70)).await;

        // Attempt 3 == maxAttempts: gives up for good.
        tick(&app, &handlers, config).await;
        let row = job_row(&app, &id).await;
        assert_eq!(row.get_str("status"), Some(STATUS_FAILED));
        assert_eq!(row.get_i64("attempts"), Some(3));
    }

    #[tokio::test]
    async fn a_stale_in_progress_job_is_reclaimed_and_can_still_complete() {
        let (app, _dir) = test_app().await;
        let id = enqueue_job(&app, "crash-prone", 5).await;

        // Simulate a worker that claimed the job and then crashed: mark it
        // in_progress with a startedAt well in the past.
        let long_ago = DateTime::from_utc(DateTime::now().inner() - chrono::Duration::hours(1));
        app.db()
            .execute(
                &format!(
                    r#"UPDATE "{COLLECTION}" SET "status" = $1, "startedAt" = $2 WHERE "id" = $3"#
                ),
                &[
                    Sql::from(STATUS_IN_PROGRESS),
                    Sql::from(long_ago.to_pb_string()),
                    Sql::from(id.clone()),
                ],
            )
            .await
            .expect("simulate crash");

        let handlers: HandlerMap = Arc::new(RwLock::new(HashMap::new()));
        handlers.write().insert(
            "crash-prone".into(),
            Arc::new(
                |_payload| -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
                    Box::pin(async move { Ok(()) })
                },
            ) as HandlerFn,
        );

        let config = TickConfig {
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
            stale_timeout: Duration::from_secs(60),
        };

        // One tick both reclaims (stale_timeout of 60s << 1 hour stuck) and
        // then immediately claims + runs the now-pending job.
        tick(&app, &handlers, config).await;

        let row = job_row(&app, &id).await;
        assert_eq!(row.get_str("status"), Some(STATUS_COMPLETED));
    }

    #[tokio::test]
    async fn a_job_with_no_registered_handler_fails_with_a_descriptive_error() {
        let (app, _dir) = test_app().await;
        let id = enqueue_job(&app, "nobody-home", 1).await;

        let handlers: HandlerMap = Arc::new(RwLock::new(HashMap::new()));
        let config = TickConfig {
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
            stale_timeout: Duration::from_secs(300),
        };
        tick(&app, &handlers, config).await;

        let row = job_row(&app, &id).await;
        assert_eq!(row.get_str("status"), Some(STATUS_FAILED));
        assert!(row
            .get_str("lastError")
            .unwrap()
            .contains("no handler registered"));
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(60);
        assert_eq!(backoff_delay(1, base, max), Duration::from_secs(2));
        assert_eq!(backoff_delay(2, base, max), Duration::from_secs(4));
        assert_eq!(backoff_delay(3, base, max), Duration::from_secs(8));
        assert_eq!(backoff_delay(10, base, max), max);
    }

    #[test]
    fn queue_handle_registers_reachable_from_tick() {
        let plugin = QueuePlugin::new();
        let handle = plugin.handle();
        handle.register_handler("noop", |_payload| async move { Ok(()) });
        assert!(plugin.handlers.read().contains_key("noop"));
    }
}
