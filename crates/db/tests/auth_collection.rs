mod common;

use cratebase_core::{new_id, now, AuthOptions, Collection, CollectionType};
use cratebase_db::records::{create_record_with_id, get_record};
use cratebase_db::{collections, system, Db};
use serde_json::{json, Map};

fn users_collection() -> Collection {
    Collection {
        id: new_id(),
        name: "users".into(),
        collection_type: CollectionType::Auth,
        schema: vec![],
        list_rule: Some(String::new()),
        view_rule: Some(String::new()),
        create_rule: Some(String::new()),
        update_rule: Some(String::new()),
        delete_rule: Some(String::new()),
        auth_options: AuthOptions::default(),
        view_query: None,
        created: now(),
        updated: now(),
    }
}

#[tokio::test]
async fn auth_record_stores_email_and_hides_password_hash() {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    system::ensure_system_tables(&db).await.unwrap();
    let col = users_collection();
    collections::create_collection(&db, &col).await.unwrap();

    let mut data = Map::new();
    data.insert("email".into(), json!("alice@example.com"));
    data.insert("password_hash".into(), json!("hashed-secret"));
    let id = new_id();
    let record = create_record_with_id(&db, &col, id.clone(), data).await.unwrap();

    assert_eq!(record["email"], json!("alice@example.com"));
    assert!(record.get("password_hash").is_none(), "password hash must never be exposed");
    assert!(record.get("password").is_none());

    let fetched = get_record(&db, &col, &id, None).await.unwrap();
    assert_eq!(fetched["email"], json!("alice@example.com"));
}

#[tokio::test]
async fn duplicate_email_is_rejected() {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    system::ensure_system_tables(&db).await.unwrap();
    let col = users_collection();
    collections::create_collection(&db, &col).await.unwrap();

    let mut data = Map::new();
    data.insert("email".into(), json!("dup@example.com"));
    data.insert("password_hash".into(), json!("h1"));
    create_record_with_id(&db, &col, new_id(), data.clone()).await.unwrap();

    let err = create_record_with_id(&db, &col, new_id(), data).await.unwrap_err();
    assert!(matches!(err, cratebase_db::DbError::UniqueViolation(_)));
}
