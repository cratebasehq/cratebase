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
//! | `CRATEBASE_DATA_DIR` | `./pb_data` | data directory (`--dir`) |
//! | `DATABASE_URL` | `sqlite:<data_dir>/data.db` | main database |
//! | `DB_POOL_SIZE` | `10` | Postgres connection pool size (ignored on SQLite) |
//! | `HOST` | `0.0.0.0` | bind address (`--http`) |
//! | `PORT` | `8090` | bind port (`--http`) |
//! | `CB_HOOKS_DIR` | `<data_dir>/../pb_hooks` | JS hooks directory |
//! | `CB_MIGRATIONS_DIR` | `<data_dir>/../pb_migrations` | JS migrations |
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
//! `DATABASE_MAX_CONNECTIONS` (SQLite reader pool size, `crates/db/src/sqlite.rs`)
//! and `EMBEDDINGS_BASE_URL` / `EMBEDDINGS_API_KEY` (`crate::embeddings`).
//! [`KNOWN_ENV_VARS`] below is the canonical, exhaustive list across all of
//! these — it's what keeps `.env.example` from drifting (see
//! `crates/server/tests/env_example.rs`).

use std::path::{Path, PathBuf};

use cratebase_core::Settings;

/// PocketBase's data directory name, so an existing `pb_data` layout is
/// recognised without any flags.
pub const DEFAULT_DATA_DIR: &str = "./pb_data";
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
    "LOG_REQUESTS",
    // --- database ------------------------------------------------------
    "DATABASE_MAX_CONNECTIONS", // crates/db/src/sqlite.rs
    "DB_POOL_SIZE",
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
];

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: String,
    pub database_url: String,
    /// Postgres connection pool size (`DB_POOL_SIZE`). Ignored on SQLite.
    pub db_pool_size: usize,
    pub host: String,
    pub port: u16,
    /// Directory scanned for `*.pb.js` hooks by the JS runtime (W5).
    pub hooks_dir: String,
    /// Directory holding `*.js` migrations written by automigrate.
    pub migrations_dir: String,
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
            hooks_dir: sibling(&dir, "pb_hooks"),
            migrations_dir: sibling(&dir, "pb_migrations"),
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
    /// default `./pb_data` while the rest of the app used `--dir`.
    pub fn from_env_with_dir(dir: Option<&str>) -> Self {
        let _ = dotenvy::dotenv();

        let data_dir = match dir {
            Some(dir) if !dir.trim().is_empty() => dir.to_string(),
            _ => env_or("CRATEBASE_DATA_DIR", DEFAULT_DATA_DIR),
        };
        std::fs::create_dir_all(&data_dir).ok();

        let mut config = Config::for_data_dir(&data_dir);
        config.database_url = env_or("DATABASE_URL", &config.database_url);
        config.db_pool_size = env_or("DB_POOL_SIZE", &config.db_pool_size.to_string())
            .parse()
            .unwrap_or(config.db_pool_size);
        config.host = env_or("HOST", &config.host);
        config.port = env_or("PORT", "8090").parse().unwrap_or(8090);
        config.hooks_dir = env_or("CB_HOOKS_DIR", &config.hooks_dir);
        config.migrations_dir = env_or("CB_MIGRATIONS_DIR", &config.migrations_dir);
        config.dev = env_bool("CB_DEV", false);
        config.automigrate = env_bool("CB_AUTOMIGRATE", true);
        config.log_requests = env_bool("LOG_REQUESTS", true);
        config.encryption_key = std::env::var(cratebase_db::params::ENCRYPTION_ENV)
            .ok()
            .filter(|v| !v.is_empty());
        config.origins = split_csv(&env_or("CORS_ALLOW_ORIGINS", "*"));
        config.secret = resolve_secret(
            &data_dir,
            std::env::var("CB_SECRET").ok(),
            std::env::var("AUTH_SECRET").ok(),
        )
        .unwrap_or_else(|msg| {
            eprintln!("cratebase: refusing to start: {msg}");
            std::process::exit(1);
        });
        config.session_tracking = env_bool("SESSION_TRACKING", true);
        config.session_cookie = env_bool("SESSION_COOKIE", false);
        config.session_cookie_name = env_or("SESSION_COOKIE_NAME", &config.session_cookie_name);
        config.session_cookie_domain = std::env::var("SESSION_COOKIE_DOMAIN").unwrap_or_default();
        config.session_cookie_same_site = match std::env::var("SESSION_COOKIE_SAMESITE") {
            Ok(raw) => SameSite::parse(&raw).unwrap_or_else(|| {
                tracing::warn!(
                    value = %raw,
                    "SESSION_COOKIE_SAMESITE must be Lax, Strict, or None; falling back to Lax"
                );
                SameSite::Lax
            }),
            Err(_) => SameSite::Lax,
        };
        config.session_cookie_secure = env_bool("SESSION_COOKIE_SECURE", true);
        config.setup_token = std::env::var("CB_SETUP_TOKEN")
            .ok()
            .filter(|v| !v.is_empty());
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

    /// The settings a first boot should start from: PocketBase defaults
    /// with the `CB_*`/`SMTP_*`/`S3_*` env vars folded in. Applied only
    /// when `_params` holds no settings row yet.
    pub fn seed_settings(&self) -> Settings {
        let mut s = Settings::default();
        // Documented as on-by-default (.env.example, docs/deploy); the
        // env var only lets an operator opt out (e.g. already
        // rate-limiting at a reverse proxy in front of Cratebase).
        s.rate_limits.enabled = env_bool("AUTH_RATE_LIMIT_ENABLED", s.rate_limits.enabled);
        if let Ok(v) = std::env::var("CB_APP_NAME") {
            s.meta.app_name = v;
        }
        if let Ok(v) = std::env::var("CB_APP_URL") {
            s.meta.app_url = v;
        }
        if let Ok(v) = std::env::var("CB_SENDER_NAME") {
            s.meta.sender_name = v;
        }
        if let Ok(v) = std::env::var("CB_SENDER_ADDRESS") {
            s.meta.sender_address = v;
        }
        if let Ok(v) = std::env::var("SMTP_HOST") {
            s.smtp.host = v;
            s.smtp.enabled = env_bool("SMTP_ENABLED", true);
        }
        if let Ok(v) = std::env::var("SMTP_PORT") {
            s.smtp.port = v.parse().unwrap_or(587);
        }
        if let Ok(v) = std::env::var("SMTP_USERNAME") {
            s.smtp.username = v;
        }
        if let Ok(v) = std::env::var("SMTP_PASSWORD") {
            s.smtp.password = v;
        }
        s.smtp.tls = env_bool("SMTP_TLS", s.smtp.tls);
        if let Ok(v) = std::env::var("S3_BUCKET") {
            s.s3.bucket = v;
            s.s3.enabled = env_bool("S3_ENABLED", true);
        }
        if let Ok(v) = std::env::var("S3_REGION") {
            s.s3.region = v;
        }
        if let Ok(v) = std::env::var("S3_ENDPOINT") {
            s.s3.endpoint = v;
        }
        if let Ok(v) = std::env::var("S3_ACCESS_KEY") {
            s.s3.access_key = v;
        }
        if let Ok(v) = std::env::var("S3_SECRET") {
            s.s3.secret = v;
        }
        s.s3.force_path_style = env_bool("S3_FORCE_PATH_STYLE", s.s3.force_path_style);
        s
    }
}

/// `<data_dir>/../<name>`, PocketBase's layout for `pb_hooks` and
/// `pb_migrations` (siblings of `pb_data`).
fn sibling(data_dir: &str, name: &str) -> String {
    Path::new(data_dir)
        .parent()
        .unwrap_or(Path::new("."))
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"),
        Err(_) => default,
    }
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
    fn defaults_follow_pocketbase_layout() {
        let c = Config::for_data_dir("./pb_data");
        assert_eq!(c.data_dir, "./pb_data");
        assert_eq!(c.database_url, "sqlite:./pb_data/data.db");
        assert!(c.hooks_dir.ends_with("pb_hooks"));
        assert!(c.migrations_dir.ends_with("pb_migrations"));
        assert!(c.automigrate);
        assert_eq!(c.port, 8090);
        assert_eq!(
            c.sqlite_main_path(),
            Some(PathBuf::from("./pb_data/data.db"))
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
