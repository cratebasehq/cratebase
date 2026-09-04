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
//! | `HOST` | `0.0.0.0` | bind address (`--http`) |
//! | `PORT` | `8090` | bind port (`--http`) |
//! | `CB_HOOKS_DIR` | `<data_dir>/../pb_hooks` | JS hooks directory |
//! | `CB_MIGRATIONS_DIR` | `<data_dir>/../pb_migrations` | JS migrations |
//! | `CB_DEV` | `false` | dev mode (`--dev`) |
//! | `CB_AUTOMIGRATE` | `true` | write migration files on schema change |
//! | `LOG_REQUESTS` | `true` | persist request logs (a deliberate divergence, see spec §15.3) |
//! | `CB_ENCRYPTION` | unset | 32-char key encrypting `_params` values |
//! | `CORS_ALLOW_ORIGINS` | `*` | comma-separated origins (`--origins`) |
//! | `CB_SECRET` / `AUTH_SECRET` | generated once into `<data_dir>/.secret` | app-wide token signing secret |
//!
//! These seed [`Settings`] on **first boot only** and are ignored once
//! settings exist in `_params` (an operator editing them in the dashboard
//! must not be overridden by a stale `.env`):
//! `CB_APP_NAME`, `CB_APP_URL`, `CB_SENDER_NAME`, `CB_SENDER_ADDRESS`,
//! `SMTP_ENABLED`, `SMTP_HOST`, `SMTP_PORT`, `SMTP_USERNAME`,
//! `SMTP_PASSWORD`, `SMTP_TLS`, `S3_ENABLED`, `S3_BUCKET`, `S3_REGION`,
//! `S3_ENDPOINT`, `S3_ACCESS_KEY`, `S3_SECRET`, `S3_FORCE_PATH_STYLE`.

use std::path::{Path, PathBuf};

use cratebase_core::Settings;

/// PocketBase's data directory name, so an existing `pb_data` layout is
/// recognised without any flags.
pub const DEFAULT_DATA_DIR: &str = "./pb_data";
/// File holding the generated app secret when none is configured.
pub const SECRET_FILE: &str = ".secret";

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: String,
    pub database_url: String,
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
        config.secret = std::env::var("CB_SECRET")
            .or_else(|_| std::env::var("AUTH_SECRET"))
            .unwrap_or_else(|_| load_or_create_secret(&data_dir));
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
}
