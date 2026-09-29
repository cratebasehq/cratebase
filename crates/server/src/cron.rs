//! The cron scheduler behind `GET /api/crons` and `POST /api/crons/{id}`.
//!
//! # Design
//!
//! One tokio task ticks every second and fires the jobs whose expression
//! matches the current *minute*; a `(job id, minute)` guard means a job
//! never runs twice within the same minute even though the ticker visits
//! that minute sixty times. Each due job is `tokio::spawn`ed rather than
//! awaited inline, so a slow job (a backup of a multi-GB database, say)
//! cannot delay every other job — and a panicking job is caught by the
//! join handle and logged instead of killing the scheduler.
//!
//! Ids are PocketBase's, verbatim (`__pbDBOptimize__`, ...), because the
//! dashboard and the `crons` API fixture list them by id.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Timelike, Utc};
use cratebase_core::AppError;
use croner::Cron;

/// The canonical timestamp for one invocation of a job: for a real
/// scheduled tick, `now` (whatever wall-clock value drove that tick)
/// floored to the minute — identical on every node that reaches the same
/// scheduled minute, which is what lets `crate::cron_history` claim a
/// tick durably instead of racing an in-transaction lock. For an
/// on-demand [`CronService::run`] call it is simply "now", full
/// precision, so triggering a job manually never collides with a
/// scheduled tick or an earlier manual trigger in the same minute.
pub type TickAt = DateTime<Utc>;

/// A job body. Boxed and shared so `run(id)` can fire the same closure
/// the scheduler uses.
pub type JobFn = Arc<dyn Fn(TickAt) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// System job ids, kept identical to PocketBase's.
pub const JOB_DB_OPTIMIZE: &str = "__pbDBOptimize__";
pub const JOB_MFA_CLEANUP: &str = "__pbMFACleanup__";
pub const JOB_OTP_CLEANUP: &str = "__pbOTPCleanup__";
/// Purges `_totps` rows still `pending` (never confirmed) an hour after
/// `.../totp/setup` created them — a confirmed row is never touched by
/// this job, only an abandoned setup attempt.
pub const JOB_TOTP_CLEANUP: &str = "__cbTOTPCleanup__";
pub const JOB_LOGS_CLEANUP: &str = "__pbLogsCleanup__";
/// The scheduled backup. Unlike the four above it is only registered once
/// `settings.backups.cron` is non-empty, so `GET /api/crons` lists it only
/// then — PocketBase behaves the same way.
pub const JOB_AUTO_BACKUP: &str = "__pbAutoBackup__";
/// Hourly `_sessions` sweep (`crate::sessions::sweep_expired`). Cratebase-only
/// — the four ids above are PocketBase's, verbatim.
pub const JOB_SESSION_SWEEP: &str = "__cbSessionSweep__";
/// `_mailLog` retention (`settings.logs.mailLogMaxDays`), same cadence as
/// [`JOB_LOGS_CLEANUP`]. Cratebase-only.
pub const JOB_MAIL_LOG_CLEANUP: &str = "__cbMailLogCleanup__";
/// `_notifications` retention (`settings.notifications.retentionDays`),
/// same cadence as [`JOB_LOGS_CLEANUP`]/[`JOB_MAIL_LOG_CLEANUP`] — only
/// *read* rows are pruned, see `cratebase_core::settings::Notifications`'s
/// doc comment. Cratebase-only.
pub const JOB_NOTIFICATIONS_CLEANUP: &str = "__cbNotificationsCleanup__";
/// Hourly `_pendingUploads` sweep (`crate::presign::sweep_expired`):
/// drops presigned-upload tickets that expired without being claimed,
/// and best-effort deletes the object they reserved. Cratebase-only.
pub const JOB_PENDING_UPLOADS_SWEEP: &str = "__cbPendingUploadsSweep__";

#[derive(Clone)]
struct Job {
    id: String,
    expression: String,
    schedule: Arc<Cron>,
    func: JobFn,
}

/// One row of `GET /api/crons`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CronJob {
    pub id: String,
    pub expression: String,
}

#[derive(Default)]
struct Inner {
    /// Insertion-ordered so `list()` is stable; PocketBase's own order is
    /// registration order and the fixture relies on it.
    order: Vec<String>,
    jobs: HashMap<String, Job>,
    /// Minute (unix minutes) each job last fired in, the dedupe guard.
    last_run: HashMap<String, i64>,
}

/// `App::cron()`. Cheap to clone; every clone shares one registry.
#[derive(Clone, Default)]
pub struct CronService {
    inner: Arc<RwLock<Inner>>,
    started: Arc<std::sync::atomic::AtomicBool>,
}

impl CronService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) a job. Returns a validation error when the
    /// expression is not a valid 5-field cron expression.
    pub fn add<F, Fut>(
        &self,
        id: impl Into<String>,
        expression: &str,
        func: F,
    ) -> Result<(), AppError>
    where
        F: Fn(TickAt) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let func: JobFn = Arc::new(move |tick_at| Box::pin(func(tick_at)));
        self.add_boxed(id, expression, func)
    }

    pub fn add_boxed(
        &self,
        id: impl Into<String>,
        expression: &str,
        func: JobFn,
    ) -> Result<(), AppError> {
        let schedule = expression
            .parse::<Cron>()
            .map_err(|e| AppError::bad_request(format!("invalid cron expression: {e}")))?;
        let id = id.into();
        let mut inner = self.inner.write().expect("cron registry poisoned");
        if !inner.jobs.contains_key(&id) {
            inner.order.push(id.clone());
        }
        inner.jobs.insert(
            id.clone(),
            Job {
                id,
                expression: expression.to_string(),
                schedule: Arc::new(schedule),
                func,
            },
        );
        Ok(())
    }

    pub fn remove(&self, id: &str) -> bool {
        let mut inner = self.inner.write().expect("cron registry poisoned");
        inner.order.retain(|j| j != id);
        inner.last_run.remove(id);
        inner.jobs.remove(id).is_some()
    }

    pub fn remove_all(&self) {
        let mut inner = self.inner.write().expect("cron registry poisoned");
        inner.order.clear();
        inner.jobs.clear();
        inner.last_run.clear();
    }

    /// `GET /api/crons`: `[{id, expression}]` in registration order.
    pub fn list(&self) -> Vec<CronJob> {
        let inner = self.inner.read().expect("cron registry poisoned");
        inner
            .order
            .iter()
            .filter_map(|id| inner.jobs.get(id))
            .map(|j| CronJob {
                id: j.id.clone(),
                expression: j.expression.clone(),
            })
            .collect()
    }

    pub fn has(&self, id: &str) -> bool {
        self.inner
            .read()
            .expect("cron registry poisoned")
            .jobs
            .contains_key(id)
    }

    /// `POST /api/crons/{id}`: run the job now, inline, so the caller
    /// learns whether it panicked. Its tick is "now", full precision —
    /// see [`TickAt`]'s doc for why that's deliberately not floored to
    /// the minute the way a real scheduled tick is.
    pub async fn run(&self, id: &str) -> Result<(), AppError> {
        let func = {
            let inner = self.inner.read().expect("cron registry poisoned");
            inner
                .jobs
                .get(id)
                .map(|j| j.func.clone())
                .ok_or_else(|| AppError::not_found("Missing or invalid cron job."))?
        };
        run_guarded(id.to_string(), func, Utc::now()).await;
        Ok(())
    }

    /// Start the ticker. Idempotent: a second call is a no-op, so a test
    /// harness that bootstraps twice does not get two schedulers.
    pub fn start(&self) {
        if self.started.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                service.tick(Utc::now());
            }
        });
    }

    pub fn is_started(&self) -> bool {
        self.started.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Fire every job due at `now`, at most once per minute per job. Every
    /// job fired by the same call gets the same [`TickAt`] — `now` floored
    /// to the minute (seconds and sub-seconds zeroed) — so two nodes
    /// ticking the same scheduled minute, even microseconds apart by wall
    /// clock, agree on exactly the same canonical tick for
    /// `crate::cron_history`'s durable claim to key on.
    ///
    /// Public so tests can drive the scheduler deterministically.
    pub fn tick(&self, now: DateTime<Utc>) -> Vec<String> {
        let minute = now.timestamp() / 60;
        // `croner` matches to the second; the schedule is minute-resolution,
        // so normalise seconds and sub-seconds away.
        let at_minute = now
            .with_second(0)
            .and_then(|t| t.with_nanosecond(0))
            .unwrap_or(now);
        let due: Vec<Job> = {
            let mut inner = self.inner.write().expect("cron registry poisoned");
            let candidates: Vec<Job> = inner
                .order
                .iter()
                .filter_map(|id| inner.jobs.get(id).cloned())
                .collect();
            candidates
                .into_iter()
                .filter(|job| {
                    if inner.last_run.get(&job.id) == Some(&minute) {
                        return false;
                    }
                    if job.schedule.is_time_matching(&at_minute).unwrap_or(false) {
                        inner.last_run.insert(job.id.clone(), minute);
                        true
                    } else {
                        false
                    }
                })
                .collect()
        };

        let mut fired = Vec::with_capacity(due.len());
        for job in due {
            fired.push(job.id.clone());
            tokio::spawn(run_guarded(job.id, job.func, at_minute));
        }
        fired
    }
}

/// Run one job, turning a panic into a log line. A cron job is background
/// work nobody is waiting on; taking the scheduler down with it would
/// silently stop every other job.
async fn run_guarded(id: String, func: JobFn, tick_at: TickAt) {
    let started = std::time::Instant::now();
    let result =
        futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(func(tick_at))).await;
    match result {
        Ok(()) => {
            tracing::debug!(job = %id, elapsed_ms = started.elapsed().as_millis() as u64, "cron job finished")
        }
        Err(_) => tracing::error!(job = %id, "cron job panicked"),
    }
}

impl std::fmt::Debug for CronService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CronService")
            .field("jobs", &self.list())
            .field("started", &self.is_started())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[tokio::test]
    async fn add_list_remove_and_run() {
        let cron = CronService::new();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        cron.add("job-a", "0 0 * * *", move |_tick_at| {
            let h = h.clone();
            async move {
                h.fetch_add(1, Ordering::SeqCst);
            }
        })
        .unwrap();
        cron.add("job-b", "*/5 * * * *", |_tick_at| async {})
            .unwrap();

        assert_eq!(
            cron.list(),
            vec![
                CronJob {
                    id: "job-a".into(),
                    expression: "0 0 * * *".into()
                },
                CronJob {
                    id: "job-b".into(),
                    expression: "*/5 * * * *".into()
                },
            ]
        );

        cron.run("job-a").await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(cron.run("nope").await.unwrap_err().status(), 404);

        assert!(cron.remove("job-b"));
        assert!(!cron.remove("job-b"));
        assert_eq!(cron.list().len(), 1);
    }

    #[test]
    fn invalid_expressions_are_rejected() {
        let cron = CronService::new();
        let err = cron
            .add("bad", "not a cron", |_tick_at| async {})
            .unwrap_err();
        assert_eq!(err.status(), 400);
    }

    #[tokio::test]
    async fn a_job_fires_once_per_matching_minute() {
        let cron = CronService::new();
        cron.add("hourly", "0 * * * *", |_tick_at| async {})
            .unwrap();

        assert_eq!(cron.tick(at("2026-09-03T12:00:00Z")), ["hourly"]);
        assert!(
            cron.tick(at("2026-09-03T12:00:30Z")).is_empty(),
            "same minute"
        );
        assert!(cron.tick(at("2026-09-03T12:01:00Z")).is_empty(), "not due");
        assert_eq!(cron.tick(at("2026-09-03T13:00:00Z")), ["hourly"]);
    }

    #[tokio::test]
    async fn a_panicking_job_does_not_poison_the_service() {
        let cron = CronService::new();
        cron.add("boom", "* * * * *", |_tick_at| async { panic!("kaboom") })
            .unwrap();
        cron.run("boom").await.unwrap();
        assert_eq!(cron.list().len(), 1);
    }

    #[tokio::test]
    async fn tick_passes_the_same_floored_minute_to_every_job_it_fires() {
        let cron = CronService::new();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        cron.add("hourly", "0 * * * *", move |tick_at| {
            let s = s.clone();
            async move {
                s.lock().unwrap().push(tick_at);
            }
        })
        .unwrap();

        // Same minute, non-zero seconds and sub-second precision — the
        // tick handed to the job body must have both zeroed.
        let fired = at("2026-09-03T12:00:47.123Z");
        assert_eq!(cron.tick(fired), ["hourly"]);
        // The job runs on a spawned task; give it a moment.
        for _ in 0..50 {
            if !seen.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let got = seen.lock().unwrap().clone();
        assert_eq!(got, vec![at("2026-09-03T12:00:00Z")]);
    }
}
