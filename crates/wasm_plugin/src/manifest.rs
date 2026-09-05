//! `plugin.toml`: what a plugin is and what it is allowed to touch.
//!
//! Loading a plugin never grants it anything beyond what its manifest
//! declares. The manifest is read and shown (by `cratebase plugin
//! install`) *before* the `.wasm` file is ever instantiated, so a person
//! installing a plugin sees "wants to read: notes" as plain text, not as
//! a side effect they discover later. [`crate::runtime::PluginInstance`]
//! re-checks every capability at call time — the manifest is not merely
//! documentation, `host_get_record` refuses a collection that is not
//! listed in `records_read` regardless of what the `.wasm` asks for.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Mirrors `cratebase_server::plugin::RESERVED_PLUGIN_NAMES`. Duplicated
/// rather than depending on the `server` crate (that would be a
/// dependency cycle: `server` depends on this crate, not the other way
/// around); the two lists are pinned equal by
/// `crates/server/src/plugin_wasm.rs`'s tests.
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

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("cannot read manifest: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot parse manifest: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid plugin name '{0}': use ASCII letters, digits, '-' and '_'")]
    InvalidName(String),
    #[error("plugin name '{0}' is reserved")]
    ReservedName(String),
    #[error("manifest declares no capabilities.routes; a plugin must expose at least one route")]
    NoRoutes,
    #[error("route '{0}' is not URL-safe: use ASCII letters, digits, '-', '_' and '/'")]
    InvalidRoute(String),
}

/// What the plugin declares it wants. Every field here is a narrow,
/// explicit grant — there is no "raw database access" or "raw HTTP
/// egress" capability at all, on purpose. This is the exact list a
/// `cratebase plugin install` run prints for a human to approve.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// Sub-paths mounted at `/api/plugins/<name>/<route>`. Every method
    /// on a declared route reaches the plugin; the plugin's own code
    /// decides what to do with the method.
    #[serde(default)]
    pub routes: Vec<String>,
    /// Collections `host_get_record` may read from, rule-enforced
    /// exactly as `GET /api/collections/{c}/records/{id}` enforces them
    /// for the caller the plugin was invoked on behalf of. Empty means
    /// the plugin can register routes but never read a record.
    #[serde(default)]
    pub records_read: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// `.wasm` file path, relative to the manifest.
    pub entry: String,
    #[serde(default)]
    pub capabilities: Capabilities,
}

impl Manifest {
    pub fn parse(raw: &str) -> Result<Self, ManifestError> {
        let manifest: Manifest = toml::from_str(raw)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let raw = std::fs::read_to_string(path)?;
        Self::parse(&raw)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.name.is_empty()
            || !self
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ManifestError::InvalidName(self.name.clone()));
        }
        if RESERVED_PLUGIN_NAMES.contains(&self.name.as_str()) {
            return Err(ManifestError::ReservedName(self.name.clone()));
        }
        if self.capabilities.routes.is_empty() {
            return Err(ManifestError::NoRoutes);
        }
        for route in &self.capabilities.routes {
            let trimmed = route.trim_matches('/');
            if trimmed.is_empty()
                || !trimmed
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '/')
            {
                return Err(ManifestError::InvalidRoute(route.clone()));
            }
        }
        Ok(())
    }

    /// A short, human-readable summary of what installing this plugin
    /// grants — what `cratebase plugin install`/`list` print.
    pub fn describe_capabilities(&self) -> String {
        let mut lines = Vec::new();
        if !self.capabilities.routes.is_empty() {
            lines.push(format!(
                "HTTP routes: {}",
                self.capabilities
                    .routes
                    .iter()
                    .map(|r| format!("/api/plugins/{}/{}", self.name, r.trim_matches('/')))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if self.capabilities.records_read.is_empty() {
            lines.push("Record access: none".to_string());
        } else {
            lines.push(format!(
                "Record read (rule-enforced): {}",
                self.capabilities.records_read.join(", ")
            ));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_minimal_manifest() {
        let m = Manifest::parse(
            r#"
            name = "hello"
            version = "0.1.0"
            entry = "plugin.wasm"
            [capabilities]
            routes = ["ping"]
            "#,
        )
        .unwrap();
        assert_eq!(m.name, "hello");
        assert_eq!(m.capabilities.routes, vec!["ping"]);
    }

    #[test]
    fn rejects_reserved_name() {
        let err = Manifest::parse(
            r#"
            name = "records"
            version = "0.1.0"
            entry = "plugin.wasm"
            [capabilities]
            routes = ["ping"]
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ManifestError::ReservedName(_)));
    }

    #[test]
    fn rejects_no_routes() {
        let err = Manifest::parse(
            r#"
            name = "hello"
            version = "0.1.0"
            entry = "plugin.wasm"
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ManifestError::NoRoutes));
    }

    #[test]
    fn rejects_bad_name_chars() {
        let err = Manifest::parse(
            r#"
            name = "hello world"
            version = "0.1.0"
            entry = "plugin.wasm"
            [capabilities]
            routes = ["ping"]
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ManifestError::InvalidName(_)));
    }
}
