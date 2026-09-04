//! [`App`]: the handle every route, plugin and hook is handed.
//!
//! `App` is a cheap `Arc` clone of one `AppInner`. It is also the axum
//! router state, so a handler writes `State(app): State<App>` and gets
//! the database, storage, mailer, settings, hooks, cron and logger from
//! the same value — there is no second "AppState" type to keep in sync.
//!
//! Construction is split in two on purpose:
//!
//! * [`App::new`] only records the [`Config`]; it opens nothing, so a CLI
//!   subcommand can build an `App` and decide afterwards whether it needs
//!   a database at all.
//! * [`App::bootstrap`] does the I/O: connect, create the system tables,
//!   run the core migrations, load settings and the collection store,
//!   build storage/mailer from those settings, run plugin setup and start
//!   the cron ticker.
//!
//! Settings are hot: `PATCH /api/settings` persists, swaps the
//! `ArcSwap<Settings>` and rebuilds the settings-derived services
//! (storage, mailer, the backups cron), so no restart is needed.

use std::sync::Arc;
use std::sync::OnceLock;

use arc_swap::ArcSwap;
use cratebase_core::{AppError, Settings};
use cratebase_db::engine::{Executor, Row, Sql};
use cratebase_db::{params, Db, Transaction};
use cratebase_mailer::Mailer;
use cratebase_storage::Storage;

use crate::config::Config;
use crate::cron::{self, CronService};
use crate::hooks::Hooks;
use crate::middleware::rate_limit::RateLimiter;
use crate::middleware::request_log::LogWriter;
use crate::plugin::{Plugin, PluginRegistry};
use crate::store::Store;

/// Shared state behind every [`App`] clone.
pub struct AppInner {
    config: Config,
    /// Set exactly once by [`App::bootstrap`]. A restore replaces the
    /// files on disk and re-execs the process rather than swapping this,
    /// so `OnceLock` is enough and `db()` can hand out a plain reference.
    db: OnceLock<Db>,
    settings: ArcSwap<Settings>,
    storage: ArcSwap<Storage>,
    mailer: ArcSwap<Mailer>,
    logger: OnceLock<LogWriter>,
    /// `Arc` so the background window sweeper can hold a weak reference
    /// and stop itself when the app goes away.
    rate_limiter: Arc<RateLimiter>,
    cron: CronService,
    realtime: crate::realtime::RealtimeService,
    hooks: Hooks,
    store: Store,
    plugins: std::sync::Mutex<PluginRegistry>,
    bootstrapped: std::sync::atomic::AtomicBool,
    /// How many times a bearer token has actually been resolved (a JWT
    /// verify plus a database round trip), as opposed to being served
    /// from the per-request cache.
    ///
    /// This is a *performance invariant*, not a statistic: the Phase 1
    /// regression was the logging layer and the handler each resolving
    /// the caller independently, which doubled the cost of every
    /// authenticated request. One relaxed increment on a path that
    /// already does a signature check is free, and it lets a test assert
    /// "exactly one resolution per request" instead of trusting a
    /// comment.
    auth_resolutions: std::sync::atomic::AtomicU64,
    /// Set once the JS runtime starts (absent `pb_hooks/`, or an empty
    /// one, means it never does). An `Arc<OnceLock<_>>` rather than a
    /// plain field so hook bridges bound while the runtime's own hook
    /// files are still being evaluated (see `register_hook`) can hold a
    /// handle to it before `Runtime::start` has returned.
    jsvm: Arc<OnceLock<cratebase_jsvm::Runtime>>,
    /// Routes a `pb_hooks` file mounted with `routerAdd`, collected while
    /// the runtime starts and turned into real axum routes by
    /// `crate::jsvm_host::js_router` once `crate::router` assembles the
    /// server.
    js_routes: std::sync::Mutex<Vec<JsRoute>>,
}

#[derive(Clone)]
pub struct App {
    inner: Arc<AppInner>,
}

/// One `routerAdd` registration: a JS-runtime route waiting to be turned
/// into a real axum route by `crate::jsvm_host::js_router`.
#[derive(Clone)]
pub struct JsRoute {
    /// Upper-case HTTP method, or empty for "any method" (PocketBase
    /// accepts both from `routerAdd`).
    pub method: String,
    /// PocketBase-style path pattern (`/hello/{name}`, `/files/{path...}`).
    pub pattern: String,
    pub handler: cratebase_jsvm::RouteHandlerId,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("data_dir", &self.inner.config.data_dir)
            .field("bootstrapped", &self.is_bootstrapped())
            .finish_non_exhaustive()
    }
}

impl App {
    /// Build the handle. Performs no I/O: storage and mailer start as
    /// harmless placeholders and are replaced from the stored settings
    /// during [`App::bootstrap`].
    pub fn new(config: Config) -> App {
        let settings = Settings::default();
        let storage = Storage::local(
            config
                .data_path()
                .join(cratebase_storage::LOCAL_STORAGE_DIR),
        )
        .unwrap_or_else(|_| {
            // Only reachable when the data dir is unwritable; bootstrap
            // reports that properly, so don't panic here.
            Storage::local(std::env::temp_dir().join("cratebase-storage"))
                .expect("a writable temp directory")
        });
        let mailer = Mailer::from_settings(&settings.smtp, &settings.meta)
            .unwrap_or_else(|_| Mailer::with_backend(Arc::new(cratebase_mailer::LogBackend)));

        App {
            inner: Arc::new(AppInner {
                config,
                db: OnceLock::new(),
                settings: ArcSwap::from_pointee(settings),
                storage: ArcSwap::from_pointee(storage),
                mailer: ArcSwap::from_pointee(mailer),
                logger: OnceLock::new(),
                rate_limiter: Arc::new(RateLimiter::new()),
                cron: CronService::new(),
                realtime: crate::realtime::RealtimeService::new(),
                hooks: Hooks::new(),
                store: Store::new(),
                plugins: std::sync::Mutex::new(PluginRegistry::new()),
                bootstrapped: std::sync::atomic::AtomicBool::new(false),
                auth_resolutions: std::sync::atomic::AtomicU64::new(0),
                jsvm: Arc::new(OnceLock::new()),
                js_routes: std::sync::Mutex::new(Vec::new()),
            }),
        }
    }

    /// Bump the resolution counter. Called once per genuine token
    /// resolution; see [`App::auth_resolutions`].
    pub(crate) fn note_auth_resolution(&self) {
        self.inner
            .auth_resolutions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Total token resolutions since boot. A request must add at most one,
    /// however many extractors and layers ask who the caller is.
    pub fn auth_resolutions(&self) -> u64 {
        self.inner
            .auth_resolutions
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    // ------------------------------------------------------------ accessors

    /// The open database. Panics when called before [`App::bootstrap`],
    /// which is a programming error rather than a runtime condition.
    pub fn db(&self) -> &Db {
        self.inner
            .db
            .get()
            .expect("App::db() called before App::bootstrap()")
    }

    pub fn try_db(&self) -> Option<&Db> {
        self.inner.db.get()
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// The current settings snapshot. Cheap; re-read it per request
    /// rather than caching, so a `PATCH /api/settings` takes effect
    /// immediately.
    pub fn settings(&self) -> Arc<Settings> {
        self.inner.settings.load_full()
    }

    /// The record-file store. Returned by value because
    /// [`App::set_settings`] can swap it (S3 turned on/off at runtime).
    pub fn storage(&self) -> Arc<Storage> {
        self.inner.storage.load_full()
    }

    pub fn mailer(&self) -> Arc<Mailer> {
        self.inner.mailer.load_full()
    }

    /// The backups object store, derived from `settings.backups.s3`.
    /// Rebuilt per call because backups are rare and the settings may
    /// have changed since the last one.
    pub fn backups_storage(&self) -> Result<Storage, AppError> {
        Storage::backups_from_settings(&self.settings().backups, self.config().data_path())
            .map_err(|e| AppError::internal(e.to_string()))
    }

    pub fn logger(&self) -> &LogWriter {
        self.inner.logger.get_or_init(LogWriter::disabled)
    }

    pub fn rate_limiter(&self) -> &RateLimiter {
        &self.inner.rate_limiter
    }

    pub fn cron(&self) -> &CronService {
        &self.inner.cron
    }

    /// The realtime (SSE) client registry.
    pub fn realtime(&self) -> &crate::realtime::RealtimeService {
        &self.inner.realtime
    }

    pub fn hooks(&self) -> &Hooks {
        &self.inner.hooks
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    /// The running JS runtime, once `App::bootstrap` has started one.
    /// `None` when no `pb_hooks` directory exists (or it is empty) — the
    /// documented zero-cost case, so nothing here ever starts a runtime
    /// speculatively.
    pub fn jsvm(&self) -> Option<cratebase_jsvm::Runtime> {
        self.inner.jsvm.get().cloned()
    }

    /// A handle a hook bridge can hold onto *before* the runtime it will
    /// eventually call into has finished starting (see
    /// `crate::hooks::bind_js_hook`): registration happens synchronously
    /// while `Runtime::start` evaluates the hook files, which is before
    /// the `Runtime` value it returns exists.
    pub(crate) fn jsvm_cell(&self) -> Arc<OnceLock<cratebase_jsvm::Runtime>> {
        self.inner.jsvm.clone()
    }

    /// Record a `routerAdd` registration. Turned into a real route by
    /// `crate::jsvm_host::js_router` when `crate::router` assembles the
    /// server, which happens once every `pb_hooks` file has already run.
    pub(crate) fn push_js_route(&self, route: JsRoute) {
        self.inner
            .js_routes
            .lock()
            .expect("js route registry poisoned")
            .push(route);
    }

    pub fn js_routes(&self) -> Vec<JsRoute> {
        self.inner
            .js_routes
            .lock()
            .expect("js route registry poisoned")
            .clone()
    }

    pub fn is_bootstrapped(&self) -> bool {
        self.inner
            .bootstrapped
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Register a plugin. Must happen before [`App::bootstrap`], which is
    /// where every registered plugin's `setup` runs.
    pub fn register_plugin(&self, plugin: impl Plugin + 'static) -> Result<(), AppError> {
        self.inner
            .plugins
            .lock()
            .expect("plugin registry poisoned")
            .register(plugin)
    }

    pub fn plugin_names(&self) -> Vec<String> {
        self.inner
            .plugins
            .lock()
            .expect("plugin registry poisoned")
            .names()
    }

    /// Every plugin's routes, already nested under `/plugins/<name>`.
    pub fn plugin_router(&self) -> axum::Router<App> {
        self.inner
            .plugins
            .lock()
            .expect("plugin registry poisoned")
            .router()
    }

    /// The HS256 key for a record token: the app secret, the record's
    /// `tokenKey` and the collection's per-type secret, per spec §2.
    /// Rotating a record's `tokenKey` therefore invalidates its sessions.
    pub fn token_signing_key(&self, record_token_key: &str, type_secret: &str) -> Vec<u8> {
        cratebase_auth::signing_key(&self.inner.config.secret, record_token_key, type_secret)
    }

    // ------------------------------------------------------------ lifecycle

    /// Open everything and get the app ready to serve. Idempotent-ish:
    /// calling it twice is refused rather than silently reopening the
    /// database.
    pub async fn bootstrap(&self) -> anyhow::Result<()> {
        if self
            .inner
            .bootstrapped
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            anyhow::bail!("App::bootstrap() called twice");
        }

        let mut event = crate::events::BootstrapEvent::new(self.clone());
        let app = self.clone();
        self.hooks()
            .on_bootstrap
            .trigger(&mut event, move |_| {
                Box::pin(async move { app.bootstrap_inner().await })
            })
            .await?;
        Ok(())
    }

    async fn bootstrap_inner(&self) -> Result<(), AppError> {
        let config = &self.inner.config;
        if !config.data_dir.is_empty() {
            std::fs::create_dir_all(&config.data_dir)
                .map_err(|e| AppError::internal(format!("cannot create data dir: {e}")))?;
        }

        let db = Db::connect(&config.database_url, &config.data_dir)
            .await
            .map_err(AppError::from)?;
        // Creates `_collections`/`_params`/`_migrations`/`_logs`, runs the
        // core migrations (which seed `_superusers`, `users` and the other
        // system collections) and loads the collection store.
        db.bootstrap().await.map_err(AppError::from)?;
        let _ = self.inner.db.set(db);

        // Settings: whatever is stored wins; a first boot seeds from env.
        let stored = params::get(self.db(), params::SETTINGS_KEY)
            .await
            .map_err(AppError::from)?;
        let settings = match stored {
            Some(raw) if !raw.trim().is_empty() => serde_json::from_str::<Settings>(&raw)
                .map_err(|e| AppError::internal(format!("stored settings are corrupt: {e}")))?,
            _ => {
                let seeded = config.seed_settings();
                params::save_settings(self.db(), &seeded)
                    .await
                    .map_err(AppError::from)?;
                seeded
            }
        };
        self.apply_settings(Arc::new(settings))?;

        // Starts the JS runtime when `pb_hooks/` exists and has at least
        // one `*.pb.js` file; a no-op otherwise (spec: absent `pb_hooks`
        // must cost nothing). Must run before `register_system_crons` /
        // `plugin.setup` below so a `pb_hooks` file's `cronAdd`/`onRecord*`
        // registrations are in place before anything can trigger them, and
        // after settings/collections are loaded so a hook file that reads
        // `$app.*` at evaluation time sees a working database.
        crate::jsvm_host::maybe_start(self).await?;

        let logger = if config.log_requests {
            LogWriter::spawn(self.db().clone())
        } else {
            LogWriter::disabled()
        };
        let _ = self.inner.logger.set(logger);

        self.inner.rate_limiter.start_sweeper();
        self.register_system_crons();
        self.sync_backup_cron();
        crate::cron_jobs::bind_hooks(self);
        crate::cron_jobs::sync_all(self).await;
        crate::webhooks::bind_hooks(self);
        crate::teams::bind_hooks(self);
        // Postgres only (see `crate::realtime`'s module doc); a no-op on
        // SQLite because `Engine::subscribe_realtime`'s default is.
        crate::realtime::start_cross_node_listener(self);
        crate::push::bind_hooks(self);

        let plugins = {
            let guard = self.inner.plugins.lock().expect("plugin registry poisoned");
            guard.clone_plugins()
        };
        for plugin in plugins {
            plugin.setup(self)?;
        }

        self.inner.cron.start();

        let mut reload = crate::events::SettingsReloadEvent::new(self.clone(), self.settings());
        self.hooks()
            .on_settings_reload
            .trigger_bare(&mut reload)
            .await?;
        Ok(())
    }

    /// Bootstrap, assemble the router, fire `on_serve` (where plugins add
    /// their routes) and listen.
    pub async fn serve(self) -> anyhow::Result<()> {
        self.bootstrap().await?;

        let address = format!("{}:{}", self.config().host, self.config().port);
        let router = crate::router(self.clone());
        let mut event = crate::events::ServeEvent::new(self.clone(), router, address.clone());
        self.hooks()
            .on_serve
            .trigger(&mut event, |_| Box::pin(async { Ok(()) }))
            .await?;

        let listener = tokio::net::TcpListener::bind(&event.address).await?;
        tracing::info!(address = %event.address, "cratebase listening");
        let router = std::mem::take(&mut event.router);
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await?;

        self.terminate(false).await;
        Ok(())
    }

    /// Fire `on_terminate` and close the database. `is_restart` is true
    /// when the process is about to re-exec itself after a restore.
    pub async fn terminate(&self, is_restart: bool) {
        let mut event = crate::events::TerminateEvent::new(self.clone(), is_restart);
        if let Err(e) = self.hooks().on_terminate.trigger_bare(&mut event).await {
            tracing::warn!(error = %e, "on_terminate handler failed");
        }
        // Stopped after `on_terminate` so a JS `onTerminate` handler still
        // runs, and before the database closes since a worker thread may
        // still be mid-call.
        if let Some(rt) = self.jsvm() {
            rt.stop();
        }
        self.logger().flush().await;
        if let Some(db) = self.try_db() {
            if let Err(e) = db.close().await {
                tracing::warn!(error = %e, "closing the database failed");
            }
        }
    }

    // ------------------------------------------------------------- settings

    /// Persist `settings`, swap them in and rebuild everything derived
    /// from them (storage, mailer, the backups cron).
    pub async fn set_settings(&self, settings: Settings) -> anyhow::Result<()> {
        params::save_settings(self.db(), &settings).await?;
        self.apply_settings(Arc::new(settings))?;
        self.sync_backup_cron();
        let mut event = crate::events::SettingsReloadEvent::new(self.clone(), self.settings());
        self.hooks()
            .on_settings_reload
            .trigger_bare(&mut event)
            .await?;
        Ok(())
    }

    /// Swap in `settings` without persisting (used by bootstrap and by
    /// tests).
    ///
    /// Everything derived from settings is rebuilt *here*, once, so the
    /// request path never has to look at the raw `Settings` — notably the
    /// rate limiter, which compiles its rules into lookup tables.
    pub fn apply_settings(&self, settings: Arc<Settings>) -> Result<(), AppError> {
        let storage = Storage::from_settings(&settings.s3, self.config().data_path())
            .map_err(|e| AppError::bad_request(format!("invalid S3 settings: {e}")))?;
        let mailer = Mailer::from_settings(&settings.smtp, &settings.meta)
            .map_err(|e| AppError::bad_request(format!("invalid SMTP settings: {e}")))?;
        self.inner.rate_limiter.configure(&settings.rate_limits);
        self.inner.storage.store(Arc::new(storage));
        self.inner.mailer.store(Arc::new(mailer));
        self.inner.settings.store(settings);
        Ok(())
    }

    // -------------------------------------------------------------- crons

    /// PocketBase's four system jobs, registered under their exact ids so
    /// `GET /api/crons` matches the fixture.
    fn register_system_crons(&self) {
        let app = self.clone();
        let _ = self
            .inner
            .cron
            .add(cron::JOB_DB_OPTIMIZE, "0 0 * * *", move || {
                let app = app.clone();
                async move {
                    if let Err(e) = app.db().engine.optimize().await {
                        tracing::warn!(error = %e, "database optimize failed");
                    }
                }
            });

        // `_mfas` / `_otps` rows are short-lived; each auth collection has
        // its own configured duration, and both system tables are shared
        // across every auth collection, so the sweep walks the collection
        // store rather than issuing one blanket cutoff.
        let app = self.clone();
        let _ = self
            .inner
            .cron
            .add(cron::JOB_MFA_CLEANUP, "0 * * * *", move || {
                let app = app.clone();
                async move {
                    for collection in app.db().collections.all().all.iter() {
                        if !collection.is_auth() || !collection.auth.mfa.enabled {
                            continue;
                        }
                        let cutoff = cratebase_core::DateTime::from_utc(
                            chrono::Utc::now()
                                - chrono::Duration::seconds(collection.auth.mfa.duration.max(1)),
                        );
                        let sql = r#"DELETE FROM "_mfas" WHERE "collectionRef" = $1 AND "created" < $2"#;
                        if let Err(e) = app
                            .db()
                            .execute(
                                sql,
                                &[
                                    Sql::Text(collection.id.clone()),
                                    Sql::Text(cutoff.to_pb_string()),
                                ],
                            )
                            .await
                        {
                            tracing::warn!(error = %e, collection = %collection.name, "mfa cleanup failed");
                        }
                    }
                }
            });
        let app = self.clone();
        let _ = self
            .inner
            .cron
            .add(cron::JOB_OTP_CLEANUP, "0 * * * *", move || {
                let app = app.clone();
                async move {
                    for collection in app.db().collections.all().all.iter() {
                        if !collection.is_auth() || !collection.auth.otp.enabled {
                            continue;
                        }
                        let cutoff = cratebase_core::DateTime::from_utc(
                            chrono::Utc::now()
                                - chrono::Duration::seconds(collection.auth.otp.duration.max(1)),
                        );
                        let sql = r#"DELETE FROM "_otps" WHERE "collectionRef" = $1 AND "created" < $2"#;
                        if let Err(e) = app
                            .db()
                            .execute(
                                sql,
                                &[
                                    Sql::Text(collection.id.clone()),
                                    Sql::Text(cutoff.to_pb_string()),
                                ],
                            )
                            .await
                        {
                            tracing::warn!(error = %e, collection = %collection.name, "otp cleanup failed");
                        }
                    }
                }
            });

        let app = self.clone();
        let _ = self
            .inner
            .cron
            .add(cron::JOB_LOGS_CLEANUP, "0 */6 * * *", move || {
                let app = app.clone();
                async move {
                    let days = app.settings().logs.max_days;
                    if days <= 0 {
                        return;
                    }
                    match cratebase_db::logs::delete_older_than(&*app.db().logs, days).await {
                        Ok(n) if n > 0 => tracing::info!(removed = n, "pruned old logs"),
                        Ok(_) => {}
                        Err(e) => tracing::warn!(error = %e, "logs cleanup failed"),
                    }
                }
            });
    }

    /// Add/remove the scheduled-backup job to match `settings.backups.cron`.
    /// The id is PocketBase's `__pbAutoBackup__`, and the job only appears
    /// in `GET /api/crons` once a cron expression is configured.
    pub fn sync_backup_cron(&self) {
        const ID: &str = crate::cron::JOB_AUTO_BACKUP;
        let expression = self.settings().backups.cron.trim().to_string();
        if expression.is_empty() {
            self.inner.cron.remove(ID);
            return;
        }
        let app = self.clone();
        if let Err(e) = self.inner.cron.add(ID, &expression, move || {
            let app = app.clone();
            async move {
                if let Err(e) = crate::routes::backups::create_scheduled(&app).await {
                    tracing::error!(error = %e, "scheduled backup failed");
                }
            }
        }) {
            tracing::warn!(error = %e, "ignoring invalid settings.backups.cron");
        }
    }

    // ---------------------------------------------------------- transactions

    /// Run `f` inside one write transaction, committing on `Ok` and
    /// rolling back on `Err`. The closure receives a [`TxApp`], so hooks
    /// and services called from inside it read and write through the same
    /// transaction instead of racing it on another connection.
    pub async fn run_in_transaction<F, Fut, T>(&self, f: F) -> Result<T, AppError>
    where
        F: FnOnce(TxApp) -> Fut + Send,
        Fut: std::future::Future<Output = Result<T, AppError>> + Send,
        T: Send,
    {
        self.run_scoped(true, f).await
    }

    /// [`run_in_transaction`](App::run_in_transaction) with an opt-out.
    ///
    /// When `transactional` is false no transaction is opened and the
    /// [`TxApp`]'s statements run in autocommit mode on the engine. Only
    /// pass false when the closure issues **one** statement and nothing
    /// else (no hook, no cascade) can join or abort it: the whole point
    /// of the transaction is to make several statements — or a statement
    /// plus a hook that may fail after it — atomic, and a single
    /// statement already is.
    ///
    /// This is worth an opt-out because the transaction is not free:
    /// `BEGIN IMMEDIATE` and `COMMIT` are two extra round trips onto the
    /// blocking pool, with the single writer connection locked across all
    /// three. Measured straight on the SQLite engine (16 readers, 6000
    /// rows), one `DELETE` costs 83µs wrapped in a transaction against
    /// 59µs in autocommit: a writer ceiling of ~12.0k versus ~17.0k
    /// deletes per second, at every concurrency from 1 to 50.
    pub async fn run_scoped<F, Fut, T>(&self, transactional: bool, f: F) -> Result<T, AppError>
    where
        F: FnOnce(TxApp) -> Fut + Send,
        Fut: std::future::Future<Output = Result<T, AppError>> + Send,
        T: Send,
    {
        let tx = if transactional {
            Some(self.db().begin().await.map_err(AppError::from)?)
        } else {
            None
        };
        let handle = Arc::new(TxHandle {
            tx: tokio::sync::Mutex::new(tx),
            transactional,
        });
        let tx_app = TxApp {
            app: self.clone(),
            handle: handle.clone(),
        };
        match f(tx_app).await {
            Ok(value) => {
                handle.commit().await?;
                Ok(value)
            }
            Err(e) => {
                handle.rollback().await;
                Err(e)
            }
        }
    }

    // ------------------------------------------------- superusers (W4a only)

    /// Look up a `_superusers` row by id, straight through the engine.
    ///
    /// The request path uses `records::find_by_id_raw` instead (it decodes
    /// into a `Record` and applies the field types); this raw form stays
    /// for the CLI, which runs before any collection is resolved.
    pub async fn find_superuser_by_id(&self, id: &str) -> Result<Option<Row>, AppError> {
        self.db()
            .query_one(
                r#"SELECT * FROM "_superusers" WHERE "id" = $1"#,
                &[Sql::from(id)],
            )
            .await
            .map_err(AppError::from)
    }

    /// Look up a `_superusers` row by email. See
    /// [`App::find_superuser_by_id`] for why this stays raw.
    pub async fn find_superuser_by_email(&self, email: &str) -> Result<Option<Row>, AppError> {
        self.db()
            .query_one(
                r#"SELECT * FROM "_superusers" WHERE "email" = $1"#,
                &[Sql::from(email)],
            )
            .await
            .map_err(AppError::from)
    }

    /// Insert a `_superusers` record. Used by `cratebase superuser create`
    /// and by the test harness.
    pub async fn create_superuser(&self, email: &str, password: &str) -> Result<String, AppError> {
        // Argon2id is deliberately expensive; the async wrapper keeps it
        // on the blocking pool so a login burst cannot stall the runtime.
        let hash = cratebase_auth::hash_password_async(password)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        let id = cratebase_core::record_id();
        let now = cratebase_core::DateTime::now().to_pb_string();
        self.db()
            .execute(
                r#"INSERT INTO "_superusers"
                   ("id", "email", "password", "tokenKey", "emailVisibility", "verified", "created", "updated")
                   VALUES ($1, $2, $3, $4, 0, 1, $5, $6)"#,
                &[
                    Sql::Text(id.clone()),
                    Sql::from(email),
                    Sql::Text(hash),
                    Sql::Text(new_token_key()),
                    Sql::Text(now.clone()),
                    Sql::Text(now),
                ],
            )
            .await
            .map_err(AppError::from)?;
        Ok(id)
    }

    /// Replace a superuser's password. `tokenKey` is rotated too, which is
    /// what invalidates every existing session for the account.
    pub async fn set_superuser_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<bool, AppError> {
        let hash = cratebase_auth::hash_password_async(password)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        let updated = self
            .db()
            .execute(
                r#"UPDATE "_superusers" SET "password" = $1, "tokenKey" = $2, "updated" = $3
                   WHERE "email" = $4"#,
                &[
                    Sql::Text(hash),
                    Sql::Text(new_token_key()),
                    Sql::Text(cratebase_core::DateTime::now().to_pb_string()),
                    Sql::from(email),
                ],
            )
            .await
            .map_err(AppError::from)?;
        Ok(updated > 0)
    }

    pub async fn delete_superuser(&self, email: &str) -> Result<bool, AppError> {
        let removed = self
            .db()
            .execute(
                r#"DELETE FROM "_superusers" WHERE "email" = $1"#,
                &[Sql::from(email)],
            )
            .await
            .map_err(AppError::from)?;
        Ok(removed > 0)
    }

    /// Mint a token for a record of `collection_name`, signed with the
    /// record's own `tokenKey` (spec §2). Note that the claims carry no
    /// `refreshable` flag; `routes::auth` mints session tokens itself
    /// with `new_auth_claims` so the SDK sees PocketBase's exact payload.
    pub async fn mint_token(
        &self,
        collection_name: &str,
        record_id: &str,
        token_type: cratebase_auth::TokenType,
        duration_secs: i64,
    ) -> Result<String, AppError> {
        let collection = self
            .db()
            .collections
            .get(collection_name)
            .ok_or_else(|| AppError::not_found("Missing or invalid collection context."))?;
        let row = self
            .db()
            .query_one(
                &format!(
                    r#"SELECT "tokenKey" FROM {} WHERE "id" = $1"#,
                    cratebase_db::quote_ident(collection.table_name())
                ),
                &[Sql::from(record_id)],
            )
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::not_found(""))?;
        let token_key = row.get_str("tokenKey").unwrap_or_default();
        let config = match token_type {
            cratebase_auth::TokenType::File => &collection.auth.file_token,
            _ => &collection.auth.auth_token,
        };
        let claims = cratebase_auth::Claims::new(
            record_id,
            token_type,
            collection.id.clone(),
            if duration_secs > 0 {
                duration_secs
            } else {
                config.duration.max(1)
            },
        );
        cratebase_auth::sign(&claims, &self.token_signing_key(token_key, &config.secret))
            .map_err(|e| AppError::internal(e.to_string()))
    }
}

/// A fresh 50-character `tokenKey`, PocketBase's
/// `autogeneratePattern: [a-zA-Z0-9]{50}`.
pub fn new_token_key() -> String {
    cratebase_core::ids::random_string(
        50,
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
    )
}

/// The open transaction shared by every [`TxApp`] clone in one
/// `run_in_transaction` scope.
struct TxHandle {
    tx: tokio::sync::Mutex<Option<Transaction>>,
    /// False for a scope opened by [`App::run_scoped`] with
    /// `transactional = false`: `tx` is then permanently `None` and the
    /// [`TxApp`] executes straight on the engine. Kept as its own flag so
    /// "no transaction was ever opened" is distinguishable from "the
    /// transaction has already been committed", which must still be an
    /// error.
    transactional: bool,
}

impl TxHandle {
    async fn commit(&self) -> Result<(), AppError> {
        match self.tx.lock().await.take() {
            Some(tx) => tx.commit().await.map_err(AppError::from),
            None => Ok(()),
        }
    }

    async fn rollback(&self) {
        if let Some(tx) = self.tx.lock().await.take() {
            if let Err(e) = tx.rollback().await {
                tracing::warn!(error = %e, "transaction rollback failed");
            }
        }
    }
}

/// An [`App`] bound to an open transaction. Record and collection hooks
/// receive this rather than the plain `App`, so a handler's own writes
/// commit or roll back together with the operation that triggered it.
#[derive(Clone)]
pub struct TxApp {
    app: App,
    handle: Arc<TxHandle>,
}

impl TxApp {
    pub fn app(&self) -> &App {
        &self.app
    }
}

impl std::ops::Deref for TxApp {
    type Target = App;
    fn deref(&self) -> &App {
        &self.app
    }
}

#[async_trait::async_trait]
impl Executor for TxApp {
    fn dialect(&self) -> cratebase_filter::Dialect {
        self.app.db().dialect()
    }

    async fn query(&self, sql: &str, params: &[Sql]) -> cratebase_db::DbResult<Vec<Row>> {
        if !self.handle.transactional {
            return self.app.db().query(sql, params).await;
        }
        let guard = self.handle.tx.lock().await;
        match guard.as_ref() {
            Some(tx) => tx.query(sql, params).await,
            None => Err(cratebase_db::DbError::other("transaction already finished")),
        }
    }

    async fn execute(&self, sql: &str, params: &[Sql]) -> cratebase_db::DbResult<u64> {
        if !self.handle.transactional {
            return self.app.db().execute(sql, params).await;
        }
        let guard = self.handle.tx.lock().await;
        match guard.as_ref() {
            Some(tx) => tx.execute(sql, params).await,
            None => Err(cratebase_db::DbError::other("transaction already finished")),
        }
    }
}

/// Ctrl-C / SIGTERM, so `serve` can shut the log writer down cleanly.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
