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

/// A job body. Boxed and shared so `run(id)` can fire the same closure
/// the scheduler uses.
pub type JobFn = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// System job ids, kept identical to PocketBase's.
pub const JOB_DB_OPTIMIZE: &str = "__pbDBOptimize__";
pub const JOB_MFA_CLEANUP: &str = "__pbMFACleanup__";
pub const JOB_OTP_CLEANUP: &str = "__pbOTPCleanup__";
pub const JOB_LOGS_CLEANUP: &str = "__pbLogsCleanup__";
/// The scheduled backup. Unlike the four above it is only registered once
/// `settings.backups.cron` is non-empty, so `GET /api/crons` lists it only
/// then — PocketBase behaves the same way.
pub const JOB_AUTO_BACKUP: &str = "__pbAutoBackup__";
/// Hourly `_sessions` sweep (`crate::sessions::sweep_expired`). Cratebase-only
/// — the four ids above are PocketBase's, verbatim.
pub const JOB_SESSION_SWEEP: &str = "__cbSessionSweep__";

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
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let func: JobFn = Arc::new(move || Box::pin(func()));
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
    /// learns whether it panicked.
    pub async fn run(&self, id: &str) -> Result<(), AppError> {
        let func = {
            let inner = self.inner.read().expect("cron registry poisoned");
            inner
                .jobs
                .get(id)
                .map(|j| j.func.clone())
                .ok_or_else(|| AppError::not_found("Missing or invalid cron job."))?
        };
        run_guarded(id.to_string(), func).await;
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

    /// Fire every job due at `now`, at most once per minute per job.
    /// Public so tests can drive the scheduler deterministically.
    pub fn tick(&self, now: DateTime<Utc>) -> Vec<String> {
        let minute = now.timestamp() / 60;
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
                    // `croner` matches to the second; the schedule is
                    // minute-resolution, so normalise the second away.
                    let at_minute = now.with_second(0).unwrap_or(now);
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
            tokio::spawn(run_guarded(job.id, job.func));
        }
        fired
    }
}

/// Run one job, turning a panic into a log line. A cron job is background
/// work nobody is waiting on; taking the scheduler down with it would
/// silently stop every other job.
async fn run_guarded(id: String, func: JobFn) {
    let started = std::time::Instant::now();
    let result = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(func())).await;
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
        cron.add("job-a", "0 0 * * *", move || {
            let h = h.clone();
            async move {
                h.fetch_add(1, Ordering::SeqCst);
            }
        })
        .unwrap();
        cron.add("job-b", "*/5 * * * *", || async {}).unwrap();

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
        let err = cron.add("bad", "not a cron", || async {}).unwrap_err();
        assert_eq!(err.status(), 400);
    }

    #[tokio::test]
    async fn a_job_fires_once_per_matching_minute() {
        let cron = CronService::new();
        cron.add("hourly", "0 * * * *", || async {}).unwrap();

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
        cron.add("boom", "* * * * *", || async { panic!("kaboom") })
            .unwrap();
        cron.run("boom").await.unwrap();
        assert_eq!(cron.list().len(), 1);
    }
}
