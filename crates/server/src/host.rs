//! Multi-project host: one process serving many isolated Cratebase apps,
//! routed by the request's `Host` header.
//!
//! Layout under a root directory:
//!
//! ```text
//! <root>/<name>/project.env   CB_HOSTS=a.example.com, CB_SECRET=…, …
//! <root>/<name>/cb_data/      this project's database and files
//! <root>/<name>/cb_hooks/     this project's JS hooks
//! <root>/<name>/cb_migrations/
//! ```
//!
//! Every project is a complete [`App`] (own database, settings, storage,
//! workers, JS runtime). `project.env` supplies the boot variables that
//! `Config::from_env` would read from the process environment; see
//! [`Config::from_vars`] for which keys never fall back to the host's own
//! environment.
//!
//! A few settings are process-wide by nature and are therefore refused in
//! `project.env` ([`PROCESS_WIDE_KEYS`]); set them in the host process
//! environment instead.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tower::ServiceExt;

use crate::app::App;
use crate::config::Config;

/// Read from the process environment at use time (not through
/// [`Config`]), so a per-project value would be silently ignored or, worse,
/// applied to every project.
const PROCESS_WIDE_KEYS: &[&str] = &[
    "CB_ENCRYPTION",
    "EMBEDDINGS_BASE_URL",
    "EMBEDDINGS_API_KEY",
    "DATABASE_MAX_CONNECTIONS",
];

const ALPHANUMERIC: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// One project found under the root directory.
#[derive(Debug, Clone)]
pub struct ProjectSpec {
    pub name: String,
    pub hosts: Vec<String>,
    pub config: Config,
}

/// Parse a `project.env` file: `KEY=value` lines, `#` comments, an
/// optional `export ` prefix, one pair of matching surrounding quotes
/// stripped, no interpolation.
pub fn parse_env(text: &str) -> anyhow::Result<BTreeMap<String, String>> {
    let mut vars = BTreeMap::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map_or(line, str::trim_start);
        let Some((key, value)) = line.split_once('=') else {
            bail!("line {}: expected KEY=value", i + 1);
        };
        let key = key.trim();
        if key.is_empty() {
            bail!("line {}: empty variable name", i + 1);
        }
        vars.insert(key.to_string(), unquote(value.trim()).to_string());
    }
    Ok(vars)
}

fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if first == last && (first == b'"' || first == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    name.len() <= 32
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Every immediate subdirectory of `root` holding a `project.env`, sorted
/// by name.
pub fn load_specs(root: &Path) -> anyhow::Result<Vec<ProjectSpec>> {
    let mut dirs: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries {
            let entry = entry?;
            let env_file = entry.path().join("project.env");
            if entry.file_type()?.is_dir() && env_file.is_file() {
                dirs.push((entry.file_name().to_string_lossy().into_owned(), env_file));
            }
        }
    }
    if dirs.is_empty() {
        bail!(
            "no projects found under {r} (expected {r}/<name>/project.env)",
            r = root.display()
        );
    }
    dirs.sort();

    let mut specs: Vec<ProjectSpec> = Vec::new();
    let mut owners: HashMap<String, String> = HashMap::new();
    for (name, env_file) in dirs {
        if !valid_name(&name) {
            bail!("project directory name {name:?} must match ^[a-z0-9][a-z0-9_-]{{0,31}}$");
        }
        let text = std::fs::read_to_string(&env_file)
            .with_context(|| format!("reading {}", env_file.display()))?;
        let vars = parse_env(&text).with_context(|| format!("{}", env_file.display()))?;
        for key in PROCESS_WIDE_KEYS {
            if vars.contains_key(*key) {
                bail!(
                    "{key} is process-wide in host mode; set it in the host process environment, not in project.env"
                );
            }
        }
        let hosts: Vec<String> = vars
            .get("CB_HOSTS")
            .map(|raw| {
                raw.split(',')
                    .map(|h| h.trim().to_ascii_lowercase())
                    .filter(|h| !h.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if hosts.is_empty() {
            bail!(
                "{}: CB_HOSTS is required (comma-separated host names this project answers to)",
                env_file.display()
            );
        }
        for host in &hosts {
            if let Some(other) = owners.insert(host.clone(), name.clone()) {
                bail!("host {host} is claimed by both project {other} and project {name}");
            }
        }
        let data_dir = root.join(&name).join("cb_data");
        let config = Config::from_vars(Some(&data_dir.to_string_lossy()), vars);
        specs.push(ProjectSpec {
            name,
            hosts,
            config,
        });
    }
    Ok(specs)
}

/// Boot every project and return the dispatching router together with the
/// apps (so the caller can terminate them on shutdown).
pub async fn build(root: &Path) -> anyhow::Result<(Router, Vec<App>)> {
    let specs = load_specs(root)?;
    let mut routes: HashMap<String, Router> = HashMap::new();
    let mut apps = Vec::with_capacity(specs.len());
    for spec in specs {
        let app = App::new(spec.config);
        crate::plugin_wasm::discover_and_register(&app)?;
        app.bootstrap()
            .await
            .with_context(|| format!("booting project {}", spec.name))?;
        let (router, _) = app.prepare_router("").await?;
        tracing::info!(project = %spec.name, hosts = ?spec.hosts, "project loaded");
        for host in spec.hosts {
            routes.insert(host, router.clone());
        }
        apps.push(app);
    }
    Ok((dispatcher(routes), apps))
}

fn dispatcher(routes: HashMap<String, Router>) -> Router {
    Router::new()
        .fallback(route_by_host)
        .with_state(Arc::new(routes))
}

async fn route_by_host(
    State(routes): State<Arc<HashMap<String, Router>>>,
    req: Request,
) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let target = routes
        .get(&host)
        .or_else(|| host.rsplit_once(':').and_then(|(h, _)| routes.get(h)));
    match target {
        // The inner router's error type is `Infallible`.
        Some(router) => match router.clone().oneshot(req).await {
            Ok(resp) => resp,
            Err(never) => match never {},
        },
        None => unknown_host(),
    }
}

fn unknown_host() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        Body::from(r#"{"code":404,"message":"Unknown project host.","data":{}}"#),
    )
        .into_response()
}

/// `cratebase host serve`: boot every project and listen until shutdown.
pub async fn serve(root: PathBuf, address: String) -> anyhow::Result<()> {
    let (router, apps) = build(&root).await?;
    let listener = tokio::net::TcpListener::bind(&address).await?;
    tracing::info!(address = %address, projects = apps.len(), "cratebase host listening");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(crate::app::shutdown_signal())
    .await?;
    for app in &apps {
        app.terminate(false).await;
    }
    Ok(())
}

/// Create `<root>/<name>/project.env` for a new project and return its
/// first-run setup token.
pub fn new_project(root: &Path, name: &str, host: &str) -> anyhow::Result<String> {
    if !valid_name(name) {
        bail!("project name must match ^[a-z0-9][a-z0-9_-]{{0,31}}$");
    }
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() || host.contains(|c: char| c.is_whitespace() || c == ',') {
        bail!("--host must be a single host name");
    }
    let dir = root.join(name);
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    std::fs::create_dir_all(&dir)?;
    let secret = cratebase_core::ids::random_string(48, ALPHANUMERIC);
    let token = cratebase_core::ids::random_string(24, ALPHANUMERIC);
    let body = format!(
        "CB_HOSTS={host}\nCB_APP_URL=https://{host}\nCB_SECRET={secret}\nCB_SETUP_TOKEN={token}\n"
    );
    std::fs::write(dir.join("project.env"), body)?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_handles_quotes_export_and_comments() {
        let vars = parse_env(
            "# comment\n\nA=1\nexport B = \"two words\"\nC='x=y'\nD=\nE=\"unbalanced\n  F = spaced  \n",
        )
        .unwrap();
        assert_eq!(vars["A"], "1");
        assert_eq!(vars["B"], "two words");
        assert_eq!(vars["C"], "x=y");
        assert_eq!(vars["D"], "");
        assert_eq!(vars["E"], "\"unbalanced");
        assert_eq!(vars["F"], "spaced");
    }

    #[test]
    fn parse_env_names_the_bad_line() {
        let err = parse_env("A=1\n\nnot a pair\n").unwrap_err().to_string();
        assert!(err.contains("line 3"), "{err}");
    }

    #[test]
    fn project_names_are_validated() {
        assert!(valid_name("nyari"));
        assert!(valid_name("a_b-9"));
        assert!(!valid_name(""));
        assert!(!valid_name("-a"));
        assert!(!valid_name("Upper"));
        assert!(!valid_name(&"a".repeat(33)));
    }
}
