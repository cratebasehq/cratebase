mod common;

use cratebase_core::field::{Field, FieldOptions, FieldType};
use cratebase_core::{new_id, now, AuthOptions, Collection, CollectionType};
use cratebase_db::records::{create_record, update_record};
use cratebase_db::{collections, system, Db};
use serde_json::{json, Map};

fn autodate_collection(name: &str) -> Collection {
    Collection {
        id: new_id(),
        name: name.to_string(),
        collection_type: CollectionType::Base,
        schema: vec![
            Field {
                id: new_id(),
                name: "title".into(),
                field_type: FieldType::Text,
                required: false,
                unique: false,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "published_at".into(),
                field_type: FieldType::Autodate,
                required: false,
                unique: false,
                options: FieldOptions {
                    on_create: Some(true),
                    ..Default::default()
                },
            },
            Field {
                id: new_id(),
                name: "touched_at".into(),
                field_type: FieldType::Autodate,
                required: false,
                unique: false,
                options: FieldOptions {
                    on_update: Some(true),
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

#[tokio::test]
async fn autodate_fires_on_configured_lifecycle_events_only() {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    system::ensure_system_tables(&db).await.unwrap();

    let col = autodate_collection("events");
    collections::create_collection(&db, &col).await.unwrap();

    // Client-supplied values for autodate fields are ignored entirely —
    // the server always computes its own timestamp.
    let mut data = Map::new();
    data.insert("title".into(), json!("launch"));
    data.insert("published_at".into(), json!("2000-01-01T00:00:00Z"));
    let created = create_record(&db, &col, data).await.unwrap();

    let published_at = created["published_at"].as_str().unwrap().to_string();
    assert_ne!(
        published_at, "2000-01-01T00:00:00Z",
        "client-supplied autodate value must be ignored"
    );
    assert!(
        chrono::DateTime::parse_from_rfc3339(&published_at).is_ok(),
        "onCreate autodate field should be set to a real timestamp on create"
    );
    assert!(
        created["touched_at"].is_null(),
        "onUpdate-only autodate field must stay unset until the first update"
    );

    let id = created["id"].as_str().unwrap().to_string();

    let mut update_data = Map::new();
    update_data.insert("title".into(), json!("launch (updated)"));
    let updated = update_record(&db, &col, &id, update_data).await.unwrap();

    assert_eq!(
        updated["published_at"], created["published_at"],
        "onCreate-only autodate field must not change on update"
    );
    let touched_at = updated["touched_at"].as_str().unwrap();
    assert!(
        chrono::DateTime::parse_from_rfc3339(touched_at).is_ok(),
        "onUpdate autodate field should be set on the first update"
    );
}
