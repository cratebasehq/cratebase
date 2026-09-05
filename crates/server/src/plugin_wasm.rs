//! Third-party WASM plugins: bridges `cratebase_wasm_plugin`'s sandbox
//! into the server's [`crate::plugin::Plugin`] mechanism, and backs the
//! `cratebase plugin install`/`list` CLI subcommands.
//!
//! # Where a plugin lives
//!
//! `<data_dir>/plugins/<name>/{plugin.toml, <entry>.wasm}`. `plugin
//! install` copies a source directory (or a bare `.wasm` next to its
//! manifest) there; [`discover_and_register`] scans it once at boot,
//! **before** [`crate::app::App::bootstrap`] runs (the same ordering
//! constraint every [`crate::plugin::Plugin`] has). A directory that
//! does not exist — the common case, no plugin ever installed — is a
//! silent no-op, so this costs nothing on a default install.
//!
//! # What actually runs
//!
//! [`WasmPluginAdapter`] is a normal [`crate::plugin::Plugin`]: its
//! `routes()` mounts one axum handler per path the manifest declared
//! under `capabilities.routes`, at the same `/api/plugins/<name>/...`
//! prefix a compile-time Rust plugin gets. The handler is a completely
//! ordinary route — it goes through the same request logging and rate
//! limiting as everything else in `/api` — that hands the request off
//! to [`cratebase_wasm_plugin::PluginProgram::handle_request`] on a
//! dedicated blocking thread (WASM execution is synchronous; running it
//! inline on a tokio worker would starve other requests while a plugin
//! runs).
//!
//! [`PluginHostApiImpl`] is the one piece of "real capability" plumbing:
//! `get_record` reuses `cratebase_db::records::find_by_id`, the exact
//! function `GET /api/collections/{c}/records/{id}` calls, so a
//! plugin's `viewRule` enforcement can never diverge from the HTTP
//! endpoint's. It intentionally does **not** fire the
//! `onRecordViewRequest` hook chain a real HTTP view does — that chain
//! is `pb_hooks`/compile-time-plugin machinery, out of scope for what
//! this prototype needed to prove (a genuinely rule-enforced read
//! reachable only through a declared capability).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use cratebase_db::DbError;
use cratebase_wasm_plugin::{
    AuthIdentity, Manifest, PluginEngine, PluginHostApi, PluginHostError, PluginProgram,
    PluginRequest, PluginRequestContext,
};
use serde_json::Value;

use crate::app::App;
use crate::extract::{Auth, RequestInfo};
use crate::plugin::Plugin;

/// `<data_dir>/plugins`, where every installed plugin's own directory
/// lives.
pub fn plugins_dir(data_dir: &str) -> PathBuf {
    Path::new(data_dir).join("plugins")
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("source '{0}' does not exist")]
    NotFound(PathBuf),
    #[error("no plugin.toml found alongside '{0}'")]
    NoManifest(PathBuf),
    #[error(transparent)]
    Manifest(#[from] cratebase_wasm_plugin::ManifestError),
    #[error("cannot read entry wasm '{0}': {1}")]
    ReadWasm(PathBuf, std::io::Error),
    #[error("'{0}' does not compile as a WASM module: {1}")]
    InvalidWasm(PathBuf, cratebase_wasm_plugin::RuntimeError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Validate and install the plugin at `src` (a directory containing
/// `plugin.toml` + its entry `.wasm`, or a bare `.wasm` file next to a
/// `plugin.toml`) into `<data_dir>/plugins/<name>`. Returns the parsed
/// manifest so the CLI can print exactly what was granted.
///
/// Compiles the module once, up front, purely to reject a corrupt or
/// non-WASM file at install time rather than at first request.
pub fn install(data_dir: &str, src: &Path) -> Result<Manifest, InstallError> {
    if !src.exists() {
        return Err(InstallError::NotFound(src.to_path_buf()));
    }
    let (source_dir, manifest_path) = if src.is_dir() {
        (src.to_path_buf(), src.join("plugin.toml"))
    } else {
        let dir = src.parent().unwrap_or(Path::new(".")).to_path_buf();
        (dir.clone(), dir.join("plugin.toml"))
    };
    if !manifest_path.exists() {
        return Err(InstallError::NoManifest(manifest_path));
    }
    let manifest = Manifest::load(&manifest_path)?;

    let wasm_path = source_dir.join(&manifest.entry);
    let wasm_bytes =
        std::fs::read(&wasm_path).map_err(|e| InstallError::ReadWasm(wasm_path.clone(), e))?;

    // Fail fast on a bad module instead of installing something that
    // will only error at first request.
    let engine =
        PluginEngine::new().map_err(|e| InstallError::InvalidWasm(wasm_path.clone(), e))?;
    engine
        .load(manifest.clone(), &wasm_bytes)
        .map_err(|e| InstallError::InvalidWasm(wasm_path.clone(), e))?;

    let dest_dir = plugins_dir(data_dir).join(&manifest.name);
    std::fs::create_dir_all(&dest_dir)?;
    std::fs::write(
        dest_dir.join("plugin.toml"),
        toml::to_string_pretty(&manifest).unwrap(),
    )?;
    std::fs::write(dest_dir.join(&manifest.entry), &wasm_bytes)?;

    Ok(manifest)
}

/// Every installed plugin's manifest, for `cratebase plugin list`.
pub fn list_installed(data_dir: &str) -> std::io::Result<Vec<Manifest>> {
    let dir = plugins_dir(data_dir);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut manifests = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.path().is_dir() {
            continue;
        }
        let manifest_path = entry.path().join("plugin.toml");
        if !manifest_path.exists() {
            continue;
        }
        match Manifest::load(&manifest_path) {
            Ok(m) => manifests.push(m),
            Err(e) => tracing::warn!(
                path = %manifest_path.display(),
                "skipping installed plugin with an invalid manifest: {e}"
            ),
        }
    }
    manifests.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(manifests)
}

/// Load and register every plugin under `<data_dir>/plugins`. Must run
/// before [`crate::app::App::bootstrap`]; see [`crate::plugin::Plugin`]'s
/// doc for why. A plugin that fails to load (missing wasm, bad
/// manifest, compile error) is logged and skipped rather than aborting
/// boot — one broken plugin must not take the whole server down.
pub fn discover_and_register(app: &App) -> anyhow::Result<()> {
    let data_dir = &app.config().data_dir;
    let manifests = list_installed(data_dir)?;
    if manifests.is_empty() {
        return Ok(());
    }
    let engine = PluginEngine::new()?;
    for manifest in manifests {
        let name = manifest.name.clone();
        match load_program(&engine, data_dir, manifest) {
            Ok(program) => {
                if let Err(e) = app.register_plugin(WasmPluginAdapter {
                    program: Arc::new(program),
                }) {
                    tracing::error!(plugin = %name, "failed to register WASM plugin: {e}");
                }
            }
            Err(e) => tracing::error!(plugin = %name, "failed to load WASM plugin: {e}"),
        }
    }
    Ok(())
}

fn load_program(
    engine: &PluginEngine,
    data_dir: &str,
    manifest: Manifest,
) -> anyhow::Result<PluginProgram> {
    let dir = plugins_dir(data_dir).join(&manifest.name);
    let wasm_bytes = std::fs::read(dir.join(&manifest.entry))?;
    Ok(engine.load(manifest, &wasm_bytes)?)
}

/// Adapts a compiled [`PluginProgram`] into a [`crate::plugin::Plugin`],
/// mounting one route per manifest-declared path.
pub struct WasmPluginAdapter {
    program: Arc<PluginProgram>,
}

impl Plugin for WasmPluginAdapter {
    fn name(&self) -> &str {
        &self.program.manifest().name
    }

    fn routes(&self) -> Option<Router<App>> {
        let mut router = Router::new();
        for route in &self.program.manifest().capabilities.routes {
            let path = format!("/{}", route.trim_matches('/'));
            let program = self.program.clone();
            router = router.route(
                &path,
                any(move |state, info, body| handle(program.clone(), state, info, body)),
            );
        }
        Some(router)
    }
}

async fn handle(
    program: Arc<PluginProgram>,
    State(app): State<App>,
    info: RequestInfo,
    body: axum::body::Bytes,
) -> Response {
    let body_value: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    };
    let identity = info.auth.as_ref().map(|a| AuthIdentity {
        id: a.id.clone(),
        collection_id: a.collection_id.clone(),
        collection_name: a.collection_name.clone(),
        email: None,
    });
    let host: Arc<dyn PluginHostApi> = Arc::new(PluginHostApiImpl {
        app,
        auth: info.auth.clone(),
    });
    let ctx = PluginRequestContext { auth: identity };
    let request = PluginRequest {
        method: info.method.clone(),
        path: info.path.clone(),
        query: Value::Object(info.query.clone()),
        body: body_value,
    };
    let handle = tokio::runtime::Handle::current();

    // Fuel is the real, deterministic defense against a runaway
    // plugin (see `runtime::DEFAULT_FUEL`'s doc) — this timeout is
    // belt-and-suspenders in case fuel accounting is ever wrong for
    // some pathological pattern. It does not (and, short of killing an
    // OS thread, cannot) actually stop the blocking thread the plugin
    // is running on; a plugin that outruns this timeout has its
    // response abandoned but keeps burning that one thread until its
    // own fuel eventually runs out. Documented as a real limitation of
    // this prototype, not silently papered over.
    const WALL_CLOCK_CEILING: std::time::Duration = std::time::Duration::from_secs(10);
    let outcome = tokio::time::timeout(
        WALL_CLOCK_CEILING,
        tokio::task::spawn_blocking(move || program.handle_request(host, ctx, handle, request)),
    )
    .await;

    let outcome = match outcome {
        Ok(join_result) => join_result,
        Err(_) => {
            tracing::error!(
                "plugin execution exceeded the {WALL_CLOCK_CEILING:?} wall-clock ceiling"
            );
            return (
                axum::http::StatusCode::GATEWAY_TIMEOUT,
                Json(serde_json::json!({
                    "status": 504,
                    "message": "plugin did not respond in time",
                    "data": {}
                })),
            )
                .into_response();
        }
    };

    match outcome {
        Ok(Ok(response)) => {
            let status = axum::http::StatusCode::from_u16(response.status)
                .unwrap_or(axum::http::StatusCode::OK);
            (status, Json(response.body)).into_response()
        }
        Ok(Err(e)) => {
            tracing::warn!("plugin execution failed: {e}");
            (
                axum::http::StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "status": 502,
                    "message": e.to_string(),
                    "data": {}
                })),
            )
                .into_response()
        }
        Err(join_err) => {
            tracing::error!("plugin execution task panicked: {join_err}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "status": 500,
                    "message": "plugin execution failed",
                    "data": {}
                })),
            )
                .into_response()
        }
    }
}

/// The server's [`PluginHostApi`] implementation: one instance per
/// request, closing over the caller's real [`Auth`] so `get_record` can
/// evaluate `viewRule` exactly as the HTTP records endpoint would for
/// this same caller.
struct PluginHostApiImpl {
    app: App,
    auth: Option<Auth>,
}

#[async_trait::async_trait]
impl PluginHostApi for PluginHostApiImpl {
    async fn get_record(
        &self,
        _ctx: &PluginRequestContext,
        collection: &str,
        id: &str,
    ) -> Result<Option<Value>, PluginHostError> {
        let Some(collection) = self.app.db().collections.get(collection) else {
            return Ok(None);
        };
        let info = RequestInfo {
            method: "GET".to_string(),
            auth: self.auth.clone(),
            context: crate::extract::CONTEXT_DEFAULT.to_string(),
            ..Default::default()
        };
        let db_ctx = info.to_context();
        let record = match cratebase_db::records::find_by_id(
            self.app.db(),
            &self.app.db().collections,
            &db_ctx,
            &collection,
            id,
            None,
        )
        .await
        {
            Ok(record) => record,
            Err(DbError::NotFound) => return Ok(None),
            Err(e) => return Err(PluginHostError::Internal(e.to_string())),
        };

        let manage = crate::routes::common::has_manage_access(
            self.app.db(),
            &self.app.db().collections,
            &db_ctx,
            &collection,
            record.id(),
        )
        .await
        .map_err(|e| PluginHostError::Internal(e.to_string()))?;
        let show_email = crate::routes::common::show_email_for(self.auth.as_ref(), &record, manage);
        let value = crate::routes::common::enrich_and_serialize(
            &self.app,
            &collection,
            record,
            self.auth.clone(),
            show_email,
        )
        .await
        .map_err(|e| PluginHostError::Internal(e.error.to_string()))?;
        Ok(Some(value))
    }

    fn log(&self, plugin: &str, level: &str, message: &str) {
        match level {
            "warn" => tracing::warn!(plugin, "{message}"),
            "error" => tracing::error!(plugin, "{message}"),
            _ => tracing::info!(plugin, "{message}"),
        }
    }
}

#[cfg(test)]
mod tests {

    /// The two reserved-name lists are duplicated across crate
    /// boundaries (`cratebase_wasm_plugin` cannot depend on `server`);
    /// pin them equal so nobody edits one without the other.
    #[test]
    fn reserved_names_match_the_compile_time_plugin_list() {
        assert_eq!(
            cratebase_wasm_plugin::manifest::RESERVED_PLUGIN_NAMES,
            crate::plugin::RESERVED_PLUGIN_NAMES
        );
    }
}
