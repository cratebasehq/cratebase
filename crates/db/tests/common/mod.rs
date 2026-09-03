#![allow(dead_code)] // shared across test binaries; each uses a subset

use cratebase_core::field::{Field, FieldOptions, FieldType};
use cratebase_core::{new_id, now, AuthOptions, Collection, CollectionType};
use cratebase_db::records::{
    create_record, delete_record, get_record, list_records, update_record, ListParams,
};
use cratebase_db::resolver::RequestContext;
use cratebase_db::{collections, Db};
use serde_json::{json, Map};

pub fn posts_collection(name: &str) -> Collection {
    Collection {
        id: new_id(),
        name: name.to_string(),
        collection_type: CollectionType::Base,
        schema: vec![
            Field {
                id: new_id(),
                name: "title".into(),
                field_type: FieldType::Text,
                required: true,
                unique: false,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "views".into(),
                field_type: FieldType::Number,
                required: false,
                unique: false,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "published".into(),
                field_type: FieldType::Bool,
                required: false,
                unique: false,
                options: FieldOptions::default(),
            },
        ],
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

/// End-to-end CRUD + validation + filtering + migration exercise, run
/// against whichever backend the caller connected `db` to. Keeping this in
/// one place means sqlite and postgres are proven to behave identically
/// rather than drifting between two hand-maintained test suites.
pub async fn full_suite(db: Db) {
    // create + get
    let col = posts_collection("posts_full");
    collections::create_collection(&db, &col).await.unwrap();

    let mut data = Map::new();
    data.insert("title".into(), json!("Hello world"));
    data.insert("views".into(), json!(10));
    data.insert("published".into(), json!(true));
    let created = create_record(&db, &col, data).await.unwrap();
    assert_eq!(created["title"], json!("Hello world"));
    assert_eq!(created["views"], json!(10.0));
    assert_eq!(created["published"], json!(true));
    let id = created["id"].as_str().unwrap().to_string();
    let fetched = get_record(&db, &col, &id, None).await.unwrap();
    assert_eq!(fetched["title"], json!("Hello world"));

    // required field validation
    let err = create_record(&db, &col, Map::new()).await.unwrap_err();
    match err {
        cratebase_db::DbError::Validation(fields) => assert!(fields.contains_key("title")),
        other => panic!("expected validation error, got {other:?}"),
    }

    // update + delete
    let mut patch = Map::new();
    patch.insert("title".into(), json!("Published"));
    patch.insert("published".into(), json!(false));
    let updated = update_record(&db, &col, &id, patch).await.unwrap();
    assert_eq!(updated["title"], json!("Published"));
    assert_eq!(updated["published"], json!(false));
    delete_record(&db, &col, &id).await.unwrap();
    let err = get_record(&db, &col, &id, None).await.unwrap_err();
    assert!(matches!(err, cratebase_db::DbError::NotFound));

    // unique constraint
    let mut unique_col = posts_collection("posts_unique");
    unique_col.schema[0].unique = true;
    collections::create_collection(&db, &unique_col)
        .await
        .unwrap();
    let mut dup = Map::new();
    dup.insert("title".into(), json!("dup"));
    create_record(&db, &unique_col, dup.clone()).await.unwrap();
    let err = create_record(&db, &unique_col, dup).await.unwrap_err();
    assert!(matches!(err, cratebase_db::DbError::UniqueViolation(_)));

    // list: filter, sort, paginate
    let list_col = posts_collection("posts_list");
    collections::create_collection(&db, &list_col)
        .await
        .unwrap();
    for i in 0..5 {
        let mut d = Map::new();
        d.insert("title".into(), json!(format!("post-{i}")));
        d.insert("views".into(), json!(i));
        d.insert("published".into(), json!(i % 2 == 0));
        create_record(&db, &list_col, d).await.unwrap();
    }
    let ctx = RequestContext::default();
    let result = list_records(
        &db,
        &list_col,
        &ctx,
        None,
        ListParams {
            filter: Some("published = true"),
            sort: Some("views"),
            page: 1,
            per_page: 10,
        },
    )
    .await
    .unwrap();
    assert_eq!(result.total_items, 3);
    let views: Vec<f64> = result
        .items
        .iter()
        .map(|r| r["views"].as_f64().unwrap())
        .collect();
    assert_eq!(views, vec![0.0, 2.0, 4.0]);

    let page1 = list_records(
        &db,
        &list_col,
        &ctx,
        None,
        ListParams {
            filter: None,
            sort: Some("views"),
            page: 1,
            per_page: 2,
        },
    )
    .await
    .unwrap();
    assert_eq!(page1.items.len(), 2);
    assert_eq!(page1.total_items, 5);
    assert_eq!(page1.total_pages, 3);

    // schema migration: add + drop columns
    let mut mig_col = posts_collection("posts_migrate");
    collections::create_collection(&db, &mig_col).await.unwrap();
    let mut d = Map::new();
    d.insert("title".into(), json!("keep me"));
    create_record(&db, &mig_col, d).await.unwrap();
    let previous = mig_col.clone();
    mig_col.schema.push(Field {
        id: new_id(),
        name: "subtitle".into(),
        field_type: FieldType::Text,
        required: false,
        unique: false,
        options: FieldOptions::default(),
    });
    mig_col.schema.retain(|f| f.name != "published");
    collections::update_collection(&db, &previous, &mig_col)
        .await
        .unwrap();
    let result = list_records(
        &db,
        &mig_col,
        &ctx,
        None,
        ListParams {
            filter: None,
            sort: None,
            page: 1,
            per_page: 10,
        },
    )
    .await
    .unwrap();
    assert_eq!(result.items[0]["title"], json!("keep me"));
    assert_eq!(result.items[0]["subtitle"], serde_json::Value::Null);
    assert!(result.items[0].get("published").is_none());
}
