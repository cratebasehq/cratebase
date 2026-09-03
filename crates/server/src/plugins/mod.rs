//! Built-in plugins and the registry wiring them into the server. See
//! [`crate::plugin`] for the extension-point contract itself.

mod example;

use crate::plugin::PluginRegistry;

/// The set of plugins compiled into this binary. Add your own module next
/// to `example` and register it here.
pub fn registry() -> PluginRegistry {
    PluginRegistry::new().register(example::ExamplePlugin)
}
