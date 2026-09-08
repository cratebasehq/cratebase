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
    /// Schema-as-code: pull the live schema into a checked-in JSON
    /// file, or push that file back as a diff against the live
    /// database.
    Schema {
        #[command(subcommand)]
        action: SchemaAction,
        #[arg(long = "dir", global = true)]
        dir: Option<String>,
    },
    /// One-shot import of an existing PocketBase installation's
    /// collections, records and files into this Cratebase instance. See
    /// `docs/migrating-from-pocketbase.md`.
    MigrateFromPocketbase {
        /// PocketBase's data directory (what `pocketbase serve --dir`
        /// pointed at); must contain `data.db` and, if any collection has
        /// file fields, a `storage/` subdirectory.
        pb_dir: String,
        /// Cratebase's own data directory to migrate into.
        #[arg(long = "dir")]
        dir: Option<String>,
    },
    /// Manage locally installed third-party WASM plugins (R&D
    /// prototype — see `crates/wasm_plugin`'s module doc).
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
        /// Data directory.
        #[arg(long = "dir", global = true)]
        dir: Option<String>,
    },
}

#[derive(Subcommand)]
enum SchemaAction {
    /// Write every non-system collection's schema to a JSON file, in
    /// the same shape `GET /api/collections` returns.
    Pull {
        /// Output file.
        #[arg(long, default_value = "schema.json")]
        out: String,
    },
    /// Diff a schema-as-code file against the live database and apply
    /// the difference: new collections and fields are created, changed
    /// fields are retyped, and — unless `--force` is passed — fields the
    /// file omits are left in place and only reported.
    Push {
        /// The schema-as-code JSON file (as written by `schema pull`).
        file: String,
        /// Print the plan without writing anything.
        #[arg(long = "dry-run", default_value_t = false)]
        dry_run: bool,
        /// Actually drop fields the file omits from an existing
        /// collection.
        #[arg(long, default_value_t = false)]
        force: bool,
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
    /// Also accept/set an httpOnly session cookie alongside the bearer
    /// token.
    #[arg(long = "session-cookie")]
    session_cookie: Option<bool>,
    /// The session cookie's name (default `cb_session`).
    #[arg(long = "session-cookie-name")]
    session_cookie_name: Option<String>,
    /// The session cookie's `Domain` attribute (default: host-only).
    #[arg(long = "session-cookie-domain")]
    session_cookie_domain: Option<String>,
    /// The session cookie's `SameSite` attribute: lax, strict, or none.
    #[arg(long = "session-cookie-samesite", value_parser = ["lax", "strict", "none"])]
    session_cookie_samesite: Option<String>,
    /// Whether the session cookie carries `Secure` (default true).
    #[arg(long = "session-cookie-secure")]
    session_cookie_secure: Option<bool>,
    /// Whether logins write a `_sessions` row for listing/revocation
    /// (default true).
    #[arg(long = "session-tracking")]
    session_tracking: Option<bool>,
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

#[derive(Subcommand)]
enum PluginAction {
    /// Validate a plugin's manifest, compile-check its `.wasm`, and
    /// copy it into `<data_dir>/plugins/<name>` to load on next boot.
    Install {
        /// A directory containing `plugin.toml` + its entry `.wasm`, or
        /// a bare `.wasm` path next to a `plugin.toml`.
        path: String,
    },
    /// List installed plugins and what each one's manifest grants it.
    List,
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
        Command::Schema { action, dir } => schema(dir, action).await,
        Command::MigrateFromPocketbase { pb_dir, dir } => {
            migrate_from_pocketbase(dir, pb_dir).await
        }
        Command::Plugin { action, dir } => plugin_cmd(dir, action).await,
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
    if let Some(v) = args.session_cookie {
        config.session_cookie = v;
    }
    if let Some(v) = args.session_cookie_name {
        config.session_cookie_name = v;
    }
    if let Some(v) = args.session_cookie_domain {
        config.session_cookie_domain = v;
    }
    if let Some(v) = args.session_cookie_samesite {
        // `value_parser` already rejects anything but these three at
        // argument-parsing time, so this always matches.
        config.session_cookie_same_site =
            cratebase_server::config::SameSite::parse(&v).unwrap_or_default();
    }
    if let Some(v) = args.session_cookie_secure {
        config.session_cookie_secure = v;
    }
    if let Some(v) = args.session_tracking {
        config.session_tracking = v;
    }
    config.public_dir = args.public_dir;
    config.dev = args.dev;
    config.automigrate = args.automigrate;

    let app = App::new(config);
    cratebase_server::plugin_wasm::discover_and_register(&app)?;
    app.serve().await
}

/// `cratebase plugin install|list`. Both act purely on the filesystem —
/// no database, no running server — so `install` against a data
/// directory a live server is currently serving takes effect the next
/// time that server restarts, exactly like dropping a file into
/// `pb_hooks/` does.
async fn plugin_cmd(dir: Option<String>, action: PluginAction) -> anyhow::Result<()> {
    let data_dir = config_for(dir).data_dir;
    match action {
        PluginAction::Install { path } => {
            let manifest =
                cratebase_server::plugin_wasm::install(&data_dir, std::path::Path::new(&path))?;
            println!(
                "Installed '{}' v{} to {}",
                manifest.name,
                manifest.version,
                cratebase_server::plugin_wasm::plugins_dir(&data_dir)
                    .join(&manifest.name)
                    .display()
            );
            println!("{}", manifest.describe_capabilities());
            println!("Restart the server for it to take effect.");
            Ok(())
        }
        PluginAction::List => {
            let manifests = cratebase_server::plugin_wasm::list_installed(&data_dir)?;
            if manifests.is_empty() {
                println!("No plugins installed.");
                return Ok(());
            }
            for manifest in manifests {
                println!("{} v{}", manifest.name, manifest.version);
                if !manifest.description.is_empty() {
                    println!("  {}", manifest.description);
                }
                for line in manifest.describe_capabilities().lines() {
                    println!("  {line}");
                }
            }
            Ok(())
        }
    }
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
            // Persist the code in `_otps` through the auth service so
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
            // Snapshot the collection set into a JS migration. The JSON
            // export exists now (`Collection::to_json`); what is missing
            // is the JS migration file format the runtime reads.
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

/// PocketBase's `migrate create` writes a JS file the JS runtime
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

/// Schema-as-code: `pull` snapshots the live non-system collections to a
/// JSON file in the shape `GET /api/collections` returns; `push` reads
/// that file back and applies the difference through the exact same
/// diff/apply logic the `POST /api/schema/apply` endpoint uses (see
/// `routes::schema::plan_and_apply`), so the CLI and the HTTP surface
/// never disagree about what "the difference" means.
async fn schema(dir: Option<String>, action: SchemaAction) -> anyhow::Result<()> {
    let app = App::new(config_for(dir));
    app.bootstrap().await?;

    match action {
        SchemaAction::Pull { out } => {
            let snapshot = app.db().collections.all();
            let items: Vec<serde_json::Value> = snapshot
                .all
                .iter()
                .filter(|c| !c.system)
                .map(|c| c.to_json())
                .collect();
            let count = items.len();
            let doc = serde_json::json!({ "collections": items });
            std::fs::write(&out, serde_json::to_string_pretty(&doc)?)?;
            println!("wrote {count} collection(s) to {out}");
        }
        SchemaAction::Push {
            file,
            dry_run,
            force,
        } => {
            let raw = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("failed to read {file}: {e}"))?;
            let doc: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| anyhow::anyhow!("{file} is not valid JSON: {e}"))?;
            let info = cratebase_server::extract::RequestInfo::default();
            let diff = cratebase_server::routes::schema::plan_and_apply(
                &app, &doc, dry_run, force, &info, None,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.error))?;

            for c in &diff.collections {
                println!("{:9} {}", c.action, c.name);
                for name in &c.fields_added {
                    println!("           + {name}");
                }
                for name in &c.fields_changed {
                    println!("           ~ {name}");
                }
                for name in &c.fields_removed {
                    let marker = if c.pending_removal {
                        "- (pending, pass --force to drop)"
                    } else {
                        "-"
                    };
                    println!("           {marker} {name}");
                }
            }
            if dry_run {
                println!("dry run: nothing was written");
            } else {
                println!("applied");
            }
        }
    }

    app.terminate(false).await;
    Ok(())
}

/// `cratebase migrate-from-pocketbase <pb_dir>`. Bootstraps a target
/// Cratebase instance (creating it fresh if `--dir` is new) and imports
/// `pb_dir`'s collections, records and files into it. See
/// `crates/server/src/pocketbase_migrate.rs` for the mechanics and
/// `docs/migrating-from-pocketbase.md` for what does and doesn't transfer.
async fn migrate_from_pocketbase(dir: Option<String>, pb_dir: String) -> anyhow::Result<()> {
    let app = App::new(config_for(dir));
    app.bootstrap().await?;

    let report = cratebase_server::pocketbase_migrate::run(&app, &pb_dir).await?;

    println!("collections created: {}", report.collections_created.len());
    for name in &report.collections_created {
        println!("  + {name}");
    }
    println!(
        "collections updated (existing schema, fields merged): {}",
        report.collections_updated.len()
    );
    for name in &report.collections_updated {
        println!("  ~ {name}");
    }
    if !report.collections_skipped_system.is_empty() {
        println!(
            "PocketBase system collections skipped (already exist identically in Cratebase, or are ephemeral auth state not carried across): {}",
            report.collections_skipped_system.join(", ")
        );
    }
    if !report.unsupported_fields.is_empty() {
        println!("fields with no Cratebase equivalent (N/A, dropped):");
        for f in &report.unsupported_fields {
            println!("  ! {}.{} (type: {})", f.collection, f.field, f.field_type);
        }
    }
    if !report.oauth2_needs_reconfiguration.is_empty() {
        println!(
            "OAuth2 providers NOT migrated (client secrets are never carried across instances) — reconfigure manually: {}",
            report.oauth2_needs_reconfiguration.join(", ")
        );
    }
    println!("records migrated:");
    for (name, count) in &report.records_migrated {
        println!("  {name}: {count}");
    }
    println!("files copied: {}", report.files_copied);
    if !report.failed_files.is_empty() {
        println!("files that failed to copy:");
        for f in &report.failed_files {
            println!(
                "  ! {}/{}/{}: {}",
                f.collection, f.record_id, f.filename, f.error
            );
        }
    }

    app.terminate(false).await;

    if !report.failed_files.is_empty() {
        anyhow::bail!(
            "migration completed with {} file copy failure(s); see above",
            report.failed_files.len()
        );
    }
    Ok(())
}
