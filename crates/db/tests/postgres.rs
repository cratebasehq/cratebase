//! The same lifecycle on Postgres. Skips (rather than fails) when
//! `TEST_POSTGRES_URL` is unset so machines without a server still pass
//! the default `cargo test`. The target database is wiped — every test
//! in this file does `DROP SCHEMA public CASCADE` against one shared
//! Postgres instance, so [`ONE_TEST_AT_A_TIME`] serializes them; `cargo
//! test`'s default multi-threaded runner would otherwise let two of
//! them race to drop/recreate the same schema underneath each other.

use cratebase_core::{Collection, CollectionType, Field, FieldKind, FieldType};
use cratebase_db::{params, Backend, Db, DbError, Executor, Sql};

static ONE_TEST_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn postgres_full_suite() {
    let _guard = ONE_TEST_AT_A_TIME.lock().await;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!("skipping postgres_full_suite: TEST_POSTGRES_URL not set");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let db = Db::connect(&url, &dir.path().to_string_lossy())
        .await
        .unwrap();
    assert_eq!(db.backend, Backend::Postgres);
    db.execute("DROP SCHEMA public CASCADE", &[]).await.unwrap();
    db.execute("CREATE SCHEMA public", &[]).await.unwrap();
    db.bootstrap().await.unwrap();

    assert_eq!(
        db.collections.get("_superusers").unwrap().id,
        "pbc_3142635823"
    );
    assert!(db.engine.table_exists("users").await.unwrap());
    let idx = db.engine.table_indexes("users").await.unwrap();
    assert!(idx.iter().any(|i| i.starts_with("idx_email_")), "{idx:?}");

    let mut posts = Collection::new("posts", CollectionType::Base);
    let pos = posts.fields.len() - 2;
    posts.fields.insert(
        pos,
        Field::new("title", FieldKind::default_for(FieldType::Text)),
    );
    posts.fields.insert(
        pos + 1,
        Field::new("views", FieldKind::default_for(FieldType::Number)),
    );
    posts
        .fields
        .insert(pos + 2, Field::new("published", FieldKind::Bool {}));
    posts.indexes = vec!["CREATE UNIQUE INDEX `idx_posts_title` ON `posts` (`title`)".into()];
    db.collections.insert(&*db.engine, &posts).await.unwrap();
    assert_eq!(
        db.engine.table_columns("posts").await.unwrap(),
        vec!["id", "title", "views", "published", "created", "updated"]
    );

    db.execute(
        "INSERT INTO \"posts\" (\"id\", \"title\", \"views\", \"published\") VALUES ($1, $2, $3, $4)",
        &[
            Sql::from("abcdefghijklmno"),
            Sql::from("Hello"),
            Sql::Int(3),
            Sql::from(true),
        ],
    )
    .await
    .unwrap();
    let row = db
        .query_one("SELECT * FROM \"posts\"", &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_f64("views"), Some(3.0));
    assert_eq!(row.get_i64("published"), Some(1));
    assert_eq!(row.get_str("created"), Some(""));
    let err = db
        .execute(
            "INSERT INTO \"posts\" (\"id\", \"title\") VALUES ($1, $2)",
            &[Sql::from("bcdefghijklmnop"), Sql::from("Hello")],
        )
        .await
        .unwrap_err();
    match err {
        DbError::UniqueViolation(c) => assert_eq!(c, "idx_posts_title"),
        other => panic!("{other:?}"),
    }

    let mut next = (*db.collections.get("posts").unwrap()).clone();
    next.name = "articles".into();
    next.fields
        .iter_mut()
        .find(|f| f.name == "title")
        .unwrap()
        .name = "heading".into();
    next.fields.retain(|f| f.name != "published");
    next.indexes = vec!["CREATE UNIQUE INDEX `idx_posts_heading` ON `posts` (`heading`)".into()];
    db.collections.update(&*db.engine, &next).await.unwrap();
    assert!(db.engine.table_exists("articles").await.unwrap());
    assert_eq!(
        db.engine.table_indexes("articles").await.unwrap(),
        vec!["idx_posts_heading"]
    );
    let row = db
        .query_one("SELECT * FROM \"articles\"", &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_str("heading"), Some("Hello"));

    let mut view = Collection::new("titles", CollectionType::View);
    view.view_query = "SELECT id, heading FROM articles".into();
    db.collections.insert(&*db.engine, &view).await.unwrap();
    assert_eq!(
        db.engine.table_columns("titles").await.unwrap(),
        vec!["id", "heading"]
    );

    params::set(&db, "k", "v").await.unwrap();
    assert_eq!(params::get(&db, "k").await.unwrap().as_deref(), Some("v"));

    db.collections.delete(&*db.engine, "titles").await.unwrap();
    db.collections
        .delete(&*db.engine, "articles")
        .await
        .unwrap();
    assert!(!db.engine.table_exists("articles").await.unwrap());
    db.close().await.unwrap();
}

/// A view query naming a quoted camelCase column unquoted folds to
/// lowercase on Postgres, so the `CREATE VIEW` fails. The failure used
/// to surface as the bare string "db error" — `tokio_postgres::Error`'s
/// Display carries no detail — leaving nothing actionable in the API
/// response or the logs (issue #22). The SQLSTATE and message must
/// survive: `column "storeid" does not exist` names the folded column
/// the user needs to quote.
#[tokio::test]
async fn postgres_view_query_failures_name_the_missing_column() {
    let _guard = ONE_TEST_AT_A_TIME.lock().await;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!("skipping postgres_view_query_failures_name_the_missing_column: TEST_POSTGRES_URL not set");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let db = Db::connect(&url, &dir.path().to_string_lossy())
        .await
        .unwrap();
    assert_eq!(db.backend, Backend::Postgres);
    db.execute("DROP SCHEMA public CASCADE", &[]).await.unwrap();
    db.execute("CREATE SCHEMA public", &[]).await.unwrap();
    db.bootstrap().await.unwrap();

    // The physical column is quoted camelCase: "storeId".
    let mut orders = Collection::new("orders", CollectionType::Base);
    let pos = orders.fields.len() - 2;
    orders.fields.insert(
        pos,
        Field::new("storeId", FieldKind::default_for(FieldType::Text)),
    );
    db.collections.insert(&*db.engine, &orders).await.unwrap();

    let mut view = Collection::new("order_events", CollectionType::View);
    view.view_query = "SELECT id, storeId FROM orders".into();
    let err = db.collections.insert(&*db.engine, &view).await.unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains(r#""storeid""#) && message.contains("does not exist"),
        "the error must name the folded column, got: {message}"
    );
    // And the failed DDL left no half-created row behind.
    assert!(db.collections.get("order_events").is_none());

    db.close().await.unwrap();
}

/// Whether `pg_dump`/`pg_restore` are on `PATH` — neither ships with
/// Cratebase itself (see `crates/db/src/pg_tools.rs`), so a machine
/// without the PostgreSQL client tools installed must still pass `cargo
/// test`, same as one without `TEST_POSTGRES_URL` set at all.
fn pg_client_tools_available() -> bool {
    std::process::Command::new("pg_dump")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
        && std::process::Command::new("pg_restore")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
}

/// `Engine::snapshot_to`/`restore_from` on Postgres (`crates/server/src/
/// routes/backups.rs`'s real usage of both, minus the ZIP/HTTP layer):
/// dump the database with real data in it, mutate it, restore the dump,
/// and confirm the mutation is gone and the original data is back —
/// proof `pg_dump --format=custom` / `pg_restore --clean --if-exists
/// --single-transaction` round-trip real rows, not just that the
/// commands exit `0`.
#[tokio::test]
async fn postgres_snapshot_to_and_restore_from_round_trip_real_rows() {
    let _guard = ONE_TEST_AT_A_TIME.lock().await;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping postgres_snapshot_to_and_restore_from_round_trip_real_rows: \
             TEST_POSTGRES_URL not set"
        );
        return;
    };
    if !pg_client_tools_available() {
        eprintln!(
            "skipping postgres_snapshot_to_and_restore_from_round_trip_real_rows: \
             pg_dump/pg_restore not found on PATH"
        );
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let db = Db::connect(&url, &dir.path().to_string_lossy())
        .await
        .unwrap();
    assert_eq!(db.backend, Backend::Postgres);
    db.execute("DROP SCHEMA public CASCADE", &[]).await.unwrap();
    db.execute("CREATE SCHEMA public", &[]).await.unwrap();
    db.bootstrap().await.unwrap();

    let mut posts = Collection::new("posts", CollectionType::Base);
    let pos = posts.fields.len() - 2;
    posts.fields.insert(
        pos,
        Field::new("title", FieldKind::default_for(FieldType::Text)),
    );
    db.collections.insert(&*db.engine, &posts).await.unwrap();
    db.execute(
        "INSERT INTO \"posts\" (\"id\", \"title\") VALUES ($1, $2)",
        &[Sql::from("rec1"), Sql::from("before-backup")],
    )
    .await
    .unwrap();

    let dest = dir.path().join("data.pgdump");
    db.engine
        .snapshot_to(&dest.to_string_lossy())
        .await
        .unwrap();
    // `pg_dump --format=custom`'s own magic header, so this is really a
    // custom-format archive and not e.g. an empty file `pg_dump` happened
    // to exit 0 without writing.
    let header = std::fs::read(&dest).unwrap();
    assert_eq!(
        &header[..5],
        b"PGDMP",
        "not a pg_dump custom-format archive"
    );

    // Mutate after the snapshot so the restore has something concrete to
    // prove it undid: change one row, add another.
    db.execute(
        "UPDATE \"posts\" SET \"title\" = $1 WHERE \"id\" = $2",
        &[Sql::from("after-backup"), Sql::from("rec1")],
    )
    .await
    .unwrap();
    db.execute(
        "INSERT INTO \"posts\" (\"id\", \"title\") VALUES ($1, $2)",
        &[Sql::from("rec2"), Sql::from("should-not-survive")],
    )
    .await
    .unwrap();

    db.engine
        .restore_from(&dest.to_string_lossy())
        .await
        .unwrap();

    let restored = db
        .query_one(
            "SELECT title FROM \"posts\" WHERE \"id\" = $1",
            &[Sql::from("rec1")],
        )
        .await
        .unwrap()
        .expect("rec1 survives the restore");
    assert_eq!(restored.get_str("title").unwrap(), "before-backup");
    let should_be_gone = db
        .query_one(
            "SELECT title FROM \"posts\" WHERE \"id\" = $1",
            &[Sql::from("rec2")],
        )
        .await
        .unwrap();
    assert!(
        should_be_gone.is_none(),
        "rec2 was inserted after the snapshot and must not survive restoring it"
    );

    db.close().await.unwrap();
}

/// A missing `pg_dump` is reported as a specific, actionable error (see
/// `crate::pg_tools::find_pg_tool`'s doc) — never PostgresEngine's old
/// blanket `DbError::Unsupported("postgres snapshot")`, which gave an
/// operator nothing to act on. Forcing `CB_PG_DUMP_PATH` at a path that
/// doesn't exist exercises exactly the error `find_pg_tool` builds for a
/// deployment that never installed the client tools, without needing the
/// ambient environment to actually lack them.
#[tokio::test]
async fn snapshot_to_reports_a_specific_error_when_pg_dump_is_missing() {
    let _guard = ONE_TEST_AT_A_TIME.lock().await;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping snapshot_to_reports_a_specific_error_when_pg_dump_is_missing: \
             TEST_POSTGRES_URL not set"
        );
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let db = Db::connect(&url, &dir.path().to_string_lossy())
        .await
        .unwrap();

    // SAFETY: no other test in this binary reads/writes CB_PG_DUMP_PATH,
    // and this file's tests are already fully serialized against each
    // other by `ONE_TEST_AT_A_TIME`.
    unsafe { std::env::set_var("CB_PG_DUMP_PATH", "/definitely/not/a/real/pg_dump") };
    let err = db
        .engine
        .snapshot_to(&dir.path().join("data.pgdump").to_string_lossy())
        .await
        .unwrap_err();
    unsafe { std::env::remove_var("CB_PG_DUMP_PATH") };

    let message = err.to_string();
    assert!(message.contains("CB_PG_DUMP_PATH"), "{message}");
    assert!(
        !message.to_lowercase().contains("unsupported"),
        "must not fall back to the old generic message: {message}"
    );

    db.close().await.unwrap();
}

/// `Engine::prepare_check` on Postgres: a real server-side `prepare`
/// validates syntax and, unlike SQLite's, refuses more than one statement
/// outright — see the trait doc and `crates/server/src/rpc.rs`, which
/// relies on exactly this for `_rpc.sql` save-time validation.
#[tokio::test]
async fn postgres_prepare_check_validates_syntax_and_rejects_multiple_statements() {
    let _guard = ONE_TEST_AT_A_TIME.lock().await;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping postgres_prepare_check_validates_syntax_and_rejects_multiple_statements: \
             TEST_POSTGRES_URL not set"
        );
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let db = Db::connect(&url, &dir.path().to_string_lossy())
        .await
        .unwrap();
    db.execute("DROP SCHEMA public CASCADE", &[]).await.unwrap();
    db.execute("CREATE SCHEMA public", &[]).await.unwrap();
    db.execute("CREATE TABLE t (id TEXT)", &[]).await.unwrap();

    db.engine
        .prepare_check("SELECT * FROM t WHERE id = $1")
        .await
        .expect("valid parameterized statement prepares");
    assert!(
        db.engine.prepare_check("SELECT * FROM nope").await.is_err(),
        "a nonexistent table must fail to prepare"
    );
    assert!(
        db.engine.prepare_check("SELECT 1; SELECT 2").await.is_err(),
        "a real server-side prepare must refuse more than one statement"
    );
    // Nothing was executed: no row was inserted.
    let count = db
        .query_scalar("SELECT COUNT(*) FROM t", &[])
        .await
        .unwrap()
        .and_then(|v| v.as_i64())
        .unwrap();
    assert_eq!(count, 0);

    db.close().await.unwrap();
}
