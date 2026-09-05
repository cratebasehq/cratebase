//! Compile-time Rust plugins.
//!
//! A plugin is, at its simplest, `fn(&App) -> Result<(), AppError>`: it
//! is handed the app during bootstrap and does whatever it likes with the
//! hooks, cron, store and router. Anything that also wants HTTP routes
//! implements the [`Plugin`] trait instead.
//!
//! ```ignore
//! fn audit(app: &App) -> Result<(), AppError> {
//!     app.hooks().on_record_create.bind_func(|e| Box::pin(async move {
//!         tracing::info!(id = e.record.id(), "creating");
//!         e.next().await
//!     }));
//!     Ok(())
//! }
//! app.register_plugin(FnPlugin::new("audit", audit))?;
//! ```
//!
//! # Routes live inside `/api`
//!
//! Plugin routers are nested at `/api/plugins/<name>` **inside** the same
//! nest as the built-in API, so they pass through request logging and the
//! rate limiter. An earlier design merged them at the router root instead,
//! which the audit flagged: a plugin route was neither logged nor
//! rate-limited.

use std::sync::Arc;

use axum::Router;
use cratebase_core::AppError;

use crate::app::App;

/// Names a plugin may not take, because they would collide with a
/// built-in API path or read as one.
pub const RESERVED_PLUGIN_NAMES: &[&str] = &[
    "admins",
    "backups",
    "batch",
    "collections",
    "crons",
    "files",
    "health",
    "logs",
    "plugins",
    "realtime",
    "records",
    "settings",
];

pub trait Plugin: Send + Sync {
    /// URL-safe identity; also the mount point (`/api/plugins/<name>`).
    fn name(&self) -> &str;

    /// One-time setup, run during [`App::bootstrap`] before the listener
    /// binds. Register hooks, cron jobs and store entries here.
    fn setup(&self, app: &App) -> Result<(), AppError> {
        let _ = app;
        Ok(())
    }

    /// Extra routes, mounted at `/api/plugins/<name>`.
    fn routes(&self) -> Option<Router<App>> {
        None
    }
}

/// The bare-function form of a plugin's setup step.
pub type SetupFn = dyn Fn(&App) -> Result<(), AppError> + Send + Sync;

/// Adapter turning a bare `fn(&App) -> Result<(), AppError>` into a
/// [`Plugin`].
pub struct FnPlugin {
    name: String,
    setup: Box<SetupFn>,
}

impl FnPlugin {
    pub fn new(
        name: impl Into<String>,
        setup: impl Fn(&App) -> Result<(), AppError> + Send + Sync + 'static,
    ) -> Self {
        FnPlugin {
            name: name.into(),
            setup: Box::new(setup),
        }
    }
}

impl Plugin for FnPlugin {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup(&self, app: &App) -> Result<(), AppError> {
        (self.setup)(app)
    }
}

#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Arc<dyn Plugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `plugin`, rejecting an empty, reserved or duplicate name.
    pub fn register(&mut self, plugin: impl Plugin + 'static) -> Result<(), AppError> {
        self.register_arc(Arc::new(plugin))
    }

    pub fn register_arc(&mut self, plugin: Arc<dyn Plugin>) -> Result<(), AppError> {
        let name = plugin.name().to_string();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(AppError::bad_request(format!(
                "invalid plugin name '{name}': use ASCII letters, digits, '-' and '_'"
            )));
        }
        if RESERVED_PLUGIN_NAMES.contains(&name.as_str()) {
            return Err(AppError::bad_request(format!(
                "plugin name '{name}' is reserved"
            )));
        }
        if self.plugins.iter().any(|p| p.name() == name) {
            return Err(AppError::bad_request(format!(
                "a plugin named '{name}' is already registered"
            )));
        }
        self.plugins.push(plugin);
        Ok(())
    }

    pub fn names(&self) -> Vec<String> {
        self.plugins.iter().map(|p| p.name().to_string()).collect()
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub(crate) fn clone_plugins(&self) -> Vec<Arc<dyn Plugin>> {
        self.plugins.clone()
    }

    /// Every plugin's routes, nested under `/plugins/<name>`. The caller
    /// merges this into the `/api` router.
    pub fn router(&self) -> Router<App> {
        let mut router = Router::new();
        for plugin in &self.plugins {
            if let Some(routes) = plugin.routes() {
                router = router.nest(&format!("/plugins/{}", plugin.name()), routes);
            }
        }
        router
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Named(&'static str);
    impl Plugin for Named {
        fn name(&self) -> &str {
            self.0
        }
    }

    #[test]
    fn names_must_be_unique_valid_and_unreserved() {
        let mut registry = PluginRegistry::new();
        registry.register(Named("webhooks")).unwrap();
        assert_eq!(registry.names(), ["webhooks"]);

        assert_eq!(
            registry.register(Named("webhooks")).unwrap_err().status(),
            400
        );
        assert_eq!(
            registry.register(Named("settings")).unwrap_err().status(),
            400
        );
        assert_eq!(
            registry.register(Named("bad name")).unwrap_err().status(),
            400
        );
        assert_eq!(registry.register(Named("")).unwrap_err().status(), 400);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn fn_plugins_run_their_closure() {
        let plugin = FnPlugin::new("noop", |_app| Ok(()));
        assert_eq!(plugin.name(), "noop");
    }
}
