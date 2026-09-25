//! `cratebase reset`: wipe a local SQLite data directory's database files
//! so the next boot starts clean, then re-run migrations. Deliberately
//! SQLite-only — see [`refuse_if_postgres`] for why.

use std::path::{Path, PathBuf};

use cratebase_core::AppError;
use cratebase_db::Backend;

use crate::config::Config;

/// Refuse a Postgres `reset` outright. Automating it would mean dropping
/// every user collection and every system table in what is very likely a
/// shared, already-provisioned server — there's no "delete the file"
/// equivalent, no dry-run-safe undo, and a mistaken `--dir`/`DATABASE_URL`
/// would land on someone else's database. An operator who actually wants
/// this drops and recreates the database by hand (`dropdb`/`createdb`,
/// or their platform's equivalent) and runs `cratebase migrate up`.
pub fn refuse_if_postgres(config: &Config) -> Result<(), AppError> {
    if Backend::from_url(&config.database_url).map_err(AppError::from)? == Backend::Postgres {
        return Err(AppError::bad_request(
            "cratebase reset only supports SQLite: automating it on Postgres would mean \
             dropping every user collection and system table in a database this tool did \
             not create and cannot safely recreate. Drop and recreate the database by hand \
             (dropdb/createdb or your platform's equivalent), then run `cratebase migrate up`.",
        ));
    }
    Ok(())
}

/// Every file a SQLite `reset` deletes: the main database and its
/// WAL/SHM sidecars, plus the auxiliary logs database (`_logs`, a
/// separate SQLite file even when the main database isn't SQLite — see
/// `cratebase_db::db`'s module doc) and its own sidecars. Listed rather
/// than deleted eagerly so a caller can print what will be removed before
/// asking for confirmation.
pub fn sqlite_reset_paths(config: &Config) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(main) = config.sqlite_main_path() {
        push_with_sidecars(&mut paths, &main);
    }
    let aux = Path::new(&config.data_dir).join(cratebase_db::db::AUXILIARY_DB);
    push_with_sidecars(&mut paths, &aux);
    paths
}

fn push_with_sidecars(paths: &mut Vec<PathBuf>, main: &Path) {
    let base = main.display().to_string();
    paths.push(main.to_path_buf());
    paths.push(PathBuf::from(format!("{base}-wal")));
    paths.push(PathBuf::from(format!("{base}-shm")));
}

/// Delete every file [`sqlite_reset_paths`] lists, ignoring ones that are
/// already gone (a fresh data directory, a database never opened with
/// WAL). Returns the paths actually removed.
pub fn wipe_sqlite_files(config: &Config) -> std::io::Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for path in sqlite_reset_paths(config) {
        match std::fs::remove_file(&path) {
            Ok(()) => removed.push(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;

    #[test]
    fn refuses_a_postgres_url() {
        let mut config = Config::for_data_dir("./pb_data");
        config.database_url = "postgres://user:pass@localhost/db".into();
        let err = refuse_if_postgres(&config).expect_err("postgres must be refused");
        assert!(err.to_string().contains("SQLite"));
    }

    #[test]
    fn accepts_a_sqlite_url() {
        let config = Config::for_data_dir("./pb_data");
        refuse_if_postgres(&config).expect("sqlite must be allowed");
    }

    #[test]
    fn lists_main_and_auxiliary_db_with_sidecars() {
        let config = Config::for_data_dir("/tmp/cratebase-reset-test-data");
        let paths = sqlite_reset_paths(&config);
        let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        assert!(names.iter().any(|n| n.ends_with("data.db")));
        assert!(names.iter().any(|n| n.ends_with("data.db-wal")));
        assert!(names.iter().any(|n| n.ends_with("data.db-shm")));
        assert!(names.iter().any(|n| n.ends_with("auxiliary.db")));
        assert!(names.iter().any(|n| n.ends_with("auxiliary.db-wal")));
    }

    #[test]
    fn in_memory_config_has_no_main_db_file_to_list() {
        let config = Config::memory("/tmp/cratebase-reset-test-memory");
        let paths = sqlite_reset_paths(&config);
        // Still lists the auxiliary logs db (real file, real data dir);
        // just no `data.db*` entries since the main database is `:memory:`.
        assert!(paths
            .iter()
            .all(|p| !p.display().to_string().contains("data.db")));
    }

    /// End-to-end: seed a real file-backed SQLite database with a
    /// superuser and a collection, wipe it, re-bootstrap, and confirm the
    /// wipe actually took (no leftover superuser, collections back to the
    /// system defaults).
    #[tokio::test]
    async fn wiping_and_rebootstrapping_yields_a_clean_database() {
        let dir = tempfile::tempdir().expect("temp dir");
        let data_dir = dir.path().join("pb_data");
        let mut config = Config::for_data_dir(data_dir.to_string_lossy().into_owned());
        config.secret = "test-secret-0123456789012345678901".into();

        let app = App::new(config.clone());
        app.bootstrap().await.expect("bootstrap");
        app.create_superuser("reset-test@example.com", "password12345")
            .await
            .expect("seed a superuser");
        assert!(app
            .find_superuser_by_email("reset-test@example.com")
            .await
            .unwrap()
            .is_some());
        app.terminate(false).await;

        refuse_if_postgres(&config).expect("sqlite config");
        let removed = wipe_sqlite_files(&config).expect("wipe");
        assert!(
            !removed.is_empty(),
            "the just-created db files must have been removed"
        );

        let app = App::new(config);
        app.bootstrap().await.expect("re-bootstrap after wipe");
        assert!(
            app.find_superuser_by_email("reset-test@example.com")
                .await
                .unwrap()
                .is_none(),
            "the wiped database must not carry the old superuser forward"
        );
        app.terminate(false).await;
    }
}
