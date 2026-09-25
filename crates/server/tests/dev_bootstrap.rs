//! `crate::dev::bootstrap`: the one-time setup behind `cratebase dev`.

use cratebase_db::Executor;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use cratebase_server::dev::{self, DevOptions};
use serde_json::json;
use std::path::PathBuf;

fn opts(schema: Option<PathBuf>, seed: Option<PathBuf>, types: Option<PathBuf>) -> DevOptions {
    DevOptions {
        schema,
        seed,
        types,
        force_schema: false,
        admin_email: None,
        admin_password: None,
    }
}

/// A file-backed (not `:memory:`) config, so state survives across two
/// separate `App` instances the way two `cratebase dev` invocations
/// against the same `--dir` would.
fn file_backed_config(data_dir: &std::path::Path) -> Config {
    let mut config = Config::for_data_dir(data_dir);
    config.database_url = format!("sqlite:{}", data_dir.join("data.db").display());
    config.secret = "test-secret-0123456789-0123456789".into();
    config.dev = true;
    config
}

#[tokio::test]
async fn first_bootstrap_creates_superuser_applies_schema_seeds_and_writes_types() {
    let dir = tempfile::tempdir().expect("temp dir");
    let data_dir = dir.path().join("pb_data");

    let schema_path = dir.path().join("schema.json");
    std::fs::write(
        &schema_path,
        json!({
            "collections": [{
                "name": "posts",
                "type": "base",
                "fields": [{"name": "title", "type": "text"}],
            }]
        })
        .to_string(),
    )
    .expect("write schema.json");

    let seed_dir = dir.path().join("pb_seed");
    std::fs::create_dir_all(&seed_dir).expect("mkdir pb_seed");
    std::fs::write(
        seed_dir.join("seed.json"),
        json!({
            "posts": [{"id": "seedpost0000001", "title": "Hello"}],
        })
        .to_string(),
    )
    .expect("write seed.json");

    let types_path = dir.path().join("cratebase-types.d.ts");

    let config = file_backed_config(&data_dir);
    let hooks_dir = config.hooks_dir.clone();
    let migrations_dir = config.migrations_dir.clone();
    let app = App::new(config);

    let report = dev::bootstrap(
        &app,
        &opts(
            Some(schema_path.clone()),
            Some(seed_dir.clone()),
            Some(types_path.clone()),
        ),
    )
    .await
    .expect("bootstrap");

    // Directories created.
    assert!(std::path::Path::new(&hooks_dir).is_dir());
    assert!(std::path::Path::new(&migrations_dir).is_dir());

    // Superuser created (none existed) with a generated password.
    let email = report.superuser_email.clone().expect("superuser created");
    assert!(report.generated_password.is_some());
    assert!(app
        .find_superuser_by_email(&email)
        .await
        .expect("query")
        .is_some());

    // Schema applied.
    let diff = report.schema_diff.expect("schema applied");
    assert_eq!(diff.collections.len(), 1);
    assert_eq!(diff.collections[0].name, "posts");
    assert!(app.db().collections.get_by_name("posts").is_some());

    // Seeded once.
    let seed_report = report.seed_report.expect("seeded");
    assert_eq!(seed_report.total(), 1);

    // Types written and contain the new collection.
    let generated = std::fs::read_to_string(&types_path).expect("types file written");
    assert!(generated.contains("PostsRecord"));

    app.terminate(false).await;

    // A second bootstrap against the same data dir must not duplicate the
    // superuser or the seeded record.
    let config2 = file_backed_config(&data_dir);
    let app2 = App::new(config2);
    let report2 = dev::bootstrap(
        &app2,
        &opts(Some(schema_path), Some(seed_dir), Some(types_path)),
    )
    .await
    .expect("second bootstrap");

    assert!(
        report2.superuser_email.is_none(),
        "a superuser already exists; nothing to report"
    );
    assert!(
        report2.seed_report.is_none(),
        "posts already has a record; seeding must be skipped"
    );

    let count = app2
        .db()
        .query_scalar(r#"SELECT COUNT(*) AS "c" FROM "posts""#, &[])
        .await
        .expect("count query")
        .expect("a row");
    assert_eq!(count.as_i64(), Some(1), "seed must not have duplicated");

    app2.terminate(false).await;
}

#[tokio::test]
async fn env_superuser_is_upserted_even_when_one_already_exists() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut config = Config::memory(dir.path());
    config.dev = true;
    let app = App::new(config);

    let env_opts = DevOptions {
        admin_email: Some("owner@example.com".into()),
        admin_password: Some("supersecret123".into()),
        ..opts(None, None, None)
    };
    let report = dev::bootstrap(&app, &env_opts).await.expect("bootstrap");

    assert_eq!(report.superuser_email.as_deref(), Some("owner@example.com"));
    assert!(
        report.generated_password.is_none(),
        "an explicit CB_ADMIN_PASSWORD is already known; nothing to print"
    );
    assert!(app
        .find_superuser_by_email("owner@example.com")
        .await
        .expect("query")
        .is_some());

    app.terminate(false).await;
}

#[tokio::test]
async fn without_schema_or_seed_or_types_bootstrap_only_creates_the_default_superuser() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut config = Config::memory(dir.path());
    config.dev = true;
    let app = App::new(config);

    let report = dev::bootstrap(&app, &opts(None, None, None))
        .await
        .expect("bootstrap");

    assert_eq!(report.superuser_email.as_deref(), Some("admin@localhost"));
    assert!(report.generated_password.is_some());
    assert!(report.schema_diff.is_none());
    assert!(report.seed_report.is_none());
    assert!(report.types_path.is_none());

    app.terminate(false).await;
}
