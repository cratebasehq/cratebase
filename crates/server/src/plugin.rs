//! Extension points for code that isn't part of Cratebase's core REST API:
//! extra HTTP routes and scheduled (cron-like) background tasks.
//!
//! This is intentionally a Rust trait, not a dynamically loaded plugin
//! format — Cratebase has no scripting runtime (unlike PocketBase's JSVM),
//! so "installing a plugin" means adding a crate/module that implements
//! [`Plugin`] and registering it in [`crate::plugins::registry`] at build
//! time, then shipping your own binary. This is the seam that a future
//! cron-job feature and admin team-management should be built on, rather
//! than growing bespoke code paths in the core server.
//!
//! See `crates/server/src/plugins/example.rs` for a minimal reference
//! plugin exercising both extension points.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use axum::Router;

use crate::state::AppState;

/// A scheduled task run on a fixed interval for as long as the server is
/// up. Not a full cron expression scheduler (no calendar semantics like
/// "every Monday at 9am") — just a `(name, interval, async fn(AppState))`
/// tuple, which covers the overwhelming majority of real maintenance jobs
/// (PocketBase's own built-in cron jobs are all fixed-interval sweeps too).
pub struct ScheduledTask {
    pub name: &'static str,
    pub interval: Duration,
    pub run: fn(AppState) -> Pin<Box<dyn Future<Output = ()> + Send>>,
}

/// Implement this to extend the server: mount extra routes, run scheduled
/// background work, or (in the future) hook into record lifecycle events.
/// Every method has a default no-op so a plugin only needs to implement
/// the extension points it actually uses.
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;

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
