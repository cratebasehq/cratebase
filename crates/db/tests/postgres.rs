//! The same lifecycle on Postgres. Skips (rather than fails) when
//! `TEST_POSTGRES_URL` is unset so machines without a server still pass
//! the default `cargo test`. The target database is wiped.

use cratebase_core::{Collection, CollectionType, Field, FieldKind, FieldType};
use cratebase_db::{params, Backend, Db, DbError, Executor, Sql};

#[tokio::test]
async fn postgres_full_suite() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!("skipping postgres_full_suite: TEST_POSTGRES_URL not set");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let db = Db::connect(&url, &dir.path().to_string_lossy())
        .await
        .unwrap();
    assert_eq!(db.backend, Backend::Postgres);
    db.execute("DROP SCHEMA public CASCADE", &[]).await.unwrap();
    db.execute("CREATE SCHEMA public", &[]).await.unwrap();
    db.bootstrap().await.unwrap();

    assert_eq!(
        db.collections.get("_superusers").unwrap().id,
        "pbc_3142635823"
    );
    assert!(db.engine.table_exists("users").await.unwrap());
    let idx = db.engine.table_indexes("users").await.unwrap();
    assert!(idx.iter().any(|i| i.starts_with("idx_email_")), "{idx:?}");

    let mut posts = Collection::new("posts", CollectionType::Base);
    let pos = posts.fields.len() - 2;
    posts.fields.insert(
        pos,
        Field::new("title", FieldKind::default_for(FieldType::Text)),
    );
    posts.fields.insert(
        pos + 1,
        Field::new("views", FieldKind::default_for(FieldType::Number)),
    );
    posts
        .fields
        .insert(pos + 2, Field::new("published", FieldKind::Bool {}));
    posts.indexes = vec!["CREATE UNIQUE INDEX `idx_posts_title` ON `posts` (`title`)".into()];
    db.collections.insert(&*db.engine, &posts).await.unwrap();
    assert_eq!(
        db.engine.table_columns("posts").await.unwrap(),
        vec!["id", "title", "views", "published", "created", "updated"]
    );

    db.execute(
        "INSERT INTO \"posts\" (\"id\", \"title\", \"views\", \"published\") VALUES ($1, $2, $3, $4)",
        &[
            Sql::from("abcdefghijklmno"),
            Sql::from("Hello"),
            Sql::Int(3),
            Sql::from(true),
        ],
    )
    .await
    .unwrap();
    let row = db
        .query_one("SELECT * FROM \"posts\"", &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_f64("views"), Some(3.0));
    assert_eq!(row.get_i64("published"), Some(1));
    assert_eq!(row.get_str("created"), Some(""));
    let err = db
        .execute(
            "INSERT INTO \"posts\" (\"id\", \"title\") VALUES ($1, $2)",
            &[Sql::from("bcdefghijklmnop"), Sql::from("Hello")],
        )
        .await
        .unwrap_err();
    match err {
        DbError::UniqueViolation(c) => assert_eq!(c, "idx_posts_title"),
        other => panic!("{other:?}"),
    }

    let mut next = (*db.collections.get("posts").unwrap()).clone();
    next.name = "articles".into();
    next.fields
        .iter_mut()
        .find(|f| f.name == "title")
        .unwrap()
        .name = "heading".into();
    next.fields.retain(|f| f.name != "published");
    next.indexes = vec!["CREATE UNIQUE INDEX `idx_posts_heading` ON `posts` (`heading`)".into()];
    db.collections.update(&*db.engine, &next).await.unwrap();
    assert!(db.engine.table_exists("articles").await.unwrap());
    assert_eq!(
        db.engine.table_indexes("articles").await.unwrap(),
        vec!["idx_posts_heading"]
    );
    let row = db
        .query_one("SELECT * FROM \"articles\"", &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_str("heading"), Some("Hello"));

    let mut view = Collection::new("titles", CollectionType::View);
    view.view_query = "SELECT id, heading FROM articles".into();
    db.collections.insert(&*db.engine, &view).await.unwrap();
    assert_eq!(
        db.engine.table_columns("titles").await.unwrap(),
        vec!["id", "heading"]
    );

    params::set(&db, "k", "v").await.unwrap();
    assert_eq!(params::get(&db, "k").await.unwrap().as_deref(), Some("v"));

    db.collections.delete(&*db.engine, "titles").await.unwrap();
    db.collections
        .delete(&*db.engine, "articles")
        .await
        .unwrap();
    assert!(!db.engine.table_exists("articles").await.unwrap());
    db.close().await.unwrap();
}
