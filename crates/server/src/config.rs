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
}

impl Config {
    pub fn from_env() -> Self {
        let _ = dotenvy::dotenv();

        let data_dir = env_or("CRATEBASE_DATA_DIR", "./data");
        std::fs::create_dir_all(&data_dir).ok();

        let database_url = env_or(
            "DATABASE_URL",
            &format!("sqlite://{data_dir}/cratebase.db"),
        );

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

        let auth_secret = std::env::var("AUTH_SECRET").unwrap_or_else(|_| load_or_create_secret(&data_dir));

        Config {
            database_url,
            storage,
            auth_secret,
            host: env_or("HOST", "0.0.0.0"),
            port: env_or("PORT", "8090").parse().unwrap_or(8090),
            admin_token_ttl_seconds: env_or("ADMIN_TOKEN_TTL_SECONDS", "604800").parse().unwrap_or(604_800),
            auth_token_ttl_seconds: env_or("AUTH_TOKEN_TTL_SECONDS", "1209600")
                .parse()
                .unwrap_or(1_209_600),
            cors_allow_origins: env_or("CORS_ALLOW_ORIGINS", "*")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            data_dir,
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
