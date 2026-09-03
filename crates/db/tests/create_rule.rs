mod common;

use cratebase_db::resolver::{evaluate_create_rule, AuthContext, RequestContext};
use cratebase_db::{system, Db};
use serde_json::json;

#[tokio::test]
async fn create_rule_uses_submitted_data_for_bare_fields() {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    system::ensure_system_tables(&db).await.unwrap();

    let mut col = common::posts_collection("posts_create_rule");
    col.create_rule = Some("published = true".to_string());
    cratebase_db::collections::create_collection(&db, &col)
        .await
        .unwrap();

    let mut data = serde_json::Map::new();
    data.insert("title".into(), json!("draft"));
    data.insert("published".into(), json!(false));
    let ctx = RequestContext {
        auth: None,
        data: Some(data),
    };
    let allowed = evaluate_create_rule(&db, &col.create_rule, &col, &ctx)
        .await
        .unwrap();
    assert!(!allowed, "published=false should fail the create rule");

    let mut data = serde_json::Map::new();
    data.insert("title".into(), json!("live"));
    data.insert("published".into(), json!(true));
    let ctx = RequestContext {
        auth: None,
        data: Some(data),
    };
    let allowed = evaluate_create_rule(&db, &col.create_rule, &col, &ctx)
        .await
        .unwrap();
    assert!(allowed, "published=true should pass the create rule");
}

#[tokio::test]
async fn superuser_bypasses_create_rule() {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    system::ensure_system_tables(&db).await.unwrap();

    let mut col = common::posts_collection("posts_admin_bypass");
    col.create_rule = None; // locked to admins only
    cratebase_db::collections::create_collection(&db, &col)
        .await
        .unwrap();

    let ctx = RequestContext {
        auth: Some(AuthContext {
            id: "admin-1".into(),
            collection_id: String::new(),
            is_superuser: true,
            record: Default::default(),
        }),
        data: None,
    };
    let allowed = evaluate_create_rule(&db, &col.create_rule, &col, &ctx)
        .await
        .unwrap();
    assert!(allowed);
}
