use crate::oauth2::ProviderConfig;
use cratebase_mailer::MailerConfig;
use cratebase_storage::StorageConfig;

/// Runtime configuration, loaded from environment variables (optionally via
/// a `.env` file). Every setting has a sane local-dev default so `cratebase
/// serve` works with zero configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub storage: StorageConfig,
    pub auth_secret: String,
    pub host: String,
    pub port: u16,
    pub admin_token_ttl_seconds: i64,
    pub auth_token_ttl_seconds: i64,
    pub cors_allow_origins: Vec<String>,
    pub data_dir: String,
    /// Rate-limits `/admins/auth-with-password` and
    /// `/collections/{c}/auth-with-password` per client IP to blunt
    /// credential-stuffing/brute-force attempts. On by default; an
    /// operator running behind their own rate limiter (or a trusted
    /// internal-only deployment) can turn it off.
    pub auth_rate_limit_enabled: bool,
    /// Persist every `/api/*` request to `_request_logs` for the
    /// dashboard's Logs page. On by default; `LOG_REQUESTS=false` turns
    /// the middleware into a no-op.
    pub log_requests: bool,
    pub mailer: MailerConfig,
    pub mail_from_address: String,
    pub mail_from_name: String,
    /// Base URL the verification/reset/email-change links in emails point
    /// to — your own web page or a mobile deep-link handler that reads
    /// `?token=` and calls the matching `confirm-*` endpoint. Cratebase
    /// has no frontend of its own for this (it doesn't know if you're
    /// building a web app, a Flutter app, or something else).
    pub public_app_url: String,
    pub verification_token_ttl_seconds: i64,
    pub password_reset_token_ttl_seconds: i64,
    pub email_change_token_ttl_seconds: i64,
    /// TTL for the short-lived, single-purpose file token minted by
    /// `POST /api/files/token` (used to authenticate a protected file
    /// download from a context that can't send an `Authorization`
    /// header, e.g. an `<img src>` tag). Deliberately much shorter than
    /// every other token kind — it exists only long enough for the
    /// browser to fetch the image right after minting it.
    pub file_token_ttl_seconds: i64,
    /// TTL for a one-time OTP code (`request-otp`/`auth-with-otp`, and
    /// the MFA second factor on `auth-with-password`) and for the
    /// matching `TokenKind::Mfa` pending marker — both are minted
    /// together and meant to expire together.
    pub otp_token_ttl_seconds: i64,
    pub oauth_providers: Vec<ProviderConfig>,
}

impl Config {
    pub fn from_env() -> Self {
        let _ = dotenvy::dotenv();

        let data_dir = env_or("CRATEBASE_DATA_DIR", "./data");
        std::fs::create_dir_all(&data_dir).ok();

        let database_url = env_or("DATABASE_URL", &format!("sqlite://{data_dir}/cratebase.db"));

        let storage = match std::env::var("STORAGE_DRIVER").as_deref() {
            Ok("s3") => StorageConfig::S3 {
                bucket: must_env("S3_BUCKET"),
                endpoint: std::env::var("S3_ENDPOINT").ok(),
                region: env_or("S3_REGION", "us-east-1"),
                access_key_id: must_env("S3_ACCESS_KEY_ID"),
                secret_access_key: must_env("S3_SECRET_ACCESS_KEY"),
                force_path_style: env_or("S3_FORCE_PATH_STYLE", "true") == "true",
            },
            _ => StorageConfig::Local {
                base_dir: env_or("STORAGE_LOCAL_DIR", &format!("{data_dir}/storage")),
            },
        };

        let mailer = match std::env::var("MAIL_DRIVER").as_deref() {
            Ok("resend") => MailerConfig::Resend {
                api_key: must_env("RESEND_API_KEY"),
            },
            Ok("smtp") => MailerConfig::Smtp {
                host: must_env("SMTP_HOST"),
                port: env_or("SMTP_PORT", "587").parse().unwrap_or(587),
                username: must_env("SMTP_USERNAME"),
                password: must_env("SMTP_PASSWORD"),
                implicit_tls: env_or("SMTP_IMPLICIT_TLS", "false") == "true",
            },
            // Unset (local dev, or an operator who hasn't configured a
            // provider yet) or an unrecognized value both fall back to
            // logging instead of a hard failure at startup.
            _ => MailerConfig::Log,
        };

        let auth_secret =
            std::env::var("AUTH_SECRET").unwrap_or_else(|_| load_or_create_secret(&data_dir));

        Config {
            database_url,
            storage,
            auth_secret,
            host: env_or("HOST", "0.0.0.0"),
            port: env_or("PORT", "8090").parse().unwrap_or(8090),
            admin_token_ttl_seconds: env_or("ADMIN_TOKEN_TTL_SECONDS", "604800")
                .parse()
                .unwrap_or(604_800),
            auth_token_ttl_seconds: env_or("AUTH_TOKEN_TTL_SECONDS", "1209600")
                .parse()
                .unwrap_or(1_209_600),
            cors_allow_origins: env_or("CORS_ALLOW_ORIGINS", "*")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            data_dir,
            auth_rate_limit_enabled: env_or("AUTH_RATE_LIMIT_ENABLED", "true") == "true",
            log_requests: env_or("LOG_REQUESTS", "true") == "true",
            mailer,
            mail_from_address: env_or("MAIL_FROM_ADDRESS", "no-reply@localhost"),
            mail_from_name: env_or("MAIL_FROM_NAME", "Cratebase"),
            public_app_url: env_or("PUBLIC_APP_URL", "http://localhost:8090"),
            verification_token_ttl_seconds: env_or("VERIFICATION_TOKEN_TTL_SECONDS", "86400")
                .parse()
                .unwrap_or(86_400),
            password_reset_token_ttl_seconds: env_or("PASSWORD_RESET_TOKEN_TTL_SECONDS", "3600")
                .parse()
                .unwrap_or(3_600),
            email_change_token_ttl_seconds: env_or("EMAIL_CHANGE_TOKEN_TTL_SECONDS", "3600")
                .parse()
                .unwrap_or(3_600),
            file_token_ttl_seconds: env_or("FILE_TOKEN_TTL_SECONDS", "120")
                .parse()
                .unwrap_or(120),
            otp_token_ttl_seconds: env_or("OTP_TOKEN_TTL_SECONDS", "300")
                .parse()
                .unwrap_or(300),
            oauth_providers: crate::oauth2::providers_from_env(),
        }
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn must_env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("missing required environment variable: {key}"))
}

/// Generate a random 64-char auth secret on first run and persist it under
/// the data dir so tokens keep validating across restarts without forcing
/// every self-hoster to set `AUTH_SECRET` by hand.
fn load_or_create_secret(data_dir: &str) -> String {
    use rand::Rng;

    let path = format!("{data_dir}/.auth_secret");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }
    let secret: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();
    let _ = std::fs::write(&path, &secret);
    secret
}
