//! One worker thread: owns a QuickJS runtime and context, evaluates the
//! hook files, and executes jobs from the shared queue.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use cratebase_core::AppError;
use rquickjs::context::EvalOptions;
use rquickjs::{Context, Ctx, Function, Object, Value as JsValue};
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle as TaskHandle;

use crate::bridge;
use crate::convert::{from_js, js_error_to_app_error, to_js};
use crate::host::{
    CronHandlerId, HookHandlerId, HookKind, HostApi, QueueHandlerId, RouteHandlerId,
};
use crate::runtime::{
    file_name, list_files, JsEvent, JsEventOutcome, JsRequest, JsResponse, RuntimeConfig,
};

/// A unit of work for any worker.
pub(crate) enum Job {
    Hook {
        id: HookHandlerId,
        event: JsEvent,
        reply: oneshot::Sender<Result<JsEventOutcome, AppError>>,
    },
    Route {
        id: RouteHandlerId,
        req: JsRequest,
        reply: oneshot::Sender<Result<JsResponse, AppError>>,
    },
    Cron {
        id: CronHandlerId,
        reply: oneshot::Sender<Result<(), AppError>>,
    },
    Queue {
        id: QueueHandlerId,
        payload: Value,
        reply: oneshot::Sender<Result<(), AppError>>,
    },
    Migration {
        file: PathBuf,
        direction: MigrationDirection,
        reply: oneshot::Sender<Result<(), AppError>>,
    },
}

impl Job {
    /// Fail the job without running it (pool shutting down).
    fn reject(self, err: AppError) {
        match self {
            Job::Hook { reply, .. } => {
                let _ = reply.send(Err(err));
            }
            Job::Route { reply, .. } => {
                let _ = reply.send(Err(err));
            }
            Job::Cron { reply, .. } | Job::Queue { reply, .. } | Job::Migration { reply, .. } => {
                let _ = reply.send(Err(err));
            }
        }
    }
}

/// A message for one specific worker.
///
/// Reloading the handler table is a two-phase protocol driven by the pool
/// (`Runtime::reload`) so that all workers switch generation together or
/// not at all: `Prepare` evaluates the hook files into a side table (the
/// worker keeps serving its current table meanwhile), then the pool sends
/// `Commit` to everyone if every worker prepared cleanly, `Discard`
/// otherwise.
pub(crate) enum Control {
    Prepare(oneshot::Sender<Result<(), AppError>>),
    Commit {
        generation: u64,
        reply: oneshot::Sender<()>,
    },
    Discard,
    Shutdown,
}

/// A registration worker 0 reports to the host. Collected while the hook
/// files are being evaluated and applied only on commit, so the host never
/// sees a half-loaded or failed generation.
pub(crate) enum Registration {
    Hook {
        id: String,
        kind: HookKind,
        tags: Vec<String>,
        priority: i32,
    },
    Route {
        method: String,
        path: String,
        id: String,
    },
    Cron {
        id: String,
        expr: String,
    },
    RemoveCron(String),
    Queue(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MigrationDirection {
    Up,
    Down,
}

impl MigrationDirection {
    fn as_str(self) -> &'static str {
        match self {
            MigrationDirection::Up => "up",
            MigrationDirection::Down => "down",
        }
    }
}

pub(crate) struct WorkerInit {
    pub index: usize,
    pub host: Arc<dyn HostApi>,
    pub cfg: Arc<RuntimeConfig>,
    pub handle: Handle,
    pub jobs: crossbeam_channel::Receiver<Job>,
    pub control: crossbeam_channel::Receiver<Control>,
    pub ready: oneshot::Sender<Result<(), AppError>>,
}

/// A transaction opened by `$app.runInTransaction` that JavaScript is
/// currently inside of. See [`bridge::tx_begin`].
pub(crate) struct PendingTx {
    pub done: Option<oneshot::Sender<Result<(), AppError>>>,
    pub join: TaskHandle<Result<(), AppError>>,
}

/// One entry of the host stack. A transaction frame owns its
/// [`PendingTx`], so the host override and the transaction it belongs to
/// are pushed and popped together and cannot get out of step.
pub(crate) struct HostFrame {
    pub host: Arc<dyn HostApi>,
    pub tx: Option<PendingTx>,
}

/// Per-worker state shared with the native functions installed in the
/// context. Everything lives on the worker thread, hence `Rc`/`RefCell`.
pub(crate) struct WorkerState {
    pub index: usize,
    pub cfg: Arc<RuntimeConfig>,
    pub handle: Handle,
    /// The runtime's default host, bottom of `host_stack`.
    pub host: Arc<dyn HostApi>,
    /// Hosts pushed for the duration of a call: the transactional app of
    /// a record hook, or a transaction opened from JavaScript.
    pub host_stack: RefCell<Vec<HostFrame>>,
    /// Interrupt deadline for the running job.
    pub deadline: Rc<Cell<Option<Instant>>>,
    /// Set by the interrupt handler when it fires, so "timed out" is
    /// recorded rather than re-derived from the clock afterwards.
    pub interrupted: Rc<Cell<bool>>,
    /// Ids reported to the host by this worker (worker 0 only), so a
    /// reload can retract them.
    pub registered_hooks: RefCell<Vec<String>>,
    pub registered_crons: RefCell<Vec<String>>,
    /// While a (re)load is being prepared, worker 0's registrations are
    /// collected here instead of going to the host.
    pub staged: RefCell<Option<Vec<Registration>>>,
    /// The handler-table generation this worker is serving.
    pub generation: Cell<u64>,
}

impl WorkerState {
    pub fn current_host(&self) -> Arc<dyn HostApi> {
        self.host_stack
            .borrow()
            .last()
            .map(|f| f.host.clone())
            .unwrap_or_else(|| self.host.clone())
    }

    /// Report a registration (worker 0 only): staged while a load is being
    /// prepared, applied to the host immediately otherwise (a runtime
    /// `cronAdd` from inside a handler).
    pub fn register(&self, reg: Registration) {
        if let Some(staged) = self.staged.borrow_mut().as_mut() {
            staged.push(reg);
            return;
        }
        self.apply(reg);
    }

    fn apply(&self, reg: Registration) {
        let host = &self.host;
        match reg {
            Registration::Hook {
                id,
                kind,
                tags,
                priority,
            } => {
                self.registered_hooks.borrow_mut().push(id.clone());
                host.register_hook(kind, tags, HookHandlerId(id), priority);
            }
            Registration::Route { method, path, id } => {
                host.register_route(&method, &path, RouteHandlerId(id));
            }
            Registration::Cron { id, expr } => {
                self.registered_crons.borrow_mut().push(id.clone());
                host.register_cron(&id, &expr, CronHandlerId(id.clone()));
            }
            Registration::RemoveCron(id) => {
                self.registered_crons.borrow_mut().retain(|c| c != &id);
                host.remove_cron(&id);
            }
            Registration::Queue(queue) => {
                host.register_queue_handler(&queue, QueueHandlerId(queue.clone()));
            }
        }
    }

    /// Whether this worker reports registrations to the host.
    pub fn reports(&self) -> bool {
        self.index == 0
    }

    /// Drive a host future to completion from the worker thread.
    pub fn block_on<F: std::future::Future>(&self, fut: F) -> F::Output {
        self.handle.block_on(fut)
    }
}

pub(crate) fn spawn(init: WorkerInit) -> JoinHandle<()> {
    let name = format!("jsvm-worker-{}", init.index);
    std::thread::Builder::new()
        .name(name)
        .spawn(move || run(init))
        .expect("spawn jsvm worker thread")
}

fn run(init: WorkerInit) {
    let WorkerInit {
        index,
        host,
        cfg,
        handle,
        jobs,
        control,
        ready,
    } = init;

    let worker = match Worker::new(index, host, cfg, handle) {
        Ok(w) => w,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    // Startup is the first prepare; the pool commits (or discards) once
    // every worker reported in, so a partially started pool never becomes
    // visible to the host.
    let _ = ready.send(worker.prepare());

    // Returns false when the worker should exit.
    let handle_control = |msg: Result<Control, crossbeam_channel::RecvError>| match msg {
        Ok(Control::Prepare(reply)) => {
            let _ = reply.send(worker.prepare());
            true
        }
        Ok(Control::Commit { generation, reply }) => {
            worker.commit(generation);
            let _ = reply.send(());
            true
        }
        Ok(Control::Discard) => {
            worker.discard();
            true
        }
        Ok(Control::Shutdown) | Err(_) => false,
    };
    loop {
        // Control messages (reload phases, shutdown) take priority over
        // queued jobs, so a shutdown is not starved by a long job queue.
        if let Ok(msg) = control.try_recv() {
            if !handle_control(Ok(msg)) {
                break;
            }
            continue;
        }
        crossbeam_channel::select! {
            recv(control) -> msg => if !handle_control(msg) { break },
            recv(jobs) -> job => match job {
                Ok(job) => worker.execute(job),
                Err(_) => break,
            },
        }
    }
    // Do not leave callers hanging on a reply that will never come: fail
    // whatever is still queued with a clear error.
    for job in jobs.try_iter() {
        job.reject(AppError::internal("jsvm runtime is shutting down"));
    }
}

struct Worker {
    state: Rc<WorkerState>,
    _runtime: rquickjs::Runtime,
    context: Context,
}

impl Worker {
    fn new(
        index: usize,
        host: Arc<dyn HostApi>,
        cfg: Arc<RuntimeConfig>,
        handle: Handle,
    ) -> Result<Self, AppError> {
        let runtime = rquickjs::Runtime::new()
            .map_err(|e| AppError::internal(format!("cannot create QuickJS runtime: {e}")))?;
        if let Some(limit) = cfg.memory_limit {
            runtime.set_memory_limit(limit);
        }
        let deadline = Rc::new(Cell::new(None::<Instant>));
        let interrupted = Rc::new(Cell::new(false));
        {
            // The interrupt handler runs on this thread only, so a plain
            // Rc is fine; it must be 'static though, hence the clone.
            let deadline = deadline.clone();
            let interrupted = interrupted.clone();
            runtime.set_interrupt_handler(Some(Box::new(move || {
                let fire = deadline.get().is_some_and(|d| Instant::now() >= d);
                if fire {
                    interrupted.set(true);
                }
                fire
            })));
        }
        let context = Context::full(&runtime)
            .map_err(|e| AppError::internal(format!("cannot create QuickJS context: {e}")))?;

        let state = Rc::new(WorkerState {
            index,
            cfg,
            handle,
            host,
            host_stack: RefCell::new(vec![]),
            deadline,
            interrupted,
            registered_hooks: RefCell::new(vec![]),
            registered_crons: RefCell::new(vec![]),
            staged: RefCell::new(None),
            generation: Cell::new(0),
        });

        let worker = Worker {
            state,
            _runtime: runtime,
            context,
        };
        worker.with_ctx(|ctx| bridge::install(ctx, worker.state.clone()))?;
        Ok(worker)
    }

    /// Run `f` with the context, converting JavaScript exceptions into
    /// [`AppError`]s (including the interrupt-on-timeout case).
    fn with_ctx<T>(&self, f: impl FnOnce(&Ctx<'_>) -> rquickjs::Result<T>) -> Result<T, AppError> {
        self.context.with(|ctx| match f(&ctx) {
            Ok(v) => Ok(v),
            Err(rquickjs::Error::Exception) => {
                let timed_out = self.state.interrupted.get();
                Err(js_error_to_app_error(
                    &ctx,
                    ctx.catch(),
                    timed_out,
                    self.state.cfg.timeout,
                ))
            }
            Err(e) => Err(AppError::internal(format!("QuickJS error: {e}"))),
        })
    }

    /// Run a job body with the timeout armed, and clean up anything the
    /// JavaScript side left open (transactions, host overrides).
    fn timed<T>(&self, f: impl FnOnce(&Ctx<'_>) -> rquickjs::Result<T>) -> Result<T, AppError> {
        self.state.interrupted.set(false);
        self.state
            .deadline
            .set(Some(Instant::now() + self.state.cfg.timeout));
        let result = self.with_ctx(f);
        self.state.deadline.set(None);
        bridge::abort_open_transactions(&self.state);
        result
    }

    fn cb<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        ctx.globals().get("__cb")
    }

    fn execute(&self, job: Job) {
        match job {
            Job::Hook { id, event, reply } => {
                let _ = reply.send(self.run_hook(&id, event));
            }
            Job::Route { id, req, reply } => {
                let _ = reply.send(self.run_route(&id, req));
            }
            Job::Cron { id, reply } => {
                let _ = reply.send(self.run_cron(&id));
            }
            Job::Queue { id, payload, reply } => {
                let _ = reply.send(self.run_queue_job(&id, payload));
            }
            Job::Migration {
                file,
                direction,
                reply,
            } => {
                let _ = reply.send(self.run_migration(&file, direction));
            }
        }
    }

    fn run_hook(&self, id: &HookHandlerId, event: JsEvent) -> Result<JsEventOutcome, AppError> {
        let JsEvent { data, app } = event;
        if let Some(app) = app {
            self.state.host_stack.borrow_mut().push(HostFrame {
                host: app,
                tx: None,
            });
        }
        let result = self.timed(|ctx| {
            let invoke: Function = Self::cb(ctx)?.get("invokeHook")?;
            let ev = to_js(ctx, &Value::Object(data))?;
            let out: JsValue = invoke.call((id.0.as_str(), ev))?;
            from_js(ctx, out)
        });
        self.state.host_stack.borrow_mut().clear();
        let raw = result?;
        serde_json::from_value(raw)
            .map_err(|e| AppError::internal(format!("bad hook outcome from JavaScript: {e}")))
    }

    fn run_route(&self, id: &RouteHandlerId, req: JsRequest) -> Result<JsResponse, AppError> {
        let raw = self.timed(|ctx| {
            let invoke: Function = Self::cb(ctx)?.get("invokeRoute")?;
            let req = serde_json::to_value(&req).unwrap_or(Value::Null);
            let out: JsValue = invoke.call((id.0.as_str(), to_js(ctx, &req)?))?;
            from_js(ctx, out)
        })?;
        serde_json::from_value(raw)
            .map_err(|e| AppError::internal(format!("bad route response from JavaScript: {e}")))
    }

    fn run_cron(&self, id: &CronHandlerId) -> Result<(), AppError> {
        self.timed(|ctx| {
            let invoke: Function = Self::cb(ctx)?.get("invokeCron")?;
            invoke.call::<_, ()>((id.0.as_str(),))
        })
    }

    /// Invoke an `onQueueJob` handler. A thrown JS error becomes
    /// `Err(AppError)` here; `crate::jsvm_host::HostApi::register_queue_handler`
    /// (server crate) turns that into the `Err(String)` `crate::queue::HandlerFn`
    /// expects, which the queue's own retry/backoff machinery treats
    /// exactly like a Rust handler's `Err` — same failure/retry path
    /// either way.
    fn run_queue_job(&self, id: &QueueHandlerId, payload: Value) -> Result<(), AppError> {
        self.timed(|ctx| {
            let invoke: Function = Self::cb(ctx)?.get("invokeQueueJob")?;
            let payload_js = to_js(ctx, &payload)?;
            invoke.call::<_, ()>((id.0.as_str(), payload_js))
        })
    }

    fn run_migration(&self, file: &PathBuf, direction: MigrationDirection) -> Result<(), AppError> {
        let source = std::fs::read_to_string(file)
            .map_err(|e| AppError::internal(format!("cannot read {}: {e}", file.display())))?;
        let name = file_name(file);
        let result = self.timed(|ctx| {
            let cb = Self::cb(ctx)?;
            cb.set("currentFile", name.as_str())?;
            cb.set("pendingMigration", JsValue::new_null(ctx.clone()))?;
            eval_file(ctx, &name, &source)?;
            let run: Function = cb.get("runMigration")?;
            run.call::<_, ()>((direction.as_str(),))
        });
        self.clear_load_context();
        result
    }

    /// Forget which file is being evaluated and any `migrate()` it
    /// registered, so neither can leak into the next load or migration
    /// (including after a failure that skipped `runMigration`).
    fn clear_load_context(&self) {
        let _ = self.with_ctx(|ctx| {
            let cb = Self::cb(ctx)?;
            cb.set("currentFile", JsValue::new_null(ctx.clone()))?;
            cb.set("pendingMigration", JsValue::new_null(ctx.clone()))
        });
    }

    /// Evaluate every `*.cb.js` in `hooks_dir`, in file-name order, into a
    /// fresh side table. The currently served table is untouched until
    /// [`Worker::commit`]; on any failure the side table is dropped and the
    /// previous (good) handlers keep serving.
    fn prepare(&self) -> Result<(), AppError> {
        *self.state.staged.borrow_mut() = self.state.reports().then(Vec::new);
        let begun = self.timed(|ctx| {
            Self::cb(ctx)?
                .get::<_, Function>("beginLoad")?
                .call::<_, ()>(())
        });
        let result = begun.and_then(|()| {
            let loaded = self.load_files();
            let ok = loaded.is_ok();
            let ended = self.timed(|ctx| {
                Self::cb(ctx)?
                    .get::<_, Function>("endLoad")?
                    .call::<_, ()>((ok,))
            });
            loaded.and(ended)
        });
        if result.is_err() {
            *self.state.staged.borrow_mut() = None;
        }
        result
    }

    fn load_files(&self) -> Result<(), AppError> {
        let files = list_files(&self.state.cfg.hooks_dir, |n| n.ends_with(".cb.js"))?;
        let mut first_error = None;
        for file in files {
            let name = file_name(&file);
            let source = match std::fs::read_to_string(&file) {
                Ok(s) => s,
                Err(e) => {
                    first_error.get_or_insert(AppError::internal(format!(
                        "cannot read {}: {e}",
                        file.display()
                    )));
                    continue;
                }
            };
            let result = self.timed(|ctx| {
                Self::cb(ctx)?.set("currentFile", name.as_str())?;
                eval_file(ctx, &name, &source)
            });
            if let Err(e) = result {
                tracing::error!(file = %name, error = %e, "failed to load hook file");
                first_error.get_or_insert(AppError::internal(format!("{name}: {e}")));
            }
        }
        self.clear_load_context();
        first_error.map_or(Ok(()), Err)
    }

    /// Switch to the prepared table and, on worker 0, swap the host's
    /// registrations: the previous ones are retracted and the new ones
    /// applied back to back, with no JavaScript evaluation in between (all
    /// of that already happened in `prepare`), so the host never observes
    /// an empty or half-built table.
    fn commit(&self, generation: u64) {
        let _ = self.with_ctx(|ctx| {
            Self::cb(ctx)?
                .get::<_, Function>("commit")?
                .call::<_, ()>(())
        });
        let staged = self.state.staged.borrow_mut().take();
        if let Some(staged) = staged {
            for id in self.state.registered_hooks.borrow_mut().drain(..) {
                self.state.host.unregister_hook(&id);
            }
            for id in self.state.registered_crons.borrow_mut().drain(..) {
                self.state.host.remove_cron(&id);
            }
            // Routes and queue handlers have no per-registration id to
            // retract individually: the host drops the whole table and the
            // staged registrations rebuild it from the new files.
            self.state.host.clear_routes();
            self.state.host.clear_queue_handlers();
            for reg in staged {
                self.state.apply(reg);
            }
        }
        self.state.generation.set(generation);
    }

    /// Throw away a prepared-but-uncommitted table.
    fn discard(&self) {
        *self.state.staged.borrow_mut() = None;
        let _ = self.with_ctx(|ctx| {
            Self::cb(ctx)?
                .get::<_, Function>("discard")?
                .call::<_, ()>(())
        });
    }
}

/// Evaluate a script in the global scope under its file name (for stack
/// traces).
pub(crate) fn eval_file(ctx: &Ctx<'_>, name: &str, source: &str) -> rquickjs::Result<()> {
    ctx.eval_with_options::<(), _>(source, eval_options(name, false))
}

/// Global-scope evaluation options with a file name for stack traces.
pub(crate) fn eval_options(name: &str, strict: bool) -> EvalOptions {
    let mut opts = EvalOptions::default();
    opts.global = true;
    opts.strict = strict;
    opts.filename = Some(name.to_string());
    opts
}
