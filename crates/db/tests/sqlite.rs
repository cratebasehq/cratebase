//! End-to-end exercise of the public API on SQLite: connect, bootstrap,
//! create a collection through the store, write and read rows through
//! the engine, migrate the schema, use params and logs.

use cratebase_core::{Collection, CollectionType, Field, FieldKind, FieldType, Settings};
use cratebase_db::{logs, migrations, params, schema, Db, DbError, Executor, Sql};
use serde_json::json;

fn posts() -> Collection {
    let mut c = Collection::new("posts", CollectionType::Base);
    let pos = c.fields.len() - 2;
    c.fields.insert(
        pos,
        Field::new("title", FieldKind::default_for(FieldType::Text)),
    );
    c.fields.insert(
        pos + 1,
        Field::new("views", FieldKind::default_for(FieldType::Number)),
    );
    c.fields
        .insert(pos + 2, Field::new("published", FieldKind::Bool {}));
    c.indexes = vec!["CREATE UNIQUE INDEX `idx_posts_title` ON `posts` (`title`)".into()];
    c.list_rule = Some(String::new());
    c
}

async fn full_suite(db: Db) {
    db.bootstrap().await.unwrap();

    // System collections are seeded with PocketBase's ids.
    assert_eq!(
        db.collections.get("_superusers").unwrap().id,
        "pbc_3142635823"
    );
    assert_eq!(db.collections.get("users").unwrap().id, "_pb_users_auth_");
    let runner = migrations::Runner::core();
    let listed = runner.list(&db).await.unwrap();
    assert!(listed.iter().all(|(_, applied)| applied.is_some()));

    // Collection lifecycle through the store.
    let posts = posts();
    let stored = db.collections.insert(&*db.engine, &posts).await.unwrap();
    assert_eq!(stored.to_json(), posts.to_json());
    assert!(db.engine.table_exists("posts").await.unwrap());

    db.execute(
        "INSERT INTO \"posts\" (\"id\", \"title\", \"views\", \"published\", \"created\", \"updated\") \
         VALUES ($1, $2, $3, $4, $5, $6)",
        &[
            Sql::from("abcdefghijklmno"),
            Sql::from("Hello"),
            Sql::Real(3.0),
            Sql::from(true),
            Sql::from("2026-09-03 12:44:06.146Z"),
            Sql::from("2026-09-03 12:44:06.146Z"),
        ],
    )
    .await
    .unwrap();
    let row = db
        .query_one(
            "SELECT * FROM \"posts\" WHERE \"id\" = $1",
            &[Sql::from("abcdefghijklmno")],
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_str("title"), Some("Hello"));
    assert_eq!(row.get_f64("views"), Some(3.0));
    assert_eq!(row.get_i64("published"), Some(1));

    let err = db
        .execute(
            "INSERT INTO \"posts\" (\"id\", \"title\") VALUES ($1, $2)",
            &[Sql::from("bcdefghijklmnop"), Sql::from("Hello")],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DbError::UniqueViolation(_)), "{err:?}");
    let app_err: cratebase_core::AppError = err.into();
    assert_eq!(
        app_err.body().data["title"]["code"],
        "validation_not_unique"
    );

    // Transaction with rollback on drop.
    {
        let tx = db.begin().await.unwrap();
        tx.execute("DELETE FROM \"posts\"", &[]).await.unwrap();
        assert_eq!(
            tx.query_scalar("SELECT COUNT(*) FROM \"posts\"", &[])
                .await
                .unwrap()
                .and_then(|v| v.as_i64()),
            Some(0)
        );
    }
    assert_eq!(
        db.query_scalar("SELECT COUNT(*) FROM \"posts\"", &[])
            .await
            .unwrap()
            .and_then(|v| v.as_i64()),
        Some(1)
    );

    // Schema migration: rename field (data kept), drop field, add field.
    let mut next = (*db.collections.get("posts").unwrap()).clone();
    next.fields
        .iter_mut()
        .find(|f| f.name == "title")
        .unwrap()
        .name = "heading".into();
    next.fields.retain(|f| f.name != "published");
    next.fields
        .push(Field::new("meta", FieldKind::default_for(FieldType::Json)));
    next.indexes = vec!["CREATE UNIQUE INDEX `idx_posts_heading` ON `posts` (`heading`)".into()];
    db.collections.update(&*db.engine, &next).await.unwrap();
    let cols = db.engine.table_columns("posts").await.unwrap();
    assert!(cols.contains(&"heading".to_string()));
    assert!(cols.contains(&"meta".to_string()));
    assert!(!cols.contains(&"published".to_string()));
    assert_eq!(
        db.engine.table_indexes("posts").await.unwrap(),
        vec!["idx_posts_heading"]
    );
    let row = db
        .query_one("SELECT * FROM \"posts\"", &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_str("heading"), Some("Hello"));
    assert_eq!(
        schema::physical_type(db.backend, next.field("views").unwrap()),
        db.backend.number_type()
    );

    // View over the table.
    let mut view = Collection::new("post_titles", CollectionType::View);
    view.view_query = "SELECT id, heading FROM posts".into();
    db.collections.insert(&*db.engine, &view).await.unwrap();
    assert_eq!(
        db.engine.table_columns("post_titles").await.unwrap(),
        vec!["id", "heading"]
    );

    // Params + settings.
    let mut s = Settings::default();
    s.meta.app_name = "Test".into();
    params::save_settings(&db, &s).await.unwrap();
    assert_eq!(
        params::load_settings(&db).await.unwrap().meta.app_name,
        "Test"
    );
    params::set(&db, "k", "v").await.unwrap();
    assert_eq!(params::get(&db, "k").await.unwrap().as_deref(), Some("v"));

    // Logs live on the auxiliary engine.
    logs::insert_batch(
        &*db.logs,
        vec![logs::LogEntry::new(
            0,
            "GET /api/health",
            json!({"type": "request"}),
        )],
    )
    .await
    .unwrap();
    let page = logs::list(
        &*db.logs,
        logs::ListParams {
            page: 1,
            per_page: 20,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.total_items, 1);
    assert_eq!(page.items[0].message, "GET /api/health");
    assert_eq!(logs::stats(&*db.logs, None).await.unwrap().len(), 1);

    db.collections
        .delete(&*db.engine, "post_titles")
        .await
        .unwrap();
    db.collections.delete(&*db.engine, "posts").await.unwrap();
    assert!(!db.engine.table_exists("posts").await.unwrap());
    db.close().await.unwrap();
}

#[tokio::test]
async fn sqlite_memory_full_suite() {
    let db = Db::connect("sqlite::memory:", "").await.unwrap();
    full_suite(db).await;
}

#[tokio::test]
async fn sqlite_file_full_suite() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_string_lossy().to_string();
    let db = Db::connect(&format!("sqlite:{data}/data.db"), &data)
        .await
        .unwrap();
    full_suite(db).await;
    assert!(dir.path().join("auxiliary.db").exists());
}
