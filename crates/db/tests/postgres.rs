mod common;

use cratebase_db::{system, Db};

/// Proves the same record engine behaves identically on Postgres. Skips
/// (rather than fails) when `TEST_POSTGRES_URL` isn't set, since CI/dev
/// machines without a running Postgres shouldn't fail the default `cargo
/// test`.
#[tokio::test]
async fn postgres_full_suite() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!("skipping postgres_full_suite: TEST_POSTGRES_URL not set");
        return;
    };
    let db = Db::connect(&url).await.unwrap();
    sqlx::query("DROP SCHEMA public CASCADE")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("CREATE SCHEMA public")
        .execute(&db.pool)
        .await
        .unwrap();
    system::ensure_system_tables(&db).await.unwrap();
    common::full_suite(db).await;
}
