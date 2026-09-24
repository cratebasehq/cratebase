//! `cratebase dev`: an opinionated, one-command local-dev bootstrap built
//! entirely from existing building blocks — no new persistence, no new
//! diffing or seeding logic.
//!
//! [`bootstrap`] takes a freshly-constructed (not yet [`App::bootstrap`]ed)
//! [`App`] and a resolved [`DevOptions`], and:
//!
//! 1. creates the data directory and the sibling `pb_hooks`/`pb_migrations`
//!    directories if they don't exist yet;
//! 2. calls [`App::bootstrap`] (opens the database, runs the core
//!    migrations, starts the JS runtime — same as `serve`);
//! 3. provisions a superuser: upserts `CB_ADMIN_EMAIL`/`CB_ADMIN_PASSWORD`
//!    when both are set, otherwise creates `admin@localhost` with a random
//!    password the first time (never touches an install that already has
//!    a superuser);
//! 4. applies a schema-as-code file additively through the same
//!    `plan_and_apply` diff `POST /api/schema/apply` and
//!    `cratebase schema push` use;
//! 5. seeds the database through [`crate::seed::run`] — but only when no
//!    non-system collection holds any record yet, so re-running `dev`
//!    against a database that already has data is a no-op;
//! 6. writes TypeScript types through [`crate::typegen::generate`].
//!
//! The caller (`cratebase dev` in `main.rs`) is responsible for resolving
//! CLI flags/env vars into a [`DevOptions`] (including setting
//! `Config::typegen_out` *before* constructing the `App`, so the
//! `--dev` + `CB_TYPEGEN_OUT` watch in
//! `crate::routes::collections::refresh_typegen_watch` keeps rewriting the
//! same file after `bootstrap` returns) and for running the server
//! afterwards (`App::listen`) — `bootstrap` only does the one-time setup.

use std::path::PathBuf;

use cratebase_db::Executor;

use crate::app::App;
use crate::extract::RequestInfo;
use crate::routes::schema::SchemaDiff;
use crate::seed::SeedReport;

/// Resolved inputs for [`bootstrap`]. The `cratebase dev` CLI command
/// builds this from flags, env vars and filesystem conventions
/// (`--schema`/`./schema.json`, `--seed`/`CB_SEED_DIR`/`./pb_seed`,
/// `--types`/`CB_TYPEGEN_OUT`/`./cratebase-types.d.ts`); a test can build
/// one directly.
#[derive(Debug, Default, Clone)]
pub struct DevOptions {
    /// A schema-as-code JSON file to apply additively. `None` skips step 4
    /// entirely (no schema file found and none given).
    pub schema: Option<PathBuf>,
    /// A seed file or directory (see `crate::seed`'s module doc). `None`
    /// skips step 5 (`--no-seed`, or no seed path found).
    pub seed: Option<PathBuf>,
    /// Where to write generated TypeScript types. `None` disables step 6
    /// entirely (`--no-types`).
    pub types: Option<PathBuf>,
    /// Also drop fields the schema file omits (`--force-schema`).
    pub force_schema: bool,
    /// `CB_ADMIN_EMAIL`. Both this and `admin_password` must be set to
    /// upsert an explicit superuser; either alone is ignored.
    pub admin_email: Option<String>,
    /// `CB_ADMIN_PASSWORD`.
    pub admin_password: Option<String>,
}

/// What [`bootstrap`] did, for the CLI's startup banner and for tests.
#[derive(Debug, Default)]
pub struct DevReport {
    /// The superuser email that was created or upserted, or `None` when
    /// a superuser already existed and no `CB_ADMIN_EMAIL` override was
    /// given (nothing to report).
    pub superuser_email: Option<String>,
    /// Set only when a brand-new `admin@localhost` password was
    /// generated (never set for an explicit `CB_ADMIN_EMAIL`/
    /// `CB_ADMIN_PASSWORD`, since that password is already known to
    /// whoever set it).
    pub generated_password: Option<String>,
    /// The schema diff, when a schema file was applied.
    pub schema_diff: Option<SchemaDiff>,
    /// The seed report, when seeding actually ran.
    pub seed_report: Option<SeedReport>,
    /// The types file path, when one was written.
    pub types_path: Option<PathBuf>,
}

/// Run every step described in the module doc against `app`, which must
/// not have been bootstrapped yet (this function calls [`App::bootstrap`]
/// itself). Idempotent in the ways that matter for a repeated `cratebase
/// dev` run against the same data directory: an existing superuser is
/// left alone (unless `CB_ADMIN_EMAIL`/`CB_ADMIN_PASSWORD` say otherwise),
/// and seeding is skipped once any non-system collection holds a record.
pub async fn bootstrap(app: &App, opts: &DevOptions) -> anyhow::Result<DevReport> {
    ensure_dirs(app)?;
    app.bootstrap().await?;

    let (superuser_email, generated_password) = ensure_superuser(app, opts).await?;
    let schema_diff = apply_schema(app, opts).await?;
    let seed_report = maybe_seed(app, opts).await?;
    let types_path = write_types(app, opts)?;

    Ok(DevReport {
        superuser_email,
        generated_password,
        schema_diff,
        seed_report,
        types_path,
    })
}

/// Step 1: the data dir (also created by `App::bootstrap`, but doing it
/// here too means the hooks/migrations dirs below are guaranteed to exist
/// alongside it even if a caller inspects the filesystem before
/// bootstrapping) and its `pb_hooks`/`pb_migrations` siblings.
fn ensure_dirs(app: &App) -> anyhow::Result<()> {
    let config = app.config();
    std::fs::create_dir_all(&config.data_dir)?;
    std::fs::create_dir_all(&config.hooks_dir)?;
    std::fs::create_dir_all(&config.migrations_dir)?;
    Ok(())
}

/// Step 3.
async fn ensure_superuser(
    app: &App,
    opts: &DevOptions,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    if let (Some(email), Some(password)) = (&opts.admin_email, &opts.admin_password) {
        if app
            .find_superuser_by_email(email)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .is_some()
        {
            app.set_superuser_password(email, password)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        } else {
            app.create_superuser(email, password)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        return Ok((Some(email.clone()), None));
    }

    if any_superuser_exists(app).await? {
        return Ok((None, None));
    }

    const DEFAULT_ADMIN_EMAIL: &str = "admin@localhost";
    let password = cratebase_core::ids::random_string(
        20,
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
    );
    app.create_superuser(DEFAULT_ADMIN_EMAIL, &password)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok((Some(DEFAULT_ADMIN_EMAIL.to_string()), Some(password)))
}

async fn any_superuser_exists(app: &App) -> anyhow::Result<bool> {
    Ok(app
        .db()
        .query_one(r#"SELECT 1 AS "x" FROM "_superusers" LIMIT 1"#, &[])
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .is_some())
}

/// Step 4: the exact diff/apply `routes::schema::plan_and_apply` also
/// backs, so `cratebase dev`, `cratebase schema push` and
/// `POST /api/schema/apply` can never disagree about what "additive"
/// means.
async fn apply_schema(app: &App, opts: &DevOptions) -> anyhow::Result<Option<SchemaDiff>> {
    let Some(path) = &opts.schema else {
        return Ok(None);
    };
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("{} is not valid JSON: {e}", path.display()))?;
    let info = RequestInfo::default();
    let diff =
        crate::routes::schema::plan_and_apply(app, &doc, false, opts.force_schema, &info, None)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.error))?;
    Ok(Some(diff))
}

/// Step 5.
async fn maybe_seed(app: &App, opts: &DevOptions) -> anyhow::Result<Option<SeedReport>> {
    let Some(path) = &opts.seed else {
        return Ok(None);
    };
    if any_records_exist(app).await? {
        return Ok(None);
    }
    let report = crate::seed::run(app, path, true)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(Some(report))
}

/// Whether any non-system collection holds at least one record — the
/// gate that keeps a second `cratebase dev` run from re-seeding (and, via
/// `--upsert`, would just duplicate id-less records) a database that
/// already has data in it, developer-added or otherwise.
async fn any_records_exist(app: &App) -> anyhow::Result<bool> {
    let snapshot = app.db().collections.all();
    for collection in snapshot.all.iter().filter(|c| !c.system) {
        let sql = format!(
            r#"SELECT 1 AS "x" FROM {} LIMIT 1"#,
            cratebase_db::quote_ident(collection.table_name())
        );
        let found = app
            .db()
            .query_one(&sql, &[])
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .is_some();
        if found {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Step 6. A plain one-shot write — the ongoing `--dev` +
/// `CB_TYPEGEN_OUT` watch (`routes::collections::refresh_typegen_watch`)
/// takes over from here for every subsequent collection change, as long
/// as the caller also set `Config::typegen_out` to this same path before
/// constructing `app`.
fn write_types(app: &App, opts: &DevOptions) -> anyhow::Result<Option<PathBuf>> {
    let Some(path) = &opts.types else {
        return Ok(None);
    };
    let snapshot = app.db().collections.all();
    let generated = crate::typegen::generate(snapshot.all.iter().map(|c| c.as_ref()));
    std::fs::write(path, generated)?;
    Ok(Some(path.clone()))
}
