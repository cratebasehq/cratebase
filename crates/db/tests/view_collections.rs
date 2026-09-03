mod common;

use cratebase_core::field::{Field, FieldOptions, FieldType};
use cratebase_core::{new_id, now, AuthOptions, Collection, CollectionType};
use cratebase_db::records::{create_record, list_records, ListParams};
use cratebase_db::resolver::RequestContext;
use cratebase_db::{collections, system, Db, DbError};
use serde_json::{json, Map};

/// A `View` collection backed by `SELECT ... FROM cb_<source> WHERE
/// published = 1`. Mirrors `common::posts_collection`'s schema for the
/// `title` field it re-exposes.
fn published_posts_view(source: &str) -> Collection {
    Collection {
        id: new_id(),
        name: "published_posts".to_string(),
        collection_type: CollectionType::View,
        schema: vec![Field {
            id: new_id(),
            name: "title".into(),
            field_type: FieldType::Text,
            required: false,
            unique: false,
            options: FieldOptions::default(),
        }],
        list_rule: Some(String::new()),
        view_rule: Some(String::new()),
        create_rule: Some(String::new()),
        update_rule: Some(String::new()),
        delete_rule: Some(String::new()),
        auth_options: AuthOptions::default(),
        view_query: Some(format!(
            "SELECT id, title, created, updated FROM cb_{source} WHERE published = 1"
        )),
        created: now(),
        updated: now(),
    }
}

#[tokio::test]
async fn view_collection_lists_filtered_rows_and_rejects_writes() {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    system::ensure_system_tables(&db).await.unwrap();

    let posts = common::posts_collection("posts_for_view");
    collections::create_collection(&db, &posts).await.unwrap();

    for (title, published) in [("One", true), ("Two", false), ("Three", true)] {
        let mut data = Map::new();
        data.insert("title".into(), json!(title));
        data.insert("published".into(), json!(published));
        create_record(&db, &posts, data).await.unwrap();
    }

    let view = published_posts_view(&posts.name);
    collections::create_collection(&db, &view).await.unwrap();

    let ctx = RequestContext {
        auth: None,
        data: None,
    };
    let result = list_records(
        &db,
        &view,
        &ctx,
        None,
        ListParams {
            filter: None,
            sort: None,
            page: 1,
            per_page: 30,
        },
    )
    .await
    .unwrap();
    assert_eq!(result.total_items, 2, "only the two published posts");
    let titles: Vec<&str> = result
        .items
        .iter()
        .map(|r| r["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"One"));
    assert!(titles.contains(&"Three"));
    assert!(
        !titles.contains(&"Two"),
        "unpublished post must be excluded"
    );

    // Writes against a view collection are rejected, not routed to a
    // nonexistent physical table.
    let mut data = Map::new();
    data.insert("title".into(), json!("Nope"));
    let err = create_record(&db, &view, data).await.unwrap_err();
    assert!(matches!(err, DbError::ViewReadOnly));
    let err = cratebase_db::records::delete_record(&db, &view, "does-not-exist")
        .await
        .unwrap_err();
    assert!(matches!(err, DbError::ViewReadOnly));

    // `view_query` can change on update; the view is recreated to match.
    let mut updated = view.clone();
    updated.view_query = Some(format!(
        "SELECT id, title, created, updated FROM cb_{} WHERE published = 0",
        posts.name
    ));
    collections::update_collection(&db, &view, &updated)
        .await
        .unwrap();
    let result = list_records(
        &db,
        &updated,
        &ctx,
        None,
        ListParams {
            filter: None,
            sort: None,
            page: 1,
            per_page: 30,
        },
    )
    .await
    .unwrap();
    assert_eq!(result.total_items, 1);
    assert_eq!(result.items[0]["title"], "Two");

    // Dropping the collection drops the backing view too (not a table drop
    // attempt against something that was never a table).
    collections::delete_collection(&db, &updated).await.unwrap();
    let recreate = published_posts_view(&posts.name);
    collections::create_collection(&db, &recreate)
        .await
        .expect("name is free again after delete_collection dropped the view");
}
