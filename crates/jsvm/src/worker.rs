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
use crate::host::{CronHandlerId, HookHandlerId, HostApi, RouteHandlerId};
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
    Migration {
        file: PathBuf,
        direction: MigrationDirection,
        reply: oneshot::Sender<Result<(), AppError>>,
    },
}

/// A message for one specific worker.
pub(crate) enum Control {
    Reload(oneshot::Sender<Result<(), AppError>>),
    Shutdown,
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
    pub host_stack: RefCell<Vec<Arc<dyn HostApi>>>,
    pub tx_stack: RefCell<Vec<PendingTx>>,
    /// Interrupt deadline for the running job.
    pub deadline: Rc<Cell<Option<Instant>>>,
    /// Ids reported to the host by this worker (worker 0 only), so a
    /// reload can retract them.
    pub registered_hooks: RefCell<Vec<String>>,
    pub registered_crons: RefCell<Vec<String>>,
}

impl WorkerState {
    pub fn current_host(&self) -> Arc<dyn HostApi> {
        self.host_stack
            .borrow()
            .last()
            .cloned()
            .unwrap_or_else(|| self.host.clone())
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
    let loaded = worker.load_hooks();
    let _ = ready.send(loaded);

    loop {
        crossbeam_channel::select! {
            recv(control) -> msg => match msg {
                Ok(Control::Reload(reply)) => { let _ = reply.send(worker.reload()); }
                Ok(Control::Shutdown) | Err(_) => break,
            },
            recv(jobs) -> job => match job {
                Ok(job) => worker.execute(job),
                Err(_) => break,
            },
        }
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
        {
            // The interrupt handler runs on this thread only, so a plain
            // Rc is fine; it must be 'static though, hence the clone.
            let deadline = deadline.clone();
            runtime.set_interrupt_handler(Some(Box::new(move || {
                deadline.get().is_some_and(|d| Instant::now() >= d)
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
            tx_stack: RefCell::new(vec![]),
            deadline,
            registered_hooks: RefCell::new(vec![]),
            registered_crons: RefCell::new(vec![]),
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
                let timed_out = self
                    .state
                    .deadline
                    .get()
                    .is_some_and(|d| Instant::now() >= d);
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
            self.state.host_stack.borrow_mut().push(app);
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

    fn run_migration(&self, file: &PathBuf, direction: MigrationDirection) -> Result<(), AppError> {
        let source = std::fs::read_to_string(file)
            .map_err(|e| AppError::internal(format!("cannot read {}: {e}", file.display())))?;
        let name = file_name(file);
        self.timed(|ctx| {
            let cb = Self::cb(ctx)?;
            cb.set("currentFile", name.as_str())?;
            cb.set("pendingMigration", JsValue::new_null(ctx.clone()))?;
            eval_file(ctx, &name, &source)?;
            let run: Function = cb.get("runMigration")?;
            run.call::<_, ()>((direction.as_str(),))
        })
    }

    /// Evaluate every `*.pb.js` in `hooks_dir`, in file-name order.
    fn load_hooks(&self) -> Result<(), AppError> {
        let files = list_files(&self.state.cfg.hooks_dir, |n| n.ends_with(".pb.js"))?;
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
        first_error.map_or(Ok(()), Err)
    }

    /// Forget every handler, retract this worker's registrations from the
    /// host, and evaluate the hook files again.
    fn reload(&self) -> Result<(), AppError> {
        if self.state.reports() {
            for id in self.state.registered_hooks.borrow_mut().drain(..) {
                self.state.host.unregister_hook(&id);
            }
            for id in self.state.registered_crons.borrow_mut().drain(..) {
                self.state.host.remove_cron(&id);
            }
        }
        self.with_ctx(|ctx| {
            let reset: Function = Self::cb(ctx)?.get("reset")?;
            reset.call::<_, ()>(())
        })?;
        self.load_hooks()
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
