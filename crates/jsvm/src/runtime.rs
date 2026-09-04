//! The public runtime: a pool of worker threads, each owning one QuickJS
//! context, fed through a shared job queue.
//!
//! # Threading model
//!
//! QuickJS contexts are single-threaded, so the runtime spawns `workers`
//! OS threads (default `max(2, cores / 2)`), each of which creates its own
//! [`rquickjs::Runtime`] + [`rquickjs::Context`], installs the PocketBase
//! globals and evaluates every `*.pb.js` under `hooks_dir`. Because each
//! worker evaluates the same files in the same order, every worker ends
//! up with the same handler table keyed by deterministic ids
//! (`<file>:<hook>:<tags>:<ordinal>`), so a call for any handler id can
//! be served by whichever worker is free. Only worker 0 reports
//! registrations (`register_hook`, `register_route`, `register_cron`) to
//! the host, so the server sees each registration exactly once.
//!
//! Calls (`call_hook`, `call_route`, `call_cron`, migrations) are
//! `async`: they put a job on a `crossbeam` MPMC channel and await a
//! `tokio::sync::oneshot` reply. Inside a worker, calls back into the
//! host (`$app.findRecordById`, `$http.send`, ...) are synchronous from
//! JavaScript's point of view: the worker thread blocks on the host
//! future through the tokio [`Handle`] captured at [`Runtime::start`].
//! Worker threads are plain OS threads, never tokio tasks, so blocking is
//! safe (a job never blocks a runtime worker).
//!
//! # `e.next()` semantics
//!
//! PocketBase's hook chain lets a JavaScript handler run code *after* the
//! remainder of the chain by calling `e.next()` in the middle of the
//! handler. This runtime approximates that: `e.next()` only records that
//! it was called. The handler runs to completion, the runtime returns a
//! [`JsEventOutcome`] describing the mutations the handler made
//! (`record` changes and their `dirty_fields`, `collection`, a response
//! set through `e.json()`), and the Rust chain continues *after* the
//! JavaScript handler returned if and only if `next_called` is `true`.
//! Consequences:
//!
//! - Code placed after `e.next()` runs *before* the rest of the chain
//!   (and before the framework's own action, e.g. the actual INSERT), not
//!   after it. For `onRecordAfter*Success` hooks this makes no observable
//!   difference; for `onRecordCreate` it means the record has no
//!   database-assigned state yet.
//! - Errors thrown by the rest of the chain do not propagate back into the
//!   handler's `try/catch` around `e.next()`.
//! - A handler that never calls `e.next()` still stops the chain, exactly
//!   as in PocketBase.
//!
//! The design leaves room for the faithful version: `e.next()` would
//! become a native function that hands a `oneshot` to the Rust chain,
//! blocks the worker thread until the chain reports back, and resumes the
//! handler. Only `Worker::run_hook` and the `next` implementation in
//! `prelude.js` would change; the public API would not.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use async_trait::async_trait;
use cratebase_core::{AppError, Collection, Record, SerializeOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::runtime::Handle;
use tokio::sync::oneshot;

use crate::host::{CronHandlerId, HookHandlerId, HostApi, RouteHandlerId};
use crate::worker::{self, Control, Job, MigrationDirection};

/// Runtime configuration.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Directory containing `*.pb.js` hook files (PocketBase's `pb_hooks`).
    pub hooks_dir: PathBuf,
    /// Directory containing `*.js` migration files (`pb_migrations`).
    pub migrations_dir: PathBuf,
    /// Where to write `types.d.ts` at startup, if anywhere.
    pub types_file: Option<PathBuf>,
    /// Worker thread count; `0` selects `max(2, cores / 2)`.
    pub workers: usize,
    /// Poll `hooks_dir` for changes and reload the hooks (`--dev`).
    pub hooks_watch: bool,
    /// Maximum wall-clock time a single JavaScript invocation may run
    /// before it is interrupted. Default 30s.
    pub timeout: Duration,
    /// Optional QuickJS heap limit per worker, in bytes.
    pub memory_limit: Option<usize>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        RuntimeConfig {
            hooks_dir: PathBuf::from("pb_hooks"),
            migrations_dir: PathBuf::from("pb_migrations"),
            types_file: None,
            workers: 0,
            hooks_watch: false,
            timeout: Duration::from_secs(30),
            memory_limit: None,
        }
    }
}

impl RuntimeConfig {
    pub fn new(hooks_dir: impl Into<PathBuf>, migrations_dir: impl Into<PathBuf>) -> Self {
        RuntimeConfig {
            hooks_dir: hooks_dir.into(),
            migrations_dir: migrations_dir.into(),
            ..Default::default()
        }
    }

    pub fn effective_workers(&self) -> usize {
        if self.workers > 0 {
            return self.workers;
        }
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        (cores / 2).max(2)
    }
}

/// A snapshot of a hook event handed to JavaScript.
///
/// `data` becomes the properties of the JS event object `e`. Well-known
/// keys get special treatment in the prelude: `record` (+ `recordIsNew`)
/// and `auth` are wrapped as `Record` objects, `collection` as a
/// `Collection`, `requestInfo` backs `e.requestInfo()`, and `request`
/// backs `e.request`. Everything else is exposed as plain data.
#[derive(Clone, Default)]
pub struct JsEvent {
    pub data: Map<String, Value>,
    /// The host to route `$app` / `e.app` calls to while this handler
    /// runs (the transactional app inside record hooks). `None` uses the
    /// runtime's default host.
    pub app: Option<Arc<dyn HostApi>>,
}

impl std::fmt::Debug for JsEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsEvent")
            .field("data", &self.data)
            .field("app", &self.app.as_ref().map(|_| "<host>"))
            .finish()
    }
}

/// Serialization used for records handed to JavaScript: hooks are trusted
/// code and see hidden fields and unmasked emails.
pub const JS_RECORD_OPTIONS: SerializeOptions = SerializeOptions {
    with_hidden: true,
    show_email: true,
    with_custom_data: true,
};

impl JsEvent {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set an arbitrary property on the event.
    pub fn with(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }

    pub fn record(mut self, record: &Record) -> Self {
        self.data
            .insert("record".into(), record.to_json(JS_RECORD_OPTIONS));
        self.data
            .insert("recordIsNew".into(), Value::Bool(record.is_new()));
        self
    }

    pub fn collection(mut self, collection: &Collection) -> Self {
        self.data.insert("collection".into(), collection.to_json());
        self
    }

    pub fn auth(mut self, auth: Option<&Record>) -> Self {
        self.data.insert(
            "auth".into(),
            auth.map(|r| r.to_json(JS_RECORD_OPTIONS))
                .unwrap_or(Value::Null),
        );
        self
    }

    /// The `e.requestInfo()` payload: `{method, query, headers, body, context}`.
    pub fn request_info(mut self, info: Value) -> Self {
        self.data.insert("requestInfo".into(), info);
        self
    }

    pub fn app(mut self, app: Arc<dyn HostApi>) -> Self {
        self.app = Some(app);
        self
    }
}

/// What a JavaScript hook handler did, for the Rust chain to apply.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsEventOutcome {
    /// `true` when the handler called `e.next()`. When `false` the chain
    /// must stop (without error), as in PocketBase.
    pub next_called: bool,
    /// The record as the handler left it (same shape as the input
    /// snapshot), if the event carried one.
    #[serde(default)]
    pub record: Option<Value>,
    /// Field names the handler wrote through `e.record.set(...)`.
    #[serde(default)]
    pub dirty_fields: Vec<String>,
    /// The collection as the handler left it, if the event carried one.
    #[serde(default)]
    pub collection: Option<Value>,
    /// A response the handler produced with `e.json()` / `e.string()` /
    /// ... (request hooks only). The server should send it and stop.
    #[serde(default)]
    pub response: Option<JsResponse>,
    /// Every other plain event property after the handler ran (e.g. a
    /// mutated `message` on mailer events).
    #[serde(default)]
    pub event: Map<String, Value>,
}

impl JsEventOutcome {
    /// Copy the fields the handler changed onto `record`.
    pub fn apply_to_record(&self, record: &mut Record) {
        let Some(Value::Object(snapshot)) = &self.record else {
            return;
        };
        for field in &self.dirty_fields {
            if let Some(v) = snapshot.get(field) {
                record.set(field, v.clone());
            }
        }
    }
}

/// An HTTP request forwarded to a `routerAdd` handler.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JsRequest {
    pub method: String,
    pub path: String,
    pub query: Map<String, Value>,
    /// Header names lower-cased.
    pub headers: Map<String, Value>,
    /// Parsed JSON body, or a string for non-JSON bodies, or `null`.
    pub body: Value,
    /// The authenticated record's JSON, if any.
    pub auth: Option<Value>,
    /// Path parameters already extracted by the server's router. When
    /// `None` the runtime matches `path` against the registered pattern.
    pub path_params: Option<Map<String, Value>>,
    pub remote_ip: String,
}

/// The body of a [`JsResponse`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum JsBody {
    Json(Value),
    Text(String),
    Html(String),
    Bytes(Vec<u8>),
    Redirect(String),
    NoContent,
}

impl Default for JsBody {
    fn default() -> Self {
        JsBody::NoContent
    }
}

/// The response a route handler produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JsResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: JsBody,
}

/// Persistence for applied migrations, implemented by the db crate.
#[async_trait]
pub trait MigrationLedger: Send + Sync {
    /// File names (not paths) of applied migrations, oldest first.
    async fn applied(&self) -> Result<Vec<String>, AppError>;
    async fn mark_applied(&self, file: &str) -> Result<(), AppError>;
    async fn mark_reverted(&self, file: &str) -> Result<(), AppError>;
}

struct WorkerHandle {
    control: crossbeam_channel::Sender<Control>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

struct Inner {
    cfg: RuntimeConfig,
    jobs: crossbeam_channel::Sender<Job>,
    workers: Vec<WorkerHandle>,
}

/// The JavaScript runtime. Cheap to clone; all clones share the pool.
#[derive(Clone)]
pub struct Runtime {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("workers", &self.inner.workers.len())
            .field("hooks_dir", &self.inner.cfg.hooks_dir)
            .finish()
    }
}

impl Runtime {
    /// Spawn the worker pool, evaluate the hook files on every worker and
    /// report registrations to `host`. Must be called from within a tokio
    /// runtime: the workers use its [`Handle`] to drive host futures.
    pub async fn start(host: Arc<dyn HostApi>, cfg: RuntimeConfig) -> Result<Runtime, AppError> {
        let handle = Handle::try_current()
            .map_err(|_| AppError::internal("Runtime::start needs a tokio runtime"))?;

        if let Some(path) = &cfg.types_file {
            if let Err(e) = crate::types::write_types_file(path) {
                tracing::warn!(error = %e, path = %path.display(), "could not write types.d.ts");
            }
        }

        let n = cfg.effective_workers();
        let (job_tx, job_rx) = crossbeam_channel::unbounded::<Job>();
        let mut workers = Vec::with_capacity(n);
        let mut ready = Vec::with_capacity(n);
        let shared = Arc::new(cfg.clone());
        for index in 0..n {
            let (control_tx, control_rx) = crossbeam_channel::unbounded::<Control>();
            let (ready_tx, ready_rx) = oneshot::channel::<Result<(), AppError>>();
            let thread = worker::spawn(worker::WorkerInit {
                index,
                host: host.clone(),
                cfg: shared.clone(),
                handle: handle.clone(),
                jobs: job_rx.clone(),
                control: control_rx,
                ready: ready_tx,
            });
            workers.push(WorkerHandle {
                control: control_tx,
                thread: Mutex::new(Some(thread)),
            });
            ready.push(ready_rx);
        }

        let mut first_error = None;
        for rx in ready {
            match rx.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    first_error.get_or_insert(e);
                }
                Err(_) => {
                    first_error.get_or_insert_with(|| {
                        AppError::internal("jsvm worker died during startup")
                    });
                }
            };
        }

        let runtime = Runtime {
            inner: Arc::new(Inner {
                cfg,
                jobs: job_tx,
                workers,
            }),
        };

        if let Some(e) = first_error {
            runtime.stop();
            return Err(e);
        }

        if runtime.inner.cfg.hooks_watch {
            spawn_watcher(Arc::downgrade(&runtime.inner), runtime.clone());
        }

        Ok(runtime)
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.inner.cfg
    }

    pub fn workers(&self) -> usize {
        self.inner.workers.len()
    }

    async fn submit<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<Result<T, AppError>>) -> Job,
    ) -> Result<T, AppError> {
        let (tx, rx) = oneshot::channel();
        self.inner
            .jobs
            .send(build(tx))
            .map_err(|_| AppError::internal("jsvm worker pool is shut down"))?;
        rx.await
            .map_err(|_| AppError::internal("jsvm worker dropped the reply"))?
    }

    /// Invoke a hook handler with an event snapshot. See the module docs
    /// for the `e.next()` semantics.
    pub async fn call_hook(
        &self,
        handler: &HookHandlerId,
        event: JsEvent,
    ) -> Result<JsEventOutcome, AppError> {
        let id = handler.clone();
        self.submit(move |reply| Job::Hook { id, event, reply })
            .await
    }

    /// Invoke a `routerAdd` handler.
    pub async fn call_route(
        &self,
        handler: &RouteHandlerId,
        req: JsRequest,
    ) -> Result<JsResponse, AppError> {
        let id = handler.clone();
        self.submit(move |reply| Job::Route { id, req, reply })
            .await
    }

    /// Invoke a `cronAdd` job.
    pub async fn call_cron(&self, handler: &CronHandlerId) -> Result<(), AppError> {
        let id = handler.clone();
        self.submit(move |reply| Job::Cron { id, reply }).await
    }

    /// Migration files in `migrations_dir`, sorted by file name.
    pub fn migration_files(&self) -> Result<Vec<PathBuf>, AppError> {
        list_files(&self.inner.cfg.migrations_dir, |name| name.ends_with(".js"))
    }

    /// Apply every migration not yet in the ledger, in file-name order.
    /// Returns the file names applied.
    pub async fn run_migrations_up(
        &self,
        ledger: &dyn MigrationLedger,
    ) -> Result<Vec<String>, AppError> {
        let applied = ledger.applied().await?;
        let mut done = Vec::new();
        for file in self.migration_files()? {
            let name = file_name(&file);
            if applied.iter().any(|a| a == &name) {
                continue;
            }
            self.run_migration(file.clone(), MigrationDirection::Up)
                .await
                .map_err(|e| prefix_error(&name, e))?;
            ledger.mark_applied(&name).await?;
            done.push(name);
        }
        Ok(done)
    }

    /// Revert the last `n` applied migrations (newest first). Returns the
    /// file names reverted.
    pub async fn run_migrations_down(
        &self,
        ledger: &dyn MigrationLedger,
        n: usize,
    ) -> Result<Vec<String>, AppError> {
        let files = self.migration_files()?;
        let mut applied = ledger.applied().await?;
        applied.sort();
        let mut done = Vec::new();
        for name in applied.into_iter().rev().take(n) {
            let Some(file) = files.iter().find(|f| file_name(f) == name) else {
                return Err(AppError::internal(format!(
                    "migration {name} is applied but its file is missing"
                )));
            };
            self.run_migration(file.clone(), MigrationDirection::Down)
                .await
                .map_err(|e| prefix_error(&name, e))?;
            ledger.mark_reverted(&name).await?;
            done.push(name);
        }
        Ok(done)
    }

    async fn run_migration(
        &self,
        file: PathBuf,
        direction: MigrationDirection,
    ) -> Result<(), AppError> {
        self.submit(move |reply| Job::Migration {
            file,
            direction,
            reply,
        })
        .await
    }

    /// Re-evaluate the hook files on every worker (used in `--dev`).
    /// Returns once every worker has reloaded; the first error, if any,
    /// is returned but the other workers still reload.
    pub async fn reload(&self) -> Result<(), AppError> {
        let mut receivers = Vec::with_capacity(self.inner.workers.len());
        for w in &self.inner.workers {
            let (tx, rx) = oneshot::channel();
            if w.control.send(Control::Reload(tx)).is_ok() {
                receivers.push(rx);
            }
        }
        let mut first_error = None;
        for rx in receivers {
            if let Ok(Err(e)) = rx.await {
                first_error.get_or_insert(e);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Ask every worker to reload without waiting for the result.
    pub fn reload_in_background(&self) {
        for w in &self.inner.workers {
            let (tx, _rx) = oneshot::channel();
            let _ = w.control.send(Control::Reload(tx));
        }
    }

    /// Stop the workers. Pending jobs fail with an error.
    pub fn stop(&self) {
        for w in &self.inner.workers {
            let _ = w.control.send(Control::Shutdown);
        }
        for w in &self.inner.workers {
            if let Some(t) = w.thread.lock().ok().and_then(|mut g| g.take()) {
                let _ = t.join();
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        for w in &self.workers {
            let _ = w.control.send(Control::Shutdown);
        }
    }
}

fn prefix_error(name: &str, e: AppError) -> AppError {
    match e {
        AppError::Internal(m) => AppError::Internal(format!("migration {name}: {m}")),
        AppError::BadRequest(m) => AppError::BadRequest(format!("migration {name}: {m}")),
        other => other,
    }
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Regular files in `dir` whose name satisfies `keep`, sorted by name.
/// A missing directory yields an empty list.
pub(crate) fn list_files(
    dir: &Path,
    keep: impl Fn(&str) -> bool,
) -> Result<Vec<PathBuf>, AppError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => {
            return Err(AppError::internal(format!(
                "cannot read {}: {e}",
                dir.display()
            )))
        }
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && keep(&file_name(p)))
        .collect();
    files.sort();
    Ok(files)
}

/// Poll the hooks directory once a second and reload when any hook file's
/// modification time or the file set changes. Stops when the runtime is
/// dropped.
fn spawn_watcher(inner: Weak<Inner>, runtime: Runtime) {
    // The watcher must not keep the runtime alive, so drop the strong
    // reference and use the weak one for the reload trigger.
    let dir = runtime.inner.cfg.hooks_dir.clone();
    drop(runtime);
    std::thread::Builder::new()
        .name("jsvm-hooks-watch".into())
        .spawn(move || {
            let mut last = snapshot(&dir);
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let Some(inner) = inner.upgrade() else { break };
                let now = snapshot(&dir);
                if now != last {
                    last = now;
                    tracing::info!("pb_hooks changed, reloading");
                    for w in &inner.workers {
                        let (tx, _rx) = oneshot::channel();
                        let _ = w.control.send(Control::Reload(tx));
                    }
                }
            }
        })
        .ok();
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, Option<std::time::SystemTime>)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            let mtime = e.metadata().ok().and_then(|m| m.modified().ok());
            out.push((p, mtime));
        }
    }
    out.sort();
    out
}
