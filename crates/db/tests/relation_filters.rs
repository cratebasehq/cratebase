//! End-to-end coverage for the "any of" filter operators (`?=`, ...) and
//! relation dot-notation (`author.name = ...`), exercised through the same
//! `list_records`/`ListParams::filter` path a real `?filter=` query param
//! takes.

use cratebase_core::field::{Field, FieldOptions, FieldType};
use cratebase_core::{new_id, now, AuthOptions, Collection, CollectionType};
use cratebase_db::records::{create_record, list_records, ListParams};
use cratebase_db::resolver::RequestContext;
use cratebase_db::{collections, system, Db};
use serde_json::{json, Map};

fn users_collection() -> Collection {
    Collection {
        id: new_id(),
        name: "rf_users".into(),
        collection_type: CollectionType::Base,
        schema: vec![Field {
            id: new_id(),
            name: "name".into(),
            field_type: FieldType::Text,
            required: true,
            unique: false,
            options: FieldOptions::default(),
        }],
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

fn posts_collection(users_id: &str) -> Collection {
    Collection {
        id: new_id(),
        name: "rf_posts".into(),
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
                name: "tags".into(),
                field_type: FieldType::Select,
                required: false,
                unique: false,
                options: FieldOptions {
                    values: Some(vec!["rust".into(), "go".into(), "db".into()]),
                    multiple: Some(true),
                    max_select: Some(3),
                    ..Default::default()
                },
            },
            Field {
                id: new_id(),
                name: "author".into(),
                field_type: FieldType::Relation,
                required: false,
                unique: false,
                options: FieldOptions {
                    collection_id: Some(users_id.to_string()),
                    ..Default::default()
                },
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

async fn titles(db: &Db, col: &Collection, filter: &str) -> Vec<String> {
    let ctx = RequestContext::default();
    let result = list_records(
        db,
        col,
        &ctx,
        None,
        ListParams {
            filter: Some(filter),
            sort: Some("title"),
            page: 1,
            per_page: 50,
        },
    )
    .await
    .unwrap_or_else(|e| panic!("filter {filter:?} failed: {e:?}"));
    result
        .items
        .iter()
        .map(|r| r["title"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn any_of_and_relation_dot_notation_filters() {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    system::ensure_system_tables(&db).await.unwrap();

    let users = users_collection();
    collections::create_collection(&db, &users).await.unwrap();
    let mut alice = Map::new();
    alice.insert("name".into(), json!("Alice"));
    let alice = create_record(&db, &users, alice).await.unwrap();
    let alice_id = alice["id"].as_str().unwrap().to_string();

    let mut bob = Map::new();
    bob.insert("name".into(), json!("Bob"));
    let bob = create_record(&db, &users, bob).await.unwrap();
    let bob_id = bob["id"].as_str().unwrap().to_string();

    let posts = posts_collection(&users.id);
    collections::create_collection(&db, &posts).await.unwrap();

    let mut p1 = Map::new();
    p1.insert("title".into(), json!("post-rust-db"));
    p1.insert("tags".into(), json!(["rust", "db"]));
    p1.insert("author".into(), json!(alice_id));
    create_record(&db, &posts, p1).await.unwrap();

    let mut p2 = Map::new();
    p2.insert("title".into(), json!("post-go"));
    p2.insert("tags".into(), json!(["go"]));
    p2.insert("author".into(), json!(bob_id));
    create_record(&db, &posts, p2).await.unwrap();

    let mut p3 = Map::new();
    p3.insert("title".into(), json!("post-rust-only"));
    p3.insert("tags".into(), json!(["rust"]));
    p3.insert("author".into(), json!(bob_id));
    create_record(&db, &posts, p3).await.unwrap();

    // `?=`: any element of the multi-valued `tags` select matches.
    let mut got = titles(&db, &posts, r#"tags ?= "rust""#).await;
    got.sort();
    assert_eq!(got, vec!["post-rust-db", "post-rust-only"]);

    let got = titles(&db, &posts, r#"tags ?= "go""#).await;
    assert_eq!(got, vec!["post-go"]);

    // `?!=`: any element differs from the literal.
    let mut got = titles(&db, &posts, r#"tags ?!= "rust""#).await;
    got.sort();
    assert_eq!(got, vec!["post-go", "post-rust-db"]);

    // Bare `=` on a multi-valued field requires every element to match.
    let got = titles(&db, &posts, r#"tags = "rust""#).await;
    assert_eq!(got, vec!["post-rust-only"]);

    // Relation dot-notation: single-valued `author` traverses into `rf_users`.
    let got = titles(&db, &posts, r#"author.name = "Alice""#).await;
    assert_eq!(got, vec!["post-rust-db"]);

    let mut got = titles(&db, &posts, r#"author.name = "Bob""#).await;
    got.sort();
    assert_eq!(got, vec!["post-go", "post-rust-only"]);

    // Combine an any-of operator with dot-notation and boolean composition.
    let got = titles(
        &db,
        &posts,
        r#"author.name = "Bob" && tags ?= "go""#,
    )
    .await;
    assert_eq!(got, vec!["post-go"]);
}
