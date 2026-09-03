use clap::{Parser, Subcommand};
use cratebase_auth::hash_password;
use cratebase_db::{admins, system, Db};
use cratebase_server::config::Config;
use cratebase_server::{build_app, build_state};

#[derive(Parser)]
#[command(
    name = "cratebase",
    version,
    about = "A fast, self-hostable backend: dynamic collections, auth, files, and realtime over SQLite or Postgres."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the HTTP API (and, once built, the embedded admin dashboard).
    Serve,
    /// Manage superuser (admin panel) accounts.
    Superuser {
        #[command(subcommand)]
        action: SuperuserAction,
    },
}

#[derive(Subcommand)]
enum SuperuserAction {
    /// Create a new superuser. Fails if the email is already taken.
    Create { email: String, password: String },
    /// Create the superuser if the email doesn't exist yet, otherwise reset
    /// its password — handy for non-interactive provisioning
    /// (`docker compose exec cratebase cratebase superuser upsert ...`).
    Upsert { email: String, password: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("cratebase_server=info".parse()?),
        )
        .init();

    let cli = Cli::parse();
    let config = Config::from_env();

    match cli.command {
        Commands::Serve => serve(config).await,
        Commands::Superuser { action } => superuser(config, action).await,
    }
}

async fn serve(config: Config) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.host, config.port);
    let state = build_state(config).await?;
    cratebase_server::plugins::registry().spawn_tasks(&state);
    let app = build_app(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(address = %addr, "cratebase listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn superuser(config: Config, action: SuperuserAction) -> anyhow::Result<()> {
    let db = Db::connect(&config.database_url).await?;
    system::ensure_system_tables(&db).await?;

    match action {
        SuperuserAction::Create { email, password } => {
            let hash = hash_password(&password)?;
            let admin = admins::create_admin(&db, &email, &hash).await?;
            println!("created superuser {} ({})", admin.email, admin.id);
        }
        SuperuserAction::Upsert { email, password } => {
            let hash = hash_password(&password)?;
            match admins::get_admin_by_email(&db, &email).await {
                Ok(existing) => {
                    admins::update_admin_password(&db, &existing.id, &hash).await?;
                    println!("updated password for superuser {email}");
                }
                Err(_) => {
                    let admin = admins::create_admin(&db, &email, &hash).await?;
                    println!("created superuser {} ({})", admin.email, admin.id);
                }
            }
        }
    }
    Ok(())
}
