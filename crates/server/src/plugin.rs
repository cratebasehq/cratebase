//! Extension points for code that isn't part of Cratebase's core REST API:
//! one-time startup setup, extra HTTP routes, and scheduled (cron-like)
//! background tasks.
//!
//! This is a compile-time Rust trait, not a dynamically loaded plugin
//! format: "installing a plugin" means implementing [`Plugin`] and
//! registering it in a [`PluginRegistry`], then shipping your own binary.
//! That binary doesn't have to be this repo's own `cratebase` binary,
//! either — `cratebase-server` is a normal library crate. A downstream
//! project can depend on it, implement its own [`Plugin`], and write a
//! small `main.rs` of its own:
//!
//! ```ignore
//! let state = cratebase_server::build_state(config).await?;
//! let plugins = cratebase_server::plugin::PluginRegistry::new()
//!     .register(MyPlugin);
//! plugins.setup_all(&state.db).await?;
//! plugins.spawn_tasks(&state);
//! let app = cratebase_server::build_app(state, &plugins);
//! axum::serve(listener, app).await?;
//! ```
//!
//! without ever touching this repo's own source. `cratebase_server::plugins::registry()`
//! returns the built-in set (see `crates/server/src/plugins/mod.rs`) if a
//! downstream binary wants those plus its own — `PluginRegistry::register`
//! composes.
//!
//! See `crates/server/src/plugins/example.rs` for a minimal reference
//! plugin exercising every extension point, and `feature_flags.rs` /
//! `cron_jobs.rs` for two that also provision their own collection.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use axum::Router;
use cratebase_db::Db;

use crate::state::AppState;

/// A scheduled task run on a fixed interval for as long as the server is
/// up. Not a full cron expression scheduler (no calendar semantics like
/// "every Monday at 9am") — just a `(name, interval, async fn(AppState))`
/// tuple, which covers the overwhelming majority of real maintenance jobs.
/// A plugin that needs real calendar cron expressions builds it on top of
/// this primitive (see `crates/server/src/plugins/cron_jobs.rs`) rather
/// than the trait growing calendar semantics of its own.
pub struct ScheduledTask {
    pub name: &'static str,
    pub interval: Duration,
    pub run: fn(AppState) -> Pin<Box<dyn Future<Output = ()> + Send>>,
}

/// Implement this to extend the server: provision anything the plugin
/// needs before it starts serving traffic, mount extra routes, or run
/// scheduled background work. Every method has a default no-op so a
/// plugin only needs to implement the extension points it actually uses.
///
/// Record lifecycle hooks (`on_create`/`on_update`/`on_delete`) are
/// deliberately not on this trait — add them when a real plugin needs one,
/// not speculatively.
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;

    /// One-time async setup run once at startup, before the server starts
    /// accepting connections — e.g. seeding a collection this plugin's
    /// routes or scheduled tasks depend on existing. Called by
    /// [`PluginRegistry::setup_all`].
    fn setup<'a>(
        &'a self,
        _db: &'a Db,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    /// Extra routes mounted under `/api/plugins/<name>/...` by the
    /// registry — see [`PluginRegistry::router`].
    fn routes(&self) -> Option<Router<AppState>> {
        None
    }

    /// Background jobs to run on a fixed interval for the life of the
    /// process. Spawned once at startup by [`PluginRegistry::spawn_tasks`].
    fn scheduled_tasks(&self) -> Vec<ScheduledTask> {
        Vec::new()
    }
}

/// Collects every registered [`Plugin`] and wires their extension points
/// into the running server.
#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(mut self, plugin: impl Plugin + 'static) -> Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    /// Runs every plugin's one-time startup setup, in registration order.
    /// Call once after `build_state`, before `spawn_tasks`/`build_app`.
    pub async fn setup_all(&self, db: &Db) -> anyhow::Result<()> {
        for plugin in &self.plugins {
            plugin.setup(db).await?;
        }
        Ok(())
    }

    /// Merges every plugin's routes under `/api/plugins/<plugin-name>`.
    pub fn router(&self) -> Router<AppState> {
        let mut router = Router::new();
        for plugin in &self.plugins {
            if let Some(plugin_router) = plugin.routes() {
                router = router.nest(&format!("/api/plugins/{}", plugin.name()), plugin_router);
            }
        }
        router
    }

    /// Spawns every plugin's scheduled tasks as background tokio tasks.
    /// Call once at startup after the app state is built; fire-and-forget
    /// for the life of the process (no cancellation — the whole process
    /// exits together).
    pub fn spawn_tasks(&self, state: &AppState) {
        for plugin in &self.plugins {
            for task in plugin.scheduled_tasks() {
                let state = state.clone();
                tokio::spawn(async move {
                    let mut ticker = tokio::time::interval(task.interval);
                    // The first tick fires immediately; skip it so a task
                    // doesn't run at t=0 before the server has accepted any
                    // traffic, only every `interval` after that.
                    ticker.tick().await;
                    loop {
                        ticker.tick().await;
                        (task.run)(state.clone()).await;
                    }
                });
            }
        }
    }
}
