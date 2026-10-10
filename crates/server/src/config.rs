//! Boot-time configuration.
//!
//! Only the values that must be known *before* the database is open live
//! here (where the data is, how to reach it, which port to bind). Anything
//! an operator can change while the server runs — SMTP, S3, rate limits,
//! logs retention, batch — lives in [`cratebase_core::Settings`], is
//! persisted in `_params` and is hot-swappable through
//! `PATCH /api/settings`.
//!
//! # Environment variables
//!
//! These are read on every boot and **always** win over anything stored
//! in the database, because they decide how the database is reached in
//! the first place:
//!
//! | Variable | Default | Meaning |
//! |---|---|---|
//! | `CRATEBASE_DATA_DIR` | `./cb_data` | data directory (`--dir`) |
//! | `DATABASE_URL` | `sqlite:<data_dir>/data.db` | main database |
//! | `DB_POOL_SIZE` | `10` | Postgres connection pool size (ignored on SQLite) |
//! | `HOST` | `0.0.0.0` | bind address (`--http`) |
//! | `PORT` | `8090` | bind port (`--http`) |
//! | `CB_HOOKS_DIR` | `<data_dir>/../cb_hooks` | JS hooks directory |
//! | `CB_MIGRATIONS_DIR` | `<data_dir>/../cb_migrations` | JS migrations |
//! | `CB_SEED_DIR` | `<data_dir>/../cb_seed` | seed data directory for `cratebase seed`/`cratebase dev` (see `crate::seed`) |
//! | `CB_DEV` | `false` | dev mode (`--dev`) |
//! | `CB_AUTOMIGRATE` | `true` | write migration files on schema change |
//! | `LOG_REQUESTS` | `true` | persist request logs (a deliberate divergence, see spec §15.3) |
//! | `CB_ENCRYPTION` | unset | 32-char key encrypting `_params` values |
//! | `CORS_ALLOW_ORIGINS` | `*` | comma-separated origins (`--origins`) |
//! | `CB_SECRET` / `AUTH_SECRET` | generated once into `<data_dir>/.secret` | app-wide token signing secret; empty values are treated as unset, and a value under 32 bytes refuses to boot |
//! | `SESSION_TRACKING` | `true` | write a `_sessions` row per login for listing/revocation |
//! | `SESSION_COOKIE` | `false` | also accept/set an httpOnly session cookie alongside the bearer token |
//! | `SESSION_COOKIE_NAME` | `cb_session` | the cookie's name |
//! | `SESSION_COOKIE_DOMAIN` | unset (host-only) | the cookie's `Domain` attribute |
//! | `SESSION_COOKIE_SAMESITE` | `Lax` | `Lax`, `Strict`, or `None` (case-insensitive) |
//! | `SESSION_COOKIE_SECURE` | `true` | whether the cookie carries `Secure` |
//! | `CB_SETUP_TOKEN` | generated at boot | first-run install token `POST /api/setup` requires (see `crate::routes::setup`); set this for a scripted deploy that needs to know it in advance |
//! | `CB_ADMIN_EMAIL` / `CB_ADMIN_PASSWORD` | unset | `cratebase dev` (see `crate::dev`): when both are set, upsert this superuser on every boot instead of the one-time generated `admin@localhost` |
//!
//! These seed [`Settings`] on **first boot only** and are ignored once
//! settings exist in `_params` (an operator editing them in the dashboard
//! must not be overridden by a stale `.env`):
//! `CB_APP_NAME`, `CB_APP_URL`, `CB_SENDER_NAME`, `CB_SENDER_ADDRESS`,
//! `SMTP_ENABLED`, `SMTP_HOST`, `SMTP_PORT`, `SMTP_USERNAME`,
//! `SMTP_PASSWORD`, `SMTP_TLS`, `S3_ENABLED`, `S3_BUCKET`, `S3_REGION`,
//! `S3_ENDPOINT`, `S3_ACCESS_KEY`, `S3_SECRET`, `S3_FORCE_PATH_STYLE`,
//! `AUTH_RATE_LIMIT_ENABLED` (default `true`).
//!
//! A few more env vars live outside this file but are just as real:
//! `DATABASE_MAX_CONNECTIONS` (SQLite reader pool size, `crates/db/src/sqlite.rs`),
//! `EMBEDDINGS_BASE_URL` / `EMBEDDINGS_API_KEY` (`crate::embeddings`), and
//! `CB_PG_DUMP_PATH` / `CB_PG_RESTORE_PATH` (override the `PATH` search
//! for `pg_dump`/`pg_restore` when backing up or restoring a Postgres
//! main database, `crates/db/src/pg_tools.rs`).
//! [`KNOWN_ENV_VARS`] below is the canonical, exhaustive list across all of
//! these — it's what keeps `.env.example` from drifting (see
//! `crates/server/tests/env_example.rs`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cratebase_core::Settings;

/// Default data directory name.
pub const DEFAULT_DATA_DIR: &str = "./cb_data";
/// File holding the generated app secret when none is configured.
pub const SECRET_FILE: &str = ".secret";

/// Every environment variable Cratebase's server binary reads or accepts,
/// across every crate — the module doc above has the per-variable
/// defaults and meaning. `.env.example` at the repo root must document
/// exactly this set: `crates/server/tests/env_example.rs` parses it and
/// fails the build if it lists a key that isn't here, so a var can't be
/// added to (or renamed in) `.env.example` without a matching entry (or
/// vice versa) — that's the drift this list exists to catch.
pub const KNOWN_ENV_VARS: &[&str] = &[
    // --- core --------------------------------------------------------
    "CRATEBASE_DATA_DIR",
    "DATABASE_URL",
    "HOST",
    "PORT",
    "CORS_ALLOW_ORIGINS",
    "CB_HOOKS_DIR",
    "CB_MIGRATIONS_DIR",
    "CB_SEED_DIR",
    "LOG_REQUESTS",
    // --- database ------------------------------------------------------
    "DATABASE_MAX_CONNECTIONS", // crates/db/src/sqlite.rs
    "DB_POOL_SIZE",
    "CB_PG_DUMP_PATH",    // crates/db/src/pg_tools.rs
    "CB_PG_RESTORE_PATH", // crates/db/src/pg_tools.rs
    // --- auth / sessions -------------------------------------------
    "CB_ENCRYPTION",
    "CB_SECRET",
    "AUTH_SECRET",
    "AUTH_RATE_LIMIT_ENABLED",
    "CB_SETUP_TOKEN",
    "SESSION_TRACKING",
    "SESSION_COOKIE",
    "SESSION_COOKIE_NAME",
    "SESSION_COOKIE_DOMAIN",
    "SESSION_COOKIE_SAMESITE",
    "SESSION_COOKIE_SECURE",
    // --- first-boot settings seed: mail (Settings::smtp/meta) --------
    "CB_APP_NAME",
    "CB_APP_URL",
    "CB_SENDER_NAME",
    "CB_SENDER_ADDRESS",
    "SMTP_ENABLED",
    "SMTP_HOST",
    "SMTP_PORT",
    "SMTP_USERNAME",
    "SMTP_PASSWORD",
    "SMTP_TLS",
    // --- first-boot settings seed: storage (Settings::s3) -----------
    "S3_ENABLED",
    "S3_BUCKET",
    "S3_REGION",
    "S3_ENDPOINT",
    "S3_ACCESS_KEY",
    "S3_SECRET",
    "S3_FORCE_PATH_STYLE",
    // --- AI / embeddings (crates/server/src/embeddings.rs) ----------
    "EMBEDDINGS_BASE_URL",
    "EMBEDDINGS_API_KEY",
    // --- dev -----------------------------------------------------------
    "CB_DEV",
    "CB_AUTOMIGRATE",
    "CB_TYPEGEN_OUT",
    "CB_ADMIN_EMAIL",
    "CB_ADMIN_PASSWORD",
];

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: String,
    pub database_url: String,
    /// Postgres connection pool size (`DB_POOL_SIZE`). Ignored on SQLite.
    pub db_pool_size: usize,
    pub host: String,
    pub port: u16,
    /// Directory scanned for `*.cb.js` hooks by the JS runtime (W5).
    pub hooks_dir: String,
    /// Directory holding `*.js` migrations written by automigrate.
    pub migrations_dir: String,
    /// Convention directory for `cratebase seed`/the (future) `cratebase
    /// dev` auto-seed: a JSON file or a directory of `*.json`/`*.js` seed
    /// files (`CB_SEED_DIR`, default `<data_dir>/../cb_seed`). Only a
    /// default location — `cratebase seed <path>` always takes its path
    /// as an explicit argument and ignores this field; it exists so a
    /// caller that wants the convention (`cratebase dev`, a test) has a
    /// ready answer for "where is it" without recomputing the sibling
    /// path itself.
    pub seed_dir: String,
    /// Static files served at `/` instead of the embedded dashboard.
    pub public_dir: Option<String>,
    pub dev: bool,
    pub automigrate: bool,
    pub log_requests: bool,
    /// `CB_ENCRYPTION`; when set, `_params` values are AES-256-GCM
    /// encrypted at rest.
    pub encryption_key: Option<String>,
    pub origins: Vec<String>,
    /// App-wide half of every token signing key (the other halves are the
    /// record's `tokenKey` and the collection's per-type secret).
    pub secret: String,
    /// Whether logins write a `_sessions` row (`crate::sessions::record`)
    /// so it can later be listed/revoked. Stateless auth verification
    /// (`crate::extract::resolve`) never depends on this — it only gates
    /// the best-effort bookkeeping insert.
    pub session_tracking: bool,
    /// Whether `crate::extract::resolve` also accepts a bearer token from
    /// an httpOnly cookie (`crate::cookie`), and whether token-minting
    /// routes set that cookie. The CORS layer (`crate::middleware::cors`)
    /// only enables `allow_credentials` when this is set — see its
    /// module doc.
    pub session_cookie: bool,
    pub session_cookie_name: String,
    /// Empty means host-only (no `Domain` attribute at all).
    pub session_cookie_domain: String,
    pub session_cookie_same_site: SameSite,
    pub session_cookie_secure: bool,
    /// `CB_SETUP_TOKEN`: the first-run install token, set explicitly for
    /// a scripted deploy. `None` means `App::bootstrap` generates a
    /// random one instead (see `crate::routes::setup`).
    pub setup_token: Option<String>,
    /// `CB_TYPEGEN_OUT`: when set and `dev` is true, `cratebase
    /// typegen`'s output is rewritten to this path every time a
    /// collection is created, updated or deleted (see
    /// `crate::routes::collections::apply`). `None` disables the watch;
    /// `cratebase typegen` still works as a one-shot regardless.
    pub typegen_out: Option<String>,
    /// Whether `App::bootstrap` starts `crate::webhook_deliveries`'s
    /// always-on ticker. `true` everywhere real (there is no
    /// `settings.webhooks.enabled` to gate this behind — see that
    /// module's doc for why delivery has no toggle); `false` only for a
    /// test that wants to manipulate `_webhookDeliveries`/`_webhooks`
    /// deterministically without racing a live worker claiming the very
    /// rows it just wrote, within the same process, on its own 1-second
    /// timer. Not a real feature flag, not exposed to `settings` or any
    /// env var — a construction-time-only testability seam.
    pub webhook_worker: bool,
    /// A multi-project host's per-project variables (`project.env`);
    /// empty in single-app mode. See [`Config::var`].
    pub vars: Arc<BTreeMap<String, String>>,
}

/// A cookie's `SameSite` attribute. Parsed case-insensitively from
/// `SESSION_COOKIE_SAMESITE`/`--session-cookie-samesite`; an unrecognised
/// env value falls back to `Lax` with a warning (env parsing here is
/// infallible), while the clap flag rejects one outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SameSite {
    #[default]
    Lax,
    Strict,
    None,
}

impl SameSite {
    pub fn as_str(self) -> &'static str {
        match self {
            SameSite::Lax => "Lax",
            SameSite::Strict => "Strict",
            SameSite::None => "None",
        }
    }

    /// `None` (not `Some(SameSite::Lax)`-as-fallback) on an unrecognised
    /// value, so callers can choose how to report it.
    pub fn parse(raw: &str) -> Option<SameSite> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "lax" => Some(SameSite::Lax),
            "strict" => Some(SameSite::Strict),
            "none" => Some(SameSite::None),
            _ => None,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config::for_data_dir(DEFAULT_DATA_DIR)
    }
}

impl Config {
    /// A config rooted at `data_dir` with every default, touching no
    /// environment and no disk.
    pub fn for_data_dir(data_dir: impl AsRef<Path>) -> Self {
        let dir = data_dir.as_ref().to_string_lossy().into_owned();
        Config {
            database_url: format!("sqlite:{dir}/data.db"),
            db_pool_size: cratebase_db::postgres::DEFAULT_POOL_SIZE,
            hooks_dir: sibling(&dir, "cb_hooks"),
            migrations_dir: sibling(&dir, "cb_migrations"),
            seed_dir: sibling(&dir, "cb_seed"),
            data_dir: dir,
            host: "0.0.0.0".into(),
            port: 8090,
            public_dir: None,
            dev: false,
            automigrate: true,
            log_requests: true,
            encryption_key: None,
            origins: vec!["*".into()],
            secret: String::new(),
            session_tracking: true,
            session_cookie: false,
            session_cookie_name: "cb_session".into(),
            session_cookie_domain: String::new(),
            session_cookie_same_site: SameSite::Lax,
            session_cookie_secure: true,
            setup_token: None,
            typegen_out: None,
            webhook_worker: true,
            vars: Arc::default(),
        }
    }

    /// An in-memory config for tests: no files, no listener.
    pub fn memory(data_dir: impl AsRef<Path>) -> Self {
        Config {
            database_url: "sqlite::memory:".into(),
            log_requests: true,
            secret: "test-secret-0123456789".into(),
            ..Config::for_data_dir(data_dir)
        }
    }

    /// Read every boot variable from the environment (after loading
    /// `.env`, if present). Creates the data directory and, on first run,
    /// the persisted app secret.
    pub fn from_env() -> Self {
        Config::from_env_with_dir(None)
    }

    /// As [`Config::from_env`], with `--dir` overriding
    /// `CRATEBASE_DATA_DIR`.
    ///
    /// The override has to be applied *before* anything derived from the
    /// data directory is computed — the database path, the hooks and
    /// migrations directories and, crucially, the generated `.secret`
    /// file, which would otherwise be written into (and read from) the
    /// default `./cb_data` while the rest of the app used `--dir`.
    pub fn from_env_with_dir(dir: Option<&str>) -> Self {
        let _ = dotenvy::dotenv();
        let get = |k: &str| std::env::var(k).ok();
        let data_dir = match dir {
            Some(dir) if !dir.trim().is_empty() => dir.to_string(),
            _ => env_or(&get, "CRATEBASE_DATA_DIR", DEFAULT_DATA_DIR),
        };
        if let Some(message) = legacy_layout_error(Path::new(&data_dir)) {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Config::build(Some(&data_dir), &get)
    }

    /// A config for one project of a multi-project host: every boot
    /// variable comes from `vars` (the project's `project.env`), falling
    /// back to the process environment — except [`PROJECT_ONLY_KEYS`],
    /// which are read from `vars` alone. A shared `DATABASE_URL` or
    /// `CB_SECRET` in the host's environment must never leak into a
    /// project, or two projects would share one database.
    pub fn from_vars(dir: Option<&str>, vars: BTreeMap<String, String>) -> Self {
        let mut config = Config::build(dir, &|k| project_var(&vars, k));
        config.vars = Arc::new(vars);
        config
    }

    /// A variable as this app sees it: the project's own value first,
    /// then the process environment (never for [`PROJECT_ONLY_KEYS`]).
    /// In single-app mode `vars` is empty and this is plain env access.
    pub fn var(&self, key: &str) -> Option<String> {
        project_var(&self.vars, key)
    }

    fn build(dir: Option<&str>, get: &dyn Fn(&str) -> Option<String>) -> Self {
        let data_dir = match dir {
            Some(dir) if !dir.trim().is_empty() => dir.to_string(),
            _ => env_or(get, "CRATEBASE_DATA_DIR", DEFAULT_DATA_DIR),
        };
        std::fs::create_dir_all(&data_dir).ok();

        let mut config = Config::for_data_dir(&data_dir);
        config.database_url = env_or(get, "DATABASE_URL", &config.database_url);
        config.db_pool_size = env_or(get, "DB_POOL_SIZE", &config.db_pool_size.to_string())
            .parse()
            .unwrap_or(config.db_pool_size);
        config.host = env_or(get, "HOST", &config.host);
        config.port = env_or(get, "PORT", "8090").parse().unwrap_or(8090);
        config.hooks_dir = env_or(get, "CB_HOOKS_DIR", &config.hooks_dir);
        config.migrations_dir = env_or(get, "CB_MIGRATIONS_DIR", &config.migrations_dir);
        config.seed_dir = env_or(get, "CB_SEED_DIR", &config.seed_dir);
        config.dev = env_bool(get, "CB_DEV", false);
        config.automigrate = env_bool(get, "CB_AUTOMIGRATE", true);
        config.log_requests = env_bool(get, "LOG_REQUESTS", true);
        config.encryption_key = get(cratebase_db::params::ENCRYPTION_ENV).filter(|v| !v.is_empty());
        config.origins = split_csv(&env_or(get, "CORS_ALLOW_ORIGINS", "*"));
        config.secret = resolve_secret(&data_dir, get("CB_SECRET"), get("AUTH_SECRET"))
            .unwrap_or_else(|msg| {
                eprintln!("cratebase: refusing to start: {msg}");
                std::process::exit(1);
            });
        config.session_tracking = env_bool(get, "SESSION_TRACKING", true);
        config.session_cookie = env_bool(get, "SESSION_COOKIE", false);
        config.session_cookie_name =
            env_or(get, "SESSION_COOKIE_NAME", &config.session_cookie_name);
        config.session_cookie_domain = get("SESSION_COOKIE_DOMAIN").unwrap_or_default();
        config.session_cookie_same_site = match get("SESSION_COOKIE_SAMESITE") {
            Some(raw) => SameSite::parse(&raw).unwrap_or_else(|| {
                tracing::warn!(
                    value = %raw,
                    "SESSION_COOKIE_SAMESITE must be Lax, Strict, or None; falling back to Lax"
                );
                SameSite::Lax
            }),
            None => SameSite::Lax,
        };
        config.session_cookie_secure = env_bool(get, "SESSION_COOKIE_SECURE", true);
        config.setup_token = get("CB_SETUP_TOKEN").filter(|v| !v.is_empty());
        config.typegen_out = get("CB_TYPEGEN_OUT").filter(|v| !v.is_empty());
        config
    }

    /// The main database file when the backend is SQLite, used by backup
    /// and restore. `None` for Postgres and in-memory databases.
    pub fn sqlite_main_path(&self) -> Option<PathBuf> {
        if !self.database_url.starts_with("sqlite") {
            return None;
        }
        let path = cratebase_db::db::sqlite_path(&self.database_url);
        (path != ":memory:").then(|| PathBuf::from(path))
    }

    pub fn data_path(&self) -> &Path {
        Path::new(&self.data_dir)
    }

    /// The settings a first boot should start from: default settings
    /// with the `CB_*`/`SMTP_*`/`S3_*` env vars folded in. Applied only
    /// when `_params` holds no settings row yet.
    pub fn seed_settings(&self) -> Settings {
        let mut s = Settings::default();
        // Documented as on-by-default (.env.example, docs/deploy); the
        // env var only lets an operator opt out (e.g. already
        // rate-limiting at a reverse proxy in front of Cratebase).
        let get = |k: &str| self.var(k);
        s.rate_limits.enabled = env_bool(&get, "AUTH_RATE_LIMIT_ENABLED", s.rate_limits.enabled);
        if let Some(v) = get("CB_APP_NAME") {
            s.meta.app_name = v;
        }
        if let Some(v) = get("CB_APP_URL") {
            s.meta.app_url = v;
        }
        if let Some(v) = get("CB_SENDER_NAME") {
            s.meta.sender_name = v;
        }
        if let Some(v) = get("CB_SENDER_ADDRESS") {
            s.meta.sender_address = v;
        }
        if let Some(v) = get("SMTP_HOST") {
            s.smtp.host = v;
            s.smtp.enabled = env_bool(&get, "SMTP_ENABLED", true);
        }
        if let Some(v) = get("SMTP_PORT") {
            s.smtp.port = v.parse().unwrap_or(587);
        }
        if let Some(v) = get("SMTP_USERNAME") {
            s.smtp.username = v;
        }
        if let Some(v) = get("SMTP_PASSWORD") {
            s.smtp.password = v;
        }
        s.smtp.tls = env_bool(&get, "SMTP_TLS", s.smtp.tls);
        if let Some(v) = get("S3_BUCKET") {
            s.s3.bucket = v;
            s.s3.enabled = env_bool(&get, "S3_ENABLED", true);
        }
        if let Some(v) = get("S3_REGION") {
            s.s3.region = v;
        }
        if let Some(v) = get("S3_ENDPOINT") {
            s.s3.endpoint = v;
        }
        if let Some(v) = get("S3_ACCESS_KEY") {
            s.s3.access_key = v;
        }
        if let Some(v) = get("S3_SECRET") {
            s.s3.secret = v;
        }
        s.s3.force_path_style = env_bool(&get, "S3_FORCE_PATH_STYLE", s.s3.force_path_style);
        s
    }
}

/// `<data_dir>/../<name>`: `cb_hooks` and `cb_migrations` are siblings of
/// `cb_data`.
fn sibling(data_dir: &str, name: &str) -> String {
    Path::new(data_dir)
        .parent()
        .unwrap_or(Path::new("."))
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// Fail-fast guard for the pre-rename directory layout. Returns a boot
/// error when `data_dir` does not exist but a `pb_data` directory sits
/// next to it (the old default layout): booting would silently create an
/// empty database beside the real one. This is the only place the old
/// names appear.
fn legacy_layout_error(data_dir: &Path) -> Option<String> {
    if data_dir.exists() {
        return None;
    }
    let parent = match data_dir.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let legacy = parent.join("pb_data");
    if !legacy.is_dir() {
        return None;
    }
    Some(format!(
        "found a legacy data directory at {} but no {}. Rename it and its siblings: \
         `pb_data` -> `cb_data`, `pb_hooks` -> `cb_hooks`, `pb_migrations` -> `cb_migrations`, \
         and rename hook/migration files from `*.pb.js` to `*.cb.js`.",
        legacy.display(),
        data_dir.display()
    ))
}

fn env_or(get: &dyn Fn(&str) -> Option<String>, key: &str, default: &str) -> String {
    get(key)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn env_bool(get: &dyn Fn(&str) -> Option<String>, key: &str, default: bool) -> bool {
    match get(key) {
        Some(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"),
        None => default,
    }
}

/// Keys a multi-project host reads from a project's own `project.env`
/// only, never from the process environment: inheriting one from the
/// host would point two projects at the same database, secret or port.
const PROJECT_ONLY_KEYS: &[&str] = &[
    "DATABASE_URL",
    "CRATEBASE_DATA_DIR",
    "CB_SECRET",
    "AUTH_SECRET",
    "CB_SETUP_TOKEN",
    "CB_APP_URL",
    "CB_HOOKS_DIR",
    "CB_MIGRATIONS_DIR",
    "CB_SEED_DIR",
    "HOST",
    "PORT",
];

fn project_var(vars: &BTreeMap<String, String>, key: &str) -> Option<String> {
    if let Some(v) = vars.get(key) {
        return Some(v.clone());
    }
    if !vars.is_empty() && PROJECT_ONLY_KEYS.contains(&key) {
        return None;
    }
    std::env::var(key).ok()
}

/// `"a, b ,"` → `["a", "b"]`. Used for `CORS_ALLOW_ORIGINS` and
/// `--origins`.
pub fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Signing secrets shorter than this are brute-forceable over HMAC; refuse
/// to boot rather than run with one.
const MIN_SECRET_LEN: usize = 32;

/// Resolves the app-wide signing secret from the two env vars that can
/// carry it, falling back to the generated/persisted one for `data_dir`
/// when neither is set.
///
/// An empty value (e.g. docker-compose's `AUTH_SECRET: ${AUTH_SECRET:-}`
/// resolving to `""` when the operator never set it) is treated the same
/// as unset, mirroring `encryption_key`'s `.filter(|v| !v.is_empty())`
/// above — an empty string must never become the signing key. A
/// non-empty value shorter than [`MIN_SECRET_LEN`] is rejected outright
/// rather than silently accepted, since it's very likely a placeholder or
/// typo, not an intentional weak secret.
fn resolve_secret(
    data_dir: &str,
    cb_secret: Option<String>,
    auth_secret: Option<String>,
) -> Result<String, String> {
    let explicit = cb_secret
        .filter(|v| !v.is_empty())
        .or_else(|| auth_secret.filter(|v| !v.is_empty()));
    match explicit {
        Some(secret) if secret.len() < MIN_SECRET_LEN => Err(format!(
            "CB_SECRET/AUTH_SECRET is {} bytes, but must be at least {MIN_SECRET_LEN}; \
             generate one with e.g. `openssl rand -base64 48`, or unset it to have \
             cratebase generate and persist one",
            secret.len()
        )),
        Some(secret) => Ok(secret),
        None => Ok(load_or_create_secret(data_dir)),
    }
}

/// Generate a random 64-char app secret on first run and persist it under
/// the data dir, so tokens keep validating across restarts without every
/// self-hoster having to set `CB_SECRET` by hand.
fn load_or_create_secret(data_dir: &str) -> String {
    let path = Path::new(data_dir).join(SECRET_FILE);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }
    let secret = cratebase_core::ids::random_string(
        64,
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
    );
    let _ = std::fs::write(&path, &secret);
    secret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_cratebase_layout() {
        let c = Config::for_data_dir("./cb_data");
        assert_eq!(c.data_dir, "./cb_data");
        assert_eq!(c.database_url, "sqlite:./cb_data/data.db");
        assert!(c.hooks_dir.ends_with("cb_hooks"));
        assert!(c.migrations_dir.ends_with("cb_migrations"));
        assert!(c.seed_dir.ends_with("cb_seed"));
        assert!(c.automigrate);
        assert_eq!(c.port, 8090);
        assert_eq!(
            c.sqlite_main_path(),
            Some(PathBuf::from("./cb_data/data.db"))
        );
        assert_eq!(c.db_pool_size, cratebase_db::postgres::DEFAULT_POOL_SIZE);
    }

    #[test]
    fn memory_config_has_no_sqlite_file() {
        assert_eq!(Config::memory("/tmp/x").sqlite_main_path(), None);
    }

    #[test]
    fn csv_origins_are_trimmed() {
        assert_eq!(
            split_csv("https://a.example , https://b.example,"),
            vec!["https://a.example", "https://b.example"]
        );
    }

    #[test]
    fn empty_secret_env_vars_fall_back_to_generated_secret() {
        let dir = tempfile::tempdir().expect("temp dir");
        let data_dir = dir.path().to_string_lossy().into_owned();
        // docker-compose.yml's `AUTH_SECRET: ${AUTH_SECRET:-}` resolves to
        // `""` when the operator never set it; that must not become the
        // signing secret (an empty HMAC key is worse than none).
        let secret = resolve_secret(&data_dir, Some(String::new()), Some(String::new()))
            .expect("empty env vars are treated as unset");
        assert!(!secret.is_empty());
        assert!(secret.len() >= MIN_SECRET_LEN);
    }

    #[test]
    fn short_explicit_secret_is_rejected() {
        let dir = tempfile::tempdir().expect("temp dir");
        let data_dir = dir.path().to_string_lossy().into_owned();
        let err = resolve_secret(&data_dir, Some("too-short".into()), None)
            .expect_err("a 9-byte secret must be rejected");
        assert!(
            err.contains("32"),
            "error should mention the minimum: {err}"
        );
    }

    #[test]
    fn short_auth_secret_fallback_is_also_rejected() {
        let dir = tempfile::tempdir().expect("temp dir");
        let data_dir = dir.path().to_string_lossy().into_owned();
        let err = resolve_secret(&data_dir, None, Some("also-too-short".into()))
            .expect_err("AUTH_SECRET is checked the same as CB_SECRET");
        assert!(err.contains("32"));
    }

    #[test]
    fn sufficiently_long_explicit_secret_is_used_verbatim() {
        let dir = tempfile::tempdir().expect("temp dir");
        let data_dir = dir.path().to_string_lossy().into_owned();
        let secret = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            resolve_secret(&data_dir, Some(secret.into()), None).unwrap(),
            secret
        );
    }

    #[test]
    fn cb_secret_takes_priority_over_auth_secret() {
        let dir = tempfile::tempdir().expect("temp dir");
        let data_dir = dir.path().to_string_lossy().into_owned();
        let cb = "cb-0123456789abcdef0123456789abcdef";
        let auth = "auth-0123456789abcdef0123456789abcdef";
        assert_eq!(
            resolve_secret(&data_dir, Some(cb.into()), Some(auth.into())).unwrap(),
            cb
        );
    }
}

#[cfg(test)]
mod legacy_layout_tests {
    use super::*;

    #[test]
    fn a_legacy_data_dir_without_the_new_one_is_refused() {
        let root = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir(root.path().join("pb_data")).unwrap();
        let error = legacy_layout_error(&root.path().join("cb_data")).expect("guard fires");
        assert!(error.contains("`pb_data` -> `cb_data`"), "{error}");
        assert!(error.contains("`*.pb.js` to `*.cb.js`"), "{error}");
    }

    #[test]
    fn the_new_layout_and_fresh_directories_pass() {
        let root = tempfile::tempdir().expect("temp dir");
        assert!(legacy_layout_error(&root.path().join("cb_data")).is_none());
        std::fs::create_dir(root.path().join("pb_data")).unwrap();
        std::fs::create_dir(root.path().join("cb_data")).unwrap();
        assert!(legacy_layout_error(&root.path().join("cb_data")).is_none());
    }
}
