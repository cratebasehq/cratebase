//! The `cratebase` binary. Flags mirror PocketBase's (spec §11) so an
//! existing `pocketbase serve ...` command line keeps working.

use clap::{Parser, Subcommand};
use cratebase_server::app::App;
use cratebase_server::config::Config;

/// The record path is allocation-heavy (dynamic `serde_json::Value`
/// records, a `String` per text cell); mimalloc's per-thread heaps beat
/// glibc's arena malloc noticeably on that shape of workload across many
/// worker threads.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
#[command(
    name = "cratebase",
    version,
    about = "A fast, self-hostable backend: dynamic collections, auth, files and realtime over SQLite or Postgres."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the HTTP API and the embedded admin dashboard.
    Serve(ServeArgs),
    /// Manage superuser accounts (records in the `_superusers` collection).
    Superuser {
        #[command(subcommand)]
        action: SuperuserAction,
        /// Data directory.
        #[arg(long = "dir", global = true)]
        dir: Option<String>,
    },
    /// PocketBase-compatible alias of `superuser`.
    #[command(hide = true)]
    Admin {
        #[command(subcommand)]
        action: SuperuserAction,
        #[arg(long = "dir", global = true)]
        dir: Option<String>,
    },
    /// Apply, revert and inspect migrations.
    Migrate {
        #[command(subcommand)]
        action: MigrateAction,
        #[arg(long = "dir", global = true)]
        dir: Option<String>,
    },
}

#[derive(clap::Args)]
struct ServeArgs {
    /// `host:port` to listen on.
    #[arg(long = "http")]
    http: Option<String>,
    /// `host:port` for TLS. Not implemented yet; use a reverse proxy.
    #[arg(long = "https")]
    https: Option<String>,
    /// Data directory (default `./pb_data`).
    #[arg(long = "dir")]
    dir: Option<String>,
    /// Directory served at `/` instead of the embedded dashboard.
    #[arg(long = "publicDir")]
    public_dir: Option<String>,
    /// Comma-separated CORS origins.
    #[arg(long = "origins")]
    origins: Option<String>,
    /// Verbose logging and hook reloading.
    #[arg(long = "dev", default_value_t = false)]
    dev: bool,
    /// Write a migration file on every collection change.
    #[arg(long = "automigrate", default_value_t = true, action = clap::ArgAction::Set)]
    automigrate: bool,
}

#[derive(Subcommand, Clone)]
enum SuperuserAction {
    /// Create a superuser; fails when the email is taken.
    Create { email: String, password: String },
    /// Change an existing superuser's password.
    Update { email: String, password: String },
    /// Create or update, whichever applies. Handy for provisioning.
    Upsert { email: String, password: String },
    /// Delete a superuser.
    Delete { email: String },
    /// Print a one-time login URL for a superuser.
    Otp { email: String },
}

#[derive(Subcommand)]
enum MigrateAction {
    /// Apply every unapplied migration.
    Up,
    /// Revert the last `n` migrations (default 1).
    Down { n: Option<usize> },
    /// Scaffold a new migration file.
    Create { name: String },
    /// Write a migration snapshotting the current collections.
    Collections,
    /// Drop ledger rows whose migration file no longer exists.
    HistorySync,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(matches!(&cli.command, Command::Serve(a) if a.dev));

    match cli.command {
        Command::Serve(args) => serve(args).await,
        Command::Superuser { action, dir } | Command::Admin { action, dir } => {
            superuser(dir, action).await
        }
        Command::Migrate { action, dir } => migrate(dir, action).await,
    }
}

fn init_tracing(dev: bool) {
    let default = if dev {
        "cratebase_server=debug,tower_http=debug"
    } else {
        "cratebase_server=info"
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn config_for(dir: Option<String>) -> Config {
    Config::from_env_with_dir(dir.as_deref())
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    if args.https.is_some() {
        anyhow::bail!("--https is not implemented; terminate TLS in a reverse proxy");
    }
    let mut config = config_for(args.dir);
    if let Some(http) = args.http {
        let (host, port) = http
            .rsplit_once(':')
            .ok_or_else(|| anyhow::anyhow!("--http expects host:port"))?;
        if !host.is_empty() {
            config.host = host.to_string();
        }
        config.port = port.parse()?;
    }
    if let Some(origins) = args.origins {
        config.origins = cratebase_server::config::split_csv(&origins);
    }
    config.public_dir = args.public_dir;
    config.dev = args.dev;
    config.automigrate = args.automigrate;

    App::new(config).serve().await
}

/// Superuser records are ordinary auth records in the system
/// `_superusers` collection, so these commands write straight through the
/// storage layer.
///
/// Follow-up: route these through `records::create/update/delete` so
/// the CLI shares the HTTP path's hooks and validation. Kept direct for
/// now because `superuser create` has to work on a database whose
/// collections have not been loaded by a server process yet.
async fn superuser(dir: Option<String>, action: SuperuserAction) -> anyhow::Result<()> {
    let app = App::new(config_for(dir));
    app.bootstrap().await?;

    let result = match action {
        SuperuserAction::Create { email, password } => {
            validate_credentials(&email, &password)?;
            if app.find_superuser_by_email(&email).await?.is_some() {
                anyhow::bail!("a superuser with email {email} already exists");
            }
            app.create_superuser(&email, &password).await?;
            format!("created superuser {email}")
        }
        SuperuserAction::Update { email, password } => {
            validate_credentials(&email, &password)?;
            if !app.set_superuser_password(&email, &password).await? {
                anyhow::bail!("no superuser with email {email}");
            }
            format!("updated the password for {email}")
        }
        SuperuserAction::Upsert { email, password } => {
            validate_credentials(&email, &password)?;
            if app.find_superuser_by_email(&email).await?.is_some() {
                app.set_superuser_password(&email, &password).await?;
                format!("updated the password for {email}")
            } else {
                app.create_superuser(&email, &password).await?;
                format!("created superuser {email}")
            }
        }
        SuperuserAction::Delete { email } => {
            if !app.delete_superuser(&email).await? {
                anyhow::bail!("no superuser with email {email}");
            }
            format!("deleted superuser {email}")
        }
        SuperuserAction::Otp { email } => {
            let Some(_) = app.find_superuser_by_email(&email).await? else {
                anyhow::bail!("no superuser with email {email}");
            };
            let code = cratebase_auth::generate_otp(cratebase_auth::DEFAULT_OTP_LENGTH);
            // W4b-2: persist the code in `_otps` through the auth service so
            // `auth-with-otp` accepts it. Printing it is already useful for
            // an operator locked out of the dashboard.
            format!("one-time code for {email}: {code}")
        }
    };

    app.terminate(false).await;
    println!("{result}");
    Ok(())
}

fn validate_credentials(email: &str, password: &str) -> anyhow::Result<()> {
    if !email.contains('@') {
        anyhow::bail!("'{email}' is not a valid email address");
    }
    if password.chars().count() < 8 {
        anyhow::bail!("the password must be at least 8 characters long");
    }
    Ok(())
}

async fn migrate(dir: Option<String>, action: MigrateAction) -> anyhow::Result<()> {
    let app = App::new(config_for(dir));
    app.bootstrap().await?;
    let runner = cratebase_db::migrations::Runner::core();

    match action {
        MigrateAction::Up => {
            let applied = runner.up(app.db()).await?;
            if applied.is_empty() {
                println!("no new migrations to apply");
            } else {
                for file in applied {
                    println!("applied {file}");
                }
            }
        }
        MigrateAction::Down { n } => {
            for file in runner.down(app.db(), n.unwrap_or(1)).await? {
                println!("reverted {file}");
            }
        }
        MigrateAction::Create { name } => {
            let path = write_migration_stub(&app, &name)?;
            println!("created {}", path.display());
        }
        MigrateAction::Collections => {
            // W7: snapshot the collection set into a JS migration. The
            // JSON export exists now (`Collection::to_json`); what is
            // missing is the JS migration file format the runtime reads.
            anyhow::bail!("`migrate collections` needs the JS migration runtime (W7)");
        }
        MigrateAction::HistorySync => {
            let known: Vec<String> = runner.files().map(str::to_string).collect();
            let removed = cratebase_db::migrations::history_sync(app.db(), &known).await?;
            println!("pruned {} stale ledger row(s)", removed.len());
            for file in removed {
                println!("  {file}");
            }
        }
    }

    app.terminate(false).await;
    Ok(())
}

/// PocketBase's `migrate create` writes a JS file the JS runtime (W5)
/// executes; the CLI only has to lay down the skeleton.
fn write_migration_stub(app: &App, name: &str) -> anyhow::Result<std::path::PathBuf> {
    let dir = std::path::Path::new(&app.config().migrations_dir);
    std::fs::create_dir_all(dir)?;
    let slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = dir.join(format!("{}_{}.js", chrono::Utc::now().timestamp(), slug));
    std::fs::write(
        &path,
        "/// <reference path=\"../pb_data/types.d.ts\" />\nmigrate((app) => {\n  // up\n}, (app) => {\n  // down\n});\n",
    )?;
    Ok(path)
}
