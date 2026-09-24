//! Shelling out to `pg_dump`/`pg_restore` for [`crate::postgres::PostgresEngine`]'s
//! `snapshot_to`/`restore_from`.
//!
//! `Engine::snapshot_to` on Postgres can't be `VACUUM INTO` (Postgres has
//! no such thing), so it spawns the real `pg_dump` binary instead,
//! `--format=custom` so the same file can later be handed straight to
//! `pg_restore --single-transaction`. Everything in this module is
//! plain, synchronous, and pure enough to unit test without a socket:
//! [`PgConnParams`] is extracted once from a live `tokio_postgres::Config`
//! (`PostgresEngine::conn_params`), and [`build_pg_dump_invocation`]/
//! [`build_pg_restore_invocation`] turn it into a [`ToolInvocation`] —
//! the argv/envp a real process actually gets — without running
//! anything. [`run_tool`] is the only part that touches a child process.
//!
//! # Why the password never touches argv
//!
//! `ps`/`/proc/<pid>/cmdline` on a shared machine make a process's
//! command line visible to every other user, so a database password
//! passed as `pg_dump -d postgres://user:pass@host/db` would leak to
//! anyone who happened to run `ps` at the right moment. `PGPASSWORD` (an
//! *environment* variable) has no such exposure — `/proc/<pid>/environ`
//! is only readable by the same user (or root). Host, port, user and
//! `sslmode` aren't secret, but they travel the same way here for one
//! reason: `pg_restore --dbname <name>` only works when nothing else
//! contradicts it, and passing the whole connection purely through
//! environment variables plus a bare database name on argv is the
//! simplest shape that is unambiguously correct for both tools.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::error::{DbError, DbResult};
use crate::postgres_tls::SslMode;

/// Overrides [`find_pg_tool`]'s `PATH` search for `pg_dump` with an
/// explicit binary path — for a deployment where the client tools aren't
/// on `PATH` at all, or where more than one Postgres major version is
/// installed and the default `pg_dump` isn't the right one.
pub(crate) const CB_PG_DUMP_PATH: &str = "CB_PG_DUMP_PATH";
/// As [`CB_PG_DUMP_PATH`], for `pg_restore`.
pub(crate) const CB_PG_RESTORE_PATH: &str = "CB_PG_RESTORE_PATH";

/// Enough of a Postgres connection to build a `pg_dump`/`pg_restore`
/// invocation. Extracted once from `tokio_postgres::Config` so the
/// builders below never need a live connection (or even a real `Config`)
/// to unit test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PgConnParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub dbname: String,
    pub sslmode: SslMode,
}

/// The argv/envp a `pg_dump`/`pg_restore` child process actually gets —
/// a plain data structure so [`build_pg_dump_invocation`] and
/// [`build_pg_restore_invocation`] can be unit tested by inspecting it
/// directly, with nothing spawned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolInvocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// `(name, value)` pairs. The password, when there is one, lives
    /// only here — never in `args` — see this module's doc.
    pub envs: Vec<(String, String)>,
}

/// The `PG*` environment variables every invocation sets: host, port,
/// user, `sslmode`, and — only when the connection has one — the
/// password. Shared by both builders below so dump and restore always
/// agree on how a `PgConnParams` becomes an environment.
fn connection_envs(params: &PgConnParams) -> Vec<(String, String)> {
    let mut envs = vec![
        ("PGHOST".to_string(), params.host.clone()),
        ("PGPORT".to_string(), params.port.to_string()),
        ("PGUSER".to_string(), params.user.clone()),
        (
            "PGSSLMODE".to_string(),
            params.sslmode.libpq_value().to_string(),
        ),
    ];
    if let Some(password) = &params.password {
        envs.push(("PGPASSWORD".to_string(), password.clone()));
    }
    envs
}

/// `pg_dump --format=custom` writing straight to `dest_path`. `-d
/// <dbname>` is a bare identifier, not a connection string — everything
/// that could identify *how* to reach the server (host/port/user/
/// password/sslmode) travels only through `envs`.
pub(crate) fn build_pg_dump_invocation(
    pg_dump: PathBuf,
    params: &PgConnParams,
    dest_path: &Path,
) -> ToolInvocation {
    ToolInvocation {
        program: pg_dump,
        args: vec![
            "--format=custom".to_string(),
            // Never block on a TTY password prompt; a missing/wrong
            // `PGPASSWORD` should fail loudly instead of hanging.
            "--no-password".to_string(),
            "--file".to_string(),
            dest_path.to_string_lossy().into_owned(),
            "-d".to_string(),
            params.dbname.clone(),
        ],
        envs: connection_envs(params),
    }
}

/// `pg_restore --clean --if-exists --single-transaction` from
/// `source_path` (a `pg_dump --format=custom` archive) into the *live*
/// database named by `params.dbname` — `--single-transaction` requires
/// `pg_restore` to hold one real connection open for the whole restore,
/// which is why this needs `-d`, not the "just emit SQL to stdout" mode
/// `pg_restore` falls back to without one.
pub(crate) fn build_pg_restore_invocation(
    pg_restore: PathBuf,
    params: &PgConnParams,
    source_path: &Path,
) -> ToolInvocation {
    ToolInvocation {
        program: pg_restore,
        args: vec![
            "--clean".to_string(),
            "--if-exists".to_string(),
            "--single-transaction".to_string(),
            "--no-password".to_string(),
            "-d".to_string(),
            params.dbname.clone(),
            source_path.to_string_lossy().into_owned(),
        ],
        envs: connection_envs(params),
    }
}

/// Locate `bin_name` (`"pg_dump"`/`"pg_restore"`): an explicit
/// `override_env` (`CB_PG_DUMP_PATH`/`CB_PG_RESTORE_PATH`) wins outright,
/// otherwise the first match on `PATH`. Neither Postgres backup nor
/// restore ships with Cratebase itself — both are external binaries the
/// deployment has to provide (the Docker image's runtime stage installs
/// `postgresql-client`; see `Dockerfile`) — so a missing one is reported
/// as a specific, actionable error rather than the generic
/// `DbError::Unsupported` a caller might otherwise reach for.
pub(crate) fn find_pg_tool(bin_name: &str, override_env: &str) -> DbResult<PathBuf> {
    if let Ok(raw) = std::env::var(override_env) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            return if path.is_file() {
                Ok(path)
            } else {
                Err(DbError::Other(format!(
                    "{override_env} is set to \"{trimmed}\", but no file exists there"
                )))
            };
        }
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(bin_name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(DbError::Other(format!(
        "{bin_name} was not found on PATH. Postgres backups and restores need the \
         PostgreSQL client tools installed (e.g. `apt-get install postgresql-client`, \
         already included in the Cratebase Docker image), or set {override_env} to the \
         full path of {bin_name}."
    )))
}

/// Run `invocation` to completion, mapping a non-zero exit or a spawn
/// failure onto a [`DbError`] whose message includes the tool's own
/// stderr — the only way an operator can tell *why* a dump/restore
/// failed (a bad password, a version mismatch, a permissions error on
/// the target file, ...).
pub(crate) async fn run_tool(invocation: ToolInvocation) -> DbResult<()> {
    let mut command = tokio::process::Command::new(&invocation.program);
    command
        .args(&invocation.args)
        .envs(
            invocation
                .envs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        )
        .stdin(Stdio::null());
    let output = command.output().await.map_err(|e| {
        DbError::Other(format!(
            "failed to run {}: {e}",
            invocation.program.display()
        ))
    })?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(DbError::Other(format!(
            "{} exited with {}: {}",
            invocation.program.display(),
            output.status,
            stderr.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> PgConnParams {
        PgConnParams {
            host: "db.internal".to_string(),
            port: 6543,
            user: "cratebase".to_string(),
            password: Some("s3cret!pw".to_string()),
            dbname: "cb_app".to_string(),
            sslmode: SslMode::Require,
        }
    }

    #[test]
    fn pg_dump_invocation_never_puts_the_password_on_argv() {
        let invocation = build_pg_dump_invocation(
            PathBuf::from("/usr/bin/pg_dump"),
            &params(),
            Path::new("/tmp/staging/data.pgdump"),
        );
        for arg in &invocation.args {
            assert!(
                !arg.contains("s3cret!pw"),
                "password leaked into argv: {arg}"
            );
        }
        assert!(invocation
            .envs
            .contains(&("PGPASSWORD".to_string(), "s3cret!pw".to_string())));
    }

    #[test]
    fn pg_restore_invocation_never_puts_the_password_on_argv() {
        let invocation = build_pg_restore_invocation(
            PathBuf::from("/usr/bin/pg_restore"),
            &params(),
            Path::new("/tmp/staging/data.pgdump"),
        );
        for arg in &invocation.args {
            assert!(
                !arg.contains("s3cret!pw"),
                "password leaked into argv: {arg}"
            );
        }
        assert!(invocation
            .envs
            .contains(&("PGPASSWORD".to_string(), "s3cret!pw".to_string())));
    }

    #[test]
    fn no_password_means_no_pgpassword_env_at_all() {
        let mut p = params();
        p.password = None;
        let invocation =
            build_pg_dump_invocation(PathBuf::from("pg_dump"), &p, Path::new("/tmp/data.pgdump"));
        assert!(!invocation.envs.iter().any(|(k, _)| k == "PGPASSWORD"));
    }

    #[test]
    fn pg_dump_invocation_carries_the_rest_of_the_connection_by_env() {
        let invocation = build_pg_dump_invocation(
            PathBuf::from("/usr/bin/pg_dump"),
            &params(),
            Path::new("/tmp/staging/data.pgdump"),
        );
        assert_eq!(invocation.program, PathBuf::from("/usr/bin/pg_dump"));
        assert!(invocation.args.contains(&"--format=custom".to_string()));
        assert!(invocation.args.contains(&"--no-password".to_string()));
        assert!(invocation
            .envs
            .contains(&("PGHOST".to_string(), "db.internal".to_string())));
        assert!(invocation
            .envs
            .contains(&("PGPORT".to_string(), "6543".to_string())));
        assert!(invocation
            .envs
            .contains(&("PGUSER".to_string(), "cratebase".to_string())));
        assert!(invocation
            .envs
            .contains(&("PGSSLMODE".to_string(), "require".to_string())));
        // dbname is not secret, so it's fine on argv (`pg_restore -d`
        // requires it there to run in "restore into a live connection"
        // mode at all).
        assert!(invocation.args.contains(&"cb_app".to_string()));
    }

    #[test]
    fn pg_restore_invocation_uses_the_safe_flags() {
        let invocation = build_pg_restore_invocation(
            PathBuf::from("/usr/bin/pg_restore"),
            &params(),
            Path::new("/tmp/staging/data.pgdump"),
        );
        for flag in [
            "--clean",
            "--if-exists",
            "--single-transaction",
            "--no-password",
        ] {
            assert!(
                invocation.args.contains(&flag.to_string()),
                "missing {flag} in {:?}",
                invocation.args
            );
        }
    }

    /// Serializes every test in this module that touches process-wide
    /// environment variables — `cargo test` runs tests in parallel by
    /// default, and two of these racing on the same `CB_PG_DUMP_PATH`
    /// would be flaky by construction otherwise.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Sets (or unsets) an env var for the guard's lifetime, restoring
    /// whatever was there before on drop (including on an assertion
    /// panic partway through a test).
    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: serialized by `ENV_LOCK`, held by every test that
            // constructs a guard.
            unsafe { std::env::set_var(key, value) };
            EnvVarGuard { key, previous }
        }

        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            unsafe { std::env::remove_var(key) };
            EnvVarGuard { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(v) => unsafe { std::env::set_var(self.key, v) },
                None => unsafe { std::env::remove_var(self.key) },
            }
        }
    }

    #[test]
    fn find_pg_tool_prefers_the_override_env_when_set() {
        let _lock = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("pg_dump");
        std::fs::write(&fake, b"#!/bin/sh\n").unwrap();
        let _guard = EnvVarGuard::set("CB_PG_DUMP_PATH", fake.to_str().unwrap());

        let found = find_pg_tool("pg_dump", "CB_PG_DUMP_PATH").unwrap();
        assert_eq!(found, fake);
    }

    #[test]
    fn find_pg_tool_reports_a_clear_actionable_error_when_missing() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvVarGuard::unset("CB_PG_DUMP_PATH");

        // A binary name essentially guaranteed not to exist anywhere on
        // this machine's real `PATH`, so the test needs no `PATH`
        // mutation of its own (which would risk breaking every other
        // test in the binary that shells out to anything).
        let err =
            find_pg_tool("cratebase_test_pg_dump_missing_9f3a1", "CB_PG_DUMP_PATH").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("cratebase_test_pg_dump_missing_9f3a1"),
            "{msg}"
        );
        assert!(msg.contains("CB_PG_DUMP_PATH"), "{msg}");
        assert!(!msg.to_lowercase().contains("unsupported"), "{msg}");
    }

    #[test]
    fn find_pg_tool_errors_clearly_when_the_override_path_does_not_exist() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvVarGuard::set("CB_PG_DUMP_PATH", "/definitely/not/a/real/path/pg_dump");

        let err = find_pg_tool("pg_dump", "CB_PG_DUMP_PATH").unwrap_err();
        assert!(err.to_string().contains("CB_PG_DUMP_PATH"));
    }
}
