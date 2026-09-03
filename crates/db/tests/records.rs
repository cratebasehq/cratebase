mod common;

use cratebase_db::{system, Db};

#[tokio::test]
async fn sqlite_full_suite() {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    system::ensure_system_tables(&db).await.unwrap();
    common::full_suite(db).await;
}
