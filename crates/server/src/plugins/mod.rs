//! Built-in plugins and the registry wiring them into the server. See
//! [`crate::plugin`] for the extension-point contract itself.

mod cron_jobs;
mod example;
mod feature_flags;
mod queue;

use crate::plugin::PluginRegistry;

/// The set of plugins compiled into this binary. Add your own module next
/// to `example` and register it here — or, for a downstream binary that
/// doesn't want to fork this repo at all, depend on `cratebase-server` as
/// a library and build your own `PluginRegistry` (see `build_app`'s doc
/// comment).
pub fn registry() -> PluginRegistry {
    PluginRegistry::new()
        .register(example::ExamplePlugin)
        .register(feature_flags::FeatureFlagsPlugin)
        .register(cron_jobs::CronJobsPlugin)
        .register(queue::QueuePlugin)
}
