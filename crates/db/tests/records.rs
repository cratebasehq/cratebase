//! End-to-end tests for the record read/write layer against an in-memory
//! SQLite database.
//!
//! The expectations here are PocketBase v0.40.2's, as measured by
//! `tests/conformance` — including the places where it coerces instead of
//! rejecting, and the places where a rule failure is not an error.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cratebase_core::record::project_fields;
use cratebase_core::{
    codes, AppError, Collection, CollectionType, Field, FieldKind, FieldType, Record,
    SerializeOptions,
};
use cratebase_db::context::{AuthContext, RequestContext};
use cratebase_db::engine::{Executor, Row, Sql};
use cratebase_db::error::DbError;
use cratebase_db::records::{self, ListParams};
use cratebase_db::{expand, rules, validate, Db, Dialect};
use serde_json::{json, Map, Value};

// --- fixture --------------------------------------------------------------

fn text(name: &str) -> Field {
    Field::new(name, FieldKind::default_for(FieldType::Text))
}

fn insert_fields(c: &mut Collection, fields: Vec<Field>) {
    // Keep `created`/`updated` last, as PocketBase's scaffold does.
    let pos = c.fields.len() - 2;
    c.fields.splice(pos..pos, fields);
}

/// `authors`, `posts` (every field type) and `comments` (cascading
/// back-relation), mirroring the conformance suite's schema.
async fn fixture() -> Db {
    let db = Db::memory().await.unwrap();

    let mut authors = Collection::new("authors", CollectionType::Base);
    insert_fields(&mut authors, vec![text("name")]);
    db.collections.insert(&*db.engine, &authors).await.unwrap();

    let mut posts = Collection::new("posts", CollectionType::Base);
    let mut title = Field::new(
        "title",
        FieldKind::Text {
            min: 3,
            max: 20,
            pattern: String::new(),
            autogenerate_pattern: String::new(),
            primary_key: false,
        },
    );
    title.required = true;
    insert_fields(
        &mut posts,
        vec![
            title,
            text("slug"),
            Field::new(
                "code",
                FieldKind::Text {
                    min: 0,
                    max: 0,
                    pattern: String::new(),
                    autogenerate_pattern: "[a-z]{6}".into(),
                    primary_key: false,
                },
            ),
            Field::new("content", FieldKind::default_for(FieldType::Editor)),
            Field::new(
                "views",
                FieldKind::Number {
                    min: Some(0.0),
                    max: Some(1000.0),
                    only_int: true,
                },
            ),
            Field::new(
                "score",
                FieldKind::Number {
                    min: None,
                    max: None,
                    only_int: false,
                },
            ),
            Field::new("published", FieldKind::Bool {}),
            Field::new(
                "contact",
                FieldKind::Email {
                    except_domains: vec![],
                    only_domains: vec![],
                },
            ),
            Field::new(
                "site",
                FieldKind::Url {
                    except_domains: vec![],
                    only_domains: vec![],
                },
            ),
            Field::new(
                "when",
                FieldKind::Date {
                    min: cratebase_core::DateTime::parse("2020-01-01 00:00:00.000Z"),
                    max: cratebase_core::DateTime::parse("2030-01-01 00:00:00.000Z"),
                },
            ),
            Field::new(
                "author",
                FieldKind::Relation {
                    collection_id: authors.id.clone(),
                    cascade_delete: false,
                    min_select: 0,
                    max_select: 1,
                },
            ),
            Field::new(
                "category",
                FieldKind::Select {
                    values: vec!["news".into(), "blog".into()],
                    max_select: 1,
                },
            ),
            Field::new(
                "tags",
                FieldKind::Select {
                    values: vec!["go".into(), "rust".into(), "js".into()],
                    max_select: 2,
                },
            ),
            Field::new(
                "cover",
                FieldKind::File {
                    max_select: 2,
                    max_size: 10,
                    mime_types: vec!["image/png".into()],
                    thumbs: vec![],
                    protected: false,
                },
            ),
            Field::new("meta", FieldKind::Json { max_size: 0 }),
            Field::new("loc", FieldKind::GeoPoint {}),
        ],
    );
    posts.indexes =
        vec!["CREATE UNIQUE INDEX `idx_posts_slug` ON `posts` (`slug`) WHERE `slug` != ''".into()];
    db.collections.insert(&*db.engine, &posts).await.unwrap();

    let mut comments = Collection::new("comments", CollectionType::Base);
    insert_fields(
        &mut comments,
        vec![
            Field::new(
                "post",
                FieldKind::Relation {
                    collection_id: posts.id.clone(),
                    cascade_delete: true,
                    min_select: 0,
                    max_select: 1,
                },
            ),
            text("text"),
            Field::new(
                "author",
                FieldKind::Relation {
                    collection_id: authors.id.clone(),
                    cascade_delete: false,
                    min_select: 0,
                    max_select: 1,
                },
            ),
        ],
    );
    db.collections.insert(&*db.engine, &comments).await.unwrap();
    db
}

fn collection(db: &Db, name: &str) -> Arc<Collection> {
    db.collections.get(name).unwrap()
}

/// Build a record from a JSON body and create it, superuser-style.
async fn create(db: &Db, name: &str, body: Value) -> Record {
    let mut record = record_from(db, name, body);
    records::create(db, &db.collections, &mut record)
        .await
        .unwrap();
    record
}

fn record_from(db: &Db, name: &str, body: Value) -> Record {
    let body = body.as_object().cloned().unwrap_or_default();
    records::from_body(collection(db, name), &body)
}

async fn try_create(db: &Db, name: &str, body: Value) -> Result<Record, DbError> {
    let mut record = record_from(db, name, body);
    records::create(db, &db.collections, &mut record).await?;
    Ok(record)
}

fn field_errors(err: DbError) -> BTreeMap<String, String> {
    match err {
        DbError::Validation(fields) => fields.into_iter().map(|(k, v)| (k, v.code)).collect(),
        other => panic!("expected a validation error, got {other:?}"),
    }
}

fn superuser() -> RequestContext {
    RequestContext::superuser()
}

// --- an Executor that counts what it is asked to run ----------------------

struct Counting<'a> {
    inner: &'a dyn Executor,
    queries: Mutex<Vec<String>>,
}

impl<'a> Counting<'a> {
    fn new(inner: &'a dyn Executor) -> Self {
        Counting {
            inner,
            queries: Mutex::new(Vec::new()),
        }
    }

    fn matching(&self, needle: &str) -> usize {
        self.queries
            .lock()
            .unwrap()
            .iter()
            .filter(|q| q.contains(needle))
            .count()
    }
}

#[async_trait]
impl Executor for Counting<'_> {
    fn dialect(&self) -> Dialect {
        self.inner.dialect()
    }
    async fn query(&self, sql: &str, params: &[Sql]) -> Result<Vec<Row>, DbError> {
        self.queries.lock().unwrap().push(sql.to_string());
        self.inner.query(sql, params).await
    }
    async fn execute(&self, sql: &str, params: &[Sql]) -> Result<u64, DbError> {
        self.queries.lock().unwrap().push(sql.to_string());
        self.inner.execute(sql, params).await
    }
}

// --- tests ----------------------------------------------------------------

#[tokio::test]
async fn crud_round_trip_with_zero_values_and_autodate() {
    let db = fixture().await;
    let posts = collection(&db, "posts");

    let created = create(&db, "posts", json!({"title": "Created", "views": 1})).await;
    assert_eq!(created.id().len(), 15);
    assert!(created
        .id()
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
    assert!(!created.is_new());

    // Every field's zero value, exactly as PocketBase reports it.
    let json = created.to_json(SerializeOptions::default());
    assert_eq!(json["title"], "Created");
    assert_eq!(json["views"], 1);
    assert_eq!(json["score"], 0);
    assert_eq!(json["published"], false);
    assert_eq!(json["content"], "");
    assert_eq!(json["author"], "");
    assert_eq!(json["tags"], json!([]));
    assert_eq!(json["cover"], json!([]));
    assert_eq!(json["category"], "");
    assert_eq!(json["when"], "");
    assert_eq!(json["meta"], Value::Null);
    assert_eq!(json["collectionId"], posts.id);
    assert_eq!(json["collectionName"], "posts");
    // `code` has an autogeneratePattern, so it was filled in.
    assert_eq!(json["code"].as_str().unwrap().len(), 6);
    let created_at = json["created"].as_str().unwrap().to_string();
    assert!(created_at.ends_with('Z') && created_at.contains(' '));
    assert_eq!(json["updated"], created_at);

    // Read it back: every value survives the column round trip.
    let loaded = records::find_by_id_raw(&db, &posts, created.id())
        .await
        .unwrap();
    assert_eq!(
        loaded.to_json(SerializeOptions::default()),
        created.to_json(SerializeOptions::default())
    );

    // Update touches only what changed, and bumps `updated`.
    let mut to_update = loaded;
    to_update.set("views", json!(6));
    records::update(&db, &db.collections, &mut to_update)
        .await
        .unwrap();
    assert_eq!(to_update.get("views"), Some(&json!(6)));
    assert_eq!(to_update.get_string("title"), "Created");
    assert!(to_update.get_string("updated") >= created_at);

    // Delete, then it is gone.
    let files = records::delete(&db, &db.collections, &to_update)
        .await
        .unwrap();
    assert!(files.is_empty());
    assert!(matches!(
        records::find_by_id_raw(&db, &posts, to_update.id()).await,
        Err(DbError::NotFound)
    ));
    assert_eq!(records::count(&db, &posts).await.unwrap(), 0);
}

#[tokio::test]
async fn every_field_type_round_trips_through_its_column() {
    let db = fixture().await;
    let posts = collection(&db, "posts");
    let author = create(&db, "authors", json!({"name": "Ann"})).await;

    let created = create(
        &db,
        "posts",
        json!({
            "title": "Shapes",
            "content": "<p>hi</p>",
            "views": 42,
            "score": 1.5,
            "published": true,
            "contact": "a@b.co",
            "site": "https://example.com/x",
            "when": "2024-05-05T12:30:00+02:00",
            "author": author.id(),
            "category": "news",
            "tags": ["go", "rust"],
            "meta": {"x": 1},
            "loc": {"lon": 4.5, "lat": -1.25},
        }),
    )
    .await;

    let loaded = records::find_by_id_raw(&db, &posts, created.id())
        .await
        .unwrap();
    let v = loaded.to_json(SerializeOptions::default());
    assert_eq!(v["content"], "<p>hi</p>");
    assert_eq!(v["views"], 42);
    assert_eq!(v["score"], 1.5);
    assert_eq!(v["published"], true);
    assert_eq!(v["contact"], "a@b.co");
    assert_eq!(v["site"], "https://example.com/x");
    // Dates are normalized to UTC in PocketBase's format.
    assert_eq!(v["when"], "2024-05-05 10:30:00.000Z");
    assert_eq!(v["author"], author.id());
    assert_eq!(v["category"], "news");
    assert_eq!(v["tags"], json!(["go", "rust"]));
    assert_eq!(v["meta"], json!({"x": 1}));
    assert_eq!(v["loc"], json!({"lon": 4.5, "lat": -1.25}));

    // `?fields=` projection stays a pure function over the serialized
    // record — the db layer never applies it.
    let mut projected = v.clone();
    project_fields(&mut projected, "id,title");
    assert_eq!(projected, json!({"id": loaded.id(), "title": "Shapes"}));
    assert_eq!(loaded.to_json(SerializeOptions::default()), v);
}

#[tokio::test]
async fn pagination_envelope_and_skip_total() {
    let db = fixture().await;
    let posts = collection(&db, "posts");
    for i in 0..3 {
        create(
            &db,
            "posts",
            json!({"title": format!("Post {i}"), "views": i}),
        )
        .await;
    }
    let ctx = superuser();

    let page = records::list(
        &db,
        &db.collections,
        &ctx,
        &posts,
        ListParams {
            page: 1,
            per_page: 2,
            sort: Some("views"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.page, 1);
    assert_eq!(page.per_page, 2);
    assert_eq!(page.total_items, 3);
    assert_eq!(page.total_pages, 2);
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].get_string("title"), "Post 0");

    // `page: 0` becomes 1 and `perPage` is capped at 1000.
    let page = records::list(
        &db,
        &db.collections,
        &ctx,
        &posts,
        ListParams {
            page: 0,
            per_page: 5000,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.page, 1);
    assert_eq!(page.per_page, 1000);

    let page = records::list(
        &db,
        &db.collections,
        &ctx,
        &posts,
        ListParams {
            page: 1,
            per_page: 1,
            skip_total: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.total_items, -1);
    assert_eq!(page.total_pages, -1);
    assert_eq!(page.items.len(), 1);
}

#[tokio::test]
async fn sorting_by_column_relation_and_macros() {
    let db = fixture().await;
    let posts = collection(&db, "posts");
    let ann = create(&db, "authors", json!({"name": "Ann"})).await;
    let bob = create(&db, "authors", json!({"name": "Bob"})).await;
    let p1 = create(
        &db,
        "posts",
        json!({"title": "Alpha", "views": 3, "author": bob.id()}),
    )
    .await;
    let p2 = create(
        &db,
        "posts",
        json!({"title": "Beta", "views": 1, "author": ann.id()}),
    )
    .await;
    let ctx = superuser();

    async fn ids(db: &Db, posts: &Arc<Collection>, sort: &str) -> Vec<String> {
        records::list(
            db,
            &db.collections,
            &RequestContext::superuser(),
            posts,
            ListParams {
                page: 1,
                per_page: 30,
                sort: Some(sort),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect()
    }

    assert_eq!(ids(&db, &posts, "views").await, vec![p2.id(), p1.id()]);
    assert_eq!(ids(&db, &posts, "-views").await, vec![p1.id(), p2.id()]);
    assert_eq!(ids(&db, &posts, "+title").await, vec![p1.id(), p2.id()]);
    assert_eq!(ids(&db, &posts, "-created,title").await.len(), 2);
    // Through a single relation, exactly like a filter path.
    assert_eq!(
        ids(&db, &posts, "author.name").await,
        vec![p2.id(), p1.id()]
    );
    assert_eq!(ids(&db, &posts, "@random").await.len(), 2);
    assert_eq!(ids(&db, &posts, "@rowid").await, vec![p1.id(), p2.id()]);

    // An unknown sort field is a bare 400 with an empty `data` object.
    let err = records::list(
        &db,
        &db.collections,
        &ctx,
        &posts,
        ListParams {
            page: 1,
            per_page: 30,
            sort: Some("nope"),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    let body = AppError::from(err).body();
    assert_eq!(body.status, 400);
    assert!(body.data.is_empty());
}

#[tokio::test]
async fn rule_and_user_filter_combine_with_correct_param_offsets() {
    let db = fixture().await;
    let ann = create(&db, "authors", json!({"name": "Ann"})).await;
    let bob = create(&db, "authors", json!({"name": "Bob"})).await;
    create(
        &db,
        "posts",
        json!({"title": "Ann pub", "published": true, "author": ann.id(), "views": 5}),
    )
    .await;
    create(
        &db,
        "posts",
        json!({"title": "Ann draft", "published": false, "author": ann.id(), "views": 50}),
    )
    .await;
    create(
        &db,
        "posts",
        json!({"title": "Bob pub", "published": true, "author": bob.id(), "views": 500}),
    )
    .await;

    // A rule that binds one parameter, plus a filter that binds two more.
    let mut posts = (*collection(&db, "posts")).clone();
    posts.list_rule = Some("published = true".into());
    db.collections.update(&*db.engine, &posts).await.unwrap();
    let posts = collection(&db, "posts");

    let ctx = RequestContext::default();
    let page = records::list(
        &db,
        &db.collections,
        &ctx,
        &posts,
        ListParams {
            page: 1,
            per_page: 30,
            filter: Some("views > 10 && title ~ 'Bob'"),
            sort: Some("title"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.total_items, 1);
    assert_eq!(page.items[0].get_string("title"), "Bob pub");

    // The rule alone still filters the unfiltered list.
    let page = records::list(
        &db,
        &db.collections,
        &ctx,
        &posts,
        ListParams {
            page: 1,
            per_page: 30,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.total_items, 2);

    // A rule referencing `@request.auth` binds the auth value, and the
    // user filter's placeholders continue after it.
    let mut posts_mut = (*posts).clone();
    posts_mut.list_rule = Some("author.name = @request.auth.name".into());
    db.collections
        .update(&*db.engine, &posts_mut)
        .await
        .unwrap();
    let posts = collection(&db, "posts");

    let users = db.collections.get("users").unwrap();
    let mut me = Record::new(users);
    me.set_id("u1");
    me.set("name", json!("Ann"));
    let ctx = RequestContext::default().with_auth(AuthContext::new(me));
    let page = records::list(
        &db,
        &db.collections,
        &ctx,
        &posts,
        ListParams {
            page: 1,
            per_page: 30,
            filter: Some("views >= 5 && views <= 50"),
            sort: Some("title"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|r| r.get_string("title"))
            .collect::<Vec<_>>(),
        vec!["Ann draft", "Ann pub"]
    );
}

#[tokio::test]
async fn list_and_view_rules_deny_without_leaking() {
    let db = fixture().await;
    let post = create(&db, "posts", json!({"title": "Secret"})).await;

    let mut posts = (*collection(&db, "posts")).clone();
    posts.list_rule = Some("published = true".into());
    posts.view_rule = Some("published = true".into());
    db.collections.update(&*db.engine, &posts).await.unwrap();
    let posts = collection(&db, "posts");
    let ctx = RequestContext::default();

    // A rule the record fails yields an empty list, not an error.
    let page = records::list(
        &db,
        &db.collections,
        &ctx,
        &posts,
        ListParams {
            page: 1,
            per_page: 30,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.total_items, 0);
    assert!(page.items.is_empty());

    // The same record via `viewRule` is a 404, never a 403.
    let err = records::find_by_id(&db, &db.collections, &ctx, &posts, post.id(), None)
        .await
        .unwrap_err();
    assert!(matches!(err, DbError::NotFound));
    assert_eq!(AppError::from(err).status(), 404);

    // A superuser sees it.
    assert!(
        records::find_by_id(&db, &db.collections, &superuser(), &posts, post.id(), None)
            .await
            .is_ok()
    );

    // A `None` rule is the 403 case, which the caller must detect itself:
    // the list below is empty rather than an error.
    let mut posts_mut = (*posts).clone();
    posts_mut.list_rule = None;
    db.collections
        .update(&*db.engine, &posts_mut)
        .await
        .unwrap();
    let posts = collection(&db, "posts");
    assert!(rules::is_superuser_only(&posts.list_rule, &ctx));
    assert!(!rules::is_superuser_only(&posts.list_rule, &superuser()));
    let page = records::list(
        &db,
        &db.collections,
        &ctx,
        &posts,
        ListParams {
            page: 1,
            per_page: 30,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(page.items.is_empty());
}

#[tokio::test]
async fn create_rule_is_evaluated_against_the_body() {
    let db = fixture().await;
    let mut posts = (*collection(&db, "posts")).clone();
    posts.create_rule = Some("@request.body.published = true".into());
    db.collections.update(&*db.engine, &posts).await.unwrap();
    let posts = collection(&db, "posts");

    let allow = RequestContext::default().with_body(
        json!({"title": "x", "published": true})
            .as_object()
            .cloned()
            .unwrap(),
    );
    let deny =
        RequestContext::default().with_body(json!({"title": "x"}).as_object().cloned().unwrap());

    let resolver = cratebase_db::CollectionResolver::new(
        posts.clone(),
        &db.collections,
        &allow,
        Dialect::Sqlite,
    );
    assert!(rules::check_create_rule(&db, &resolver, &posts.create_rule)
        .await
        .unwrap());
    let resolver = cratebase_db::CollectionResolver::new(
        posts.clone(),
        &db.collections,
        &deny,
        Dialect::Sqlite,
    );
    assert!(
        !rules::check_create_rule(&db, &resolver, &posts.create_rule)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn expansion_is_one_query_per_collection_per_level() {
    let db = fixture().await;
    let ann = create(&db, "authors", json!({"name": "Ann"})).await;
    let bob = create(&db, "authors", json!({"name": "Bob"})).await;
    let p1 = create(&db, "posts", json!({"title": "First", "author": ann.id()})).await;
    let p2 = create(&db, "posts", json!({"title": "Second", "author": bob.id()})).await;
    for i in 0..3 {
        create(
            &db,
            "comments",
            json!({"post": p1.id(), "text": format!("c{i}"), "author": ann.id()}),
        )
        .await;
    }
    let posts = collection(&db, "posts");
    let ctx = superuser();

    // Single forward relation, two parents: one extra query.
    let counting = Counting::new(&db);
    let page = records::list(
        &counting,
        &db.collections,
        &ctx,
        &posts,
        ListParams {
            page: 1,
            per_page: 30,
            sort: Some("title"),
            expand: Some("author"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(counting.matching("FROM \"authors\""), 1);
    assert_eq!(page.items[0].expand()["author"]["name"], "Ann");
    assert_eq!(page.items[1].expand()["author"]["name"], "Bob");
    // A relation that resolved to nothing is simply absent.
    assert!(page.items[0].expand().get("comments_via_post").is_none());

    // Back-relation + a nested level: one query for comments, one for the
    // authors those comments point at.
    let counting = Counting::new(&db);
    let mut one = vec![records::find_by_id_raw(&db, &posts, p1.id()).await.unwrap()];
    expand::resolve(
        &counting,
        &db.collections,
        &ctx,
        &mut one,
        "comments_via_post.author",
        0,
    )
    .await
    .unwrap();
    assert_eq!(counting.matching("FROM \"comments\""), 1);
    assert_eq!(counting.matching("FROM \"authors\""), 1);
    let comments = one[0].expand()["comments_via_post"].as_array().unwrap();
    assert_eq!(comments.len(), 3);
    assert_eq!(comments[0]["expand"]["author"]["name"], "Ann");

    // A post with no comments gets no back-relation key at all.
    let mut none = vec![records::find_by_id_raw(&db, &posts, p2.id()).await.unwrap()];
    expand::resolve(
        &db,
        &db.collections,
        &ctx,
        &mut none,
        "comments_via_post",
        0,
    )
    .await
    .unwrap();
    assert!(none[0].expand().get("comments_via_post").is_none());
}

#[tokio::test]
async fn expansion_applies_the_target_view_rule_and_handles_multi_relations() {
    let db = fixture().await;
    let ann = create(&db, "authors", json!({"name": "Ann"})).await;
    let post = create(&db, "posts", json!({"title": "Hidden", "author": ann.id()})).await;

    let mut authors = (*collection(&db, "authors")).clone();
    authors.view_rule = Some("name = 'Nobody'".into());
    db.collections.update(&*db.engine, &authors).await.unwrap();
    let posts = collection(&db, "posts");

    // A rule the related record fails simply omits it.
    let mut items = vec![records::find_by_id_raw(&db, &posts, post.id())
        .await
        .unwrap()];
    expand::resolve(
        &db,
        &db.collections,
        &RequestContext::default(),
        &mut items,
        "author",
        0,
    )
    .await
    .unwrap();
    assert!(items[0].expand().get("author").is_none());

    // A superuser bypasses it.
    let mut items = vec![records::find_by_id_raw(&db, &posts, post.id())
        .await
        .unwrap()];
    expand::resolve(&db, &db.collections, &superuser(), &mut items, "author", 0)
        .await
        .unwrap();
    assert_eq!(items[0].expand()["author"]["name"], "Ann");

    // Depth is capped, so a runaway spec cannot recurse forever.
    let mut items = vec![records::find_by_id_raw(&db, &posts, post.id())
        .await
        .unwrap()];
    expand::resolve(
        &db,
        &db.collections,
        &superuser(),
        &mut items,
        "author",
        expand::MAX_DEPTH,
    )
    .await
    .unwrap();
    assert!(items[0].expand().is_empty());
}

#[tokio::test]
async fn multi_relation_expands_to_an_array() {
    let db = fixture().await;
    let mut tags = Collection::new("linked", CollectionType::Base);
    insert_fields(&mut tags, vec![text("name")]);
    db.collections.insert(&*db.engine, &tags).await.unwrap();

    let mut holder = Collection::new("holders", CollectionType::Base);
    insert_fields(
        &mut holder,
        vec![Field::new(
            "links",
            FieldKind::Relation {
                collection_id: tags.id.clone(),
                cascade_delete: false,
                min_select: 0,
                max_select: 5,
            },
        )],
    );
    db.collections.insert(&*db.engine, &holder).await.unwrap();

    let a = create(&db, "linked", json!({"name": "a"})).await;
    let b = create(&db, "linked", json!({"name": "b"})).await;
    let h = create(&db, "holders", json!({"links": [a.id(), b.id()]})).await;

    let holders = collection(&db, "holders");
    let mut items = vec![records::find_by_id_raw(&db, &holders, h.id())
        .await
        .unwrap()];
    assert_eq!(items[0].get("links"), Some(&json!([a.id(), b.id()])));
    expand::resolve(&db, &db.collections, &superuser(), &mut items, "links", 0)
        .await
        .unwrap();
    let links = items[0].expand()["links"].as_array().unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0]["name"], "a");
    assert_eq!(links[1]["name"], "b");
}

#[tokio::test]
async fn validation_codes_match_pocketbase() {
    let db = fixture().await;
    let author = create(&db, "authors", json!({"name": "Ann"})).await;

    // required
    assert_eq!(
        field_errors(try_create(&db, "posts", json!({})).await.unwrap_err())["title"],
        codes::REQUIRED
    );
    // text min / max
    assert_eq!(
        field_errors(
            try_create(&db, "posts", json!({"title": "ab"}))
                .await
                .unwrap_err()
        )["title"],
        codes::MIN_TEXT
    );
    assert_eq!(
        field_errors(
            try_create(&db, "posts", json!({"title": "x".repeat(21)}))
                .await
                .unwrap_err()
        )["title"],
        codes::MAX_TEXT
    );
    // number onlyInt / min / max
    let e = field_errors(
        try_create(&db, "posts", json!({"title": "Num", "views": 1.5}))
            .await
            .unwrap_err(),
    );
    assert_eq!(e["views"], codes::ONLY_INT);
    assert_eq!(
        field_errors(
            try_create(&db, "posts", json!({"title": "Num", "views": -1}))
                .await
                .unwrap_err()
        )["views"],
        codes::MIN_NUMBER
    );
    assert_eq!(
        field_errors(
            try_create(&db, "posts", json!({"title": "Num", "views": 1001}))
                .await
                .unwrap_err()
        )["views"],
        codes::MAX_NUMBER
    );
    // ... but a non-numeric string is coerced to 0, not rejected.
    let r = create(&db, "posts", json!({"title": "Num", "views": "abc"})).await;
    assert_eq!(r.get("views"), Some(&json!(0.0)));

    // select
    assert_eq!(
        field_errors(
            try_create(&db, "posts", json!({"title": "Sel", "category": "nope"}))
                .await
                .unwrap_err()
        )["category"],
        codes::NOT_IN_LIST
    );
    assert_eq!(
        field_errors(
            try_create(
                &db,
                "posts",
                json!({"title": "Sel", "tags": ["go", "rust", "js"]})
            )
            .await
            .unwrap_err()
        )["tags"],
        codes::MAX_SELECT
    );
    // a single-valued select given a list keeps the LAST element
    let r = create(
        &db,
        "posts",
        json!({"title": "Sel", "category": ["news", "blog"]}),
    )
    .await;
    assert_eq!(r.get("category"), Some(&json!("blog")));

    // relation existence
    assert_eq!(
        field_errors(
            try_create(
                &db,
                "posts",
                json!({"title": "Rel", "author": "doesnotexist000"})
            )
            .await
            .unwrap_err()
        )["author"],
        codes::MISSING_REL
    );
    assert!(
        try_create(&db, "posts", json!({"title": "Rel", "author": author.id()}))
            .await
            .is_ok()
    );

    // email / url
    assert_eq!(
        field_errors(
            try_create(&db, "posts", json!({"title": "Fmt", "contact": "nope"}))
                .await
                .unwrap_err()
        )["contact"],
        codes::INVALID_EMAIL
    );
    assert_eq!(
        field_errors(
            try_create(&db, "posts", json!({"title": "Fmt", "site": "not a url"}))
                .await
                .unwrap_err()
        )["site"],
        "validation_invalid_url"
    );

    // date bounds; garbage coerces to ""
    assert_eq!(
        field_errors(
            try_create(
                &db,
                "posts",
                json!({"title": "Date", "when": "2010-01-01 00:00:00.000Z"})
            )
            .await
            .unwrap_err()
        )["when"],
        "validation_min_greater_equal_than_required"
    );
    assert_eq!(
        field_errors(
            try_create(
                &db,
                "posts",
                json!({"title": "Date", "when": "2040-01-01 00:00:00.000Z"})
            )
            .await
            .unwrap_err()
        )["when"],
        "validation_max_less_equal_than_required"
    );
    let r = create(&db, "posts", json!({"title": "Date", "when": "not a date"})).await;
    assert_eq!(r.get("when"), Some(&json!("")));

    // geoPoint range
    assert_eq!(
        field_errors(
            try_create(
                &db,
                "posts",
                json!({"title": "Geo", "loc": {"lon": 999.0, "lat": 0.0}})
            )
            .await
            .unwrap_err()
        )["loc"],
        codes::INVALID_FORMAT
    );

    // several failures are reported together
    let e = field_errors(
        try_create(
            &db,
            "posts",
            json!({"title": "x", "views": 5000, "contact": "bad"}),
        )
        .await
        .unwrap_err(),
    );
    let mut keys: Vec<&String> = e.keys().collect();
    keys.sort();
    assert_eq!(keys, vec!["contact", "title", "views"]);
}

#[tokio::test]
async fn client_supplied_ids_are_validated_like_a_text_field() {
    let db = fixture().await;
    let id = "abcdefghijklmno";
    let r = create(&db, "posts", json!({"id": id, "title": "With id"})).await;
    assert_eq!(r.id(), id);

    assert_eq!(
        field_errors(
            try_create(&db, "posts", json!({"id": "TOO-SHORT", "title": "Bad"}))
                .await
                .unwrap_err()
        )["id"],
        codes::MIN_TEXT
    );
    assert_eq!(
        field_errors(
            try_create(
                &db,
                "posts",
                json!({"id": "abcdefghijklmnO", "title": "Bad"})
            )
            .await
            .unwrap_err()
        )["id"],
        codes::PATTERN_MISMATCH
    );
    // A duplicate id is `validation_not_unique`, not a raw PK violation.
    assert_eq!(
        field_errors(
            try_create(&db, "posts", json!({"id": id, "title": "Dup"}))
                .await
                .unwrap_err()
        )["id"],
        codes::NOT_UNIQUE
    );
}

#[tokio::test]
async fn unique_index_violations_name_the_right_field() {
    let db = fixture().await;
    create(&db, "posts", json!({"title": "Uniq", "slug": "same"})).await;
    let err = try_create(&db, "posts", json!({"title": "Uniq2", "slug": "same"}))
        .await
        .unwrap_err();
    assert_eq!(field_errors(err)["slug"], codes::NOT_UNIQUE);

    // The partial index (`WHERE slug != ''`) still allows many blanks.
    create(&db, "posts", json!({"title": "Blank1"})).await;
    create(&db, "posts", json!({"title": "Blank2"})).await;

    // The same mapping applies on update.
    let mut second = create(&db, "posts", json!({"title": "Other", "slug": "other"})).await;
    second.set("slug", json!("same"));
    let err = records::update(&db, &db.collections, &mut second)
        .await
        .unwrap_err();
    assert_eq!(field_errors(err)["slug"], codes::NOT_UNIQUE);
}

#[tokio::test]
async fn delete_cascades_and_strips_references_and_reports_files() {
    let db = fixture().await;
    let ann = create(&db, "authors", json!({"name": "Ann"})).await;
    let post = create(
        &db,
        "posts",
        json!({"title": "Cascade", "author": ann.id()}),
    )
    .await;
    let comment = create(
        &db,
        "comments",
        json!({"post": post.id(), "text": "x", "author": ann.id()}),
    )
    .await;
    let comments = collection(&db, "comments");
    let posts = collection(&db, "posts");
    let authors = collection(&db, "authors");

    // `comments.post` cascades: deleting the post deletes the comment.
    let files = records::delete(&db, &db.collections, &post).await.unwrap();
    assert!(files.is_empty());
    assert!(matches!(
        records::find_by_id_raw(&db, &comments, comment.id()).await,
        Err(DbError::NotFound)
    ));

    // `posts.author` does not cascade: deleting the author blanks it.
    let post = create(&db, "posts", json!({"title": "Kept", "author": ann.id()})).await;
    records::delete(&db, &db.collections, &ann).await.unwrap();
    let post = records::find_by_id_raw(&db, &posts, post.id())
        .await
        .unwrap();
    assert_eq!(post.get_string("author"), "");
    assert!(matches!(
        records::find_by_id_raw(&db, &authors, ann.id()).await,
        Err(DbError::NotFound)
    ));

    // A multi-valued relation loses just the deleted id and keeps the rest.
    let mut credits = Collection::new("credits", CollectionType::Base);
    insert_fields(
        &mut credits,
        vec![Field::new(
            "people",
            FieldKind::Relation {
                collection_id: authors.id.clone(),
                cascade_delete: false,
                min_select: 0,
                max_select: 5,
            },
        )],
    );
    db.collections.insert(&*db.engine, &credits).await.unwrap();
    let a = create(&db, "authors", json!({"name": "A"})).await;
    let b = create(&db, "authors", json!({"name": "B"})).await;
    let both = create(&db, "credits", json!({"people": [a.id(), b.id()]})).await;
    records::delete(&db, &db.collections, &a).await.unwrap();
    let credits = collection(&db, "credits");
    let both = records::find_by_id_raw(&db, &credits, both.id())
        .await
        .unwrap();
    assert_eq!(both.get("people"), Some(&json!([b.id()])));
}

#[tokio::test]
async fn deleted_records_report_their_files() {
    let db = fixture().await;
    // The file names are written directly: `crates/db` never uploads, it
    // only reports what the caller must now remove from storage.
    let posts = collection(&db, "posts");
    let mut record = Record::new(posts.clone());
    record.set("title", json!("Files"));
    record.set("cover", json!(["a.png", "b.png"]));
    records::create(&db, &db.collections, &mut record)
        .await
        .unwrap();

    let files = records::delete(&db, &db.collections, &record)
        .await
        .unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].collection_id, posts.id);
    assert_eq!(files[0].record_id, record.id());
    assert_eq!(files[0].filename, "a.png");
    assert_eq!(files[1].filename, "b.png");
}

#[tokio::test]
async fn multi_value_modifiers() {
    let db = fixture().await;
    let posts = collection(&db, "posts");
    let mut record = create(&db, "posts", json!({"title": "Mods", "tags": ["go"]})).await;

    let mut body = json!({"tags+": ["rust"]}).as_object().cloned().unwrap();
    validate::apply_modifiers(Some(&record), &mut body, &posts);
    assert_eq!(body["tags"], json!(["go", "rust"]));
    records::apply_body(&mut record, &body);
    records::update(&db, &db.collections, &mut record)
        .await
        .unwrap();
    assert_eq!(record.get("tags"), Some(&json!(["go", "rust"])));

    let mut body = json!({"tags-": ["go"]}).as_object().cloned().unwrap();
    validate::apply_modifiers(Some(&record), &mut body, &posts);
    assert_eq!(body["tags"], json!(["rust"]));

    let mut body = json!({"+tags": ["js"]}).as_object().cloned().unwrap();
    validate::apply_modifiers(Some(&record), &mut body, &posts);
    assert_eq!(body["tags"], json!(["js", "go", "rust"]));
}

#[tokio::test]
async fn auth_records_hash_passwords_once_and_generate_a_token_key() {
    let db = Db::memory().await.unwrap();
    let users = db.collections.get("users").unwrap();

    let mut user = Record::new(users.clone());
    user.set("email", json!("a@b.co"));
    user.set("password", json!("supersecret"));
    records::create(&db, &db.collections, &mut user)
        .await
        .unwrap();

    let hash = user.password_hash();
    assert!(hash.starts_with("$argon2id$"), "{hash}");
    assert!(cratebase_auth::verify_password("supersecret", &hash));
    // `tokenKey` is autogenerated from the field's pattern...
    let token_key = user.token_key();
    assert_eq!(token_key.len(), 50);
    // ... and `emailVisibility` defaults to false.
    assert!(!user.email_visibility());

    // An update that does not touch the password leaves the hash alone.
    let mut loaded = records::find_by_id_raw(&db, &users, user.id())
        .await
        .unwrap();
    assert_eq!(loaded.password_hash(), hash);
    loaded.set("name", json!("Ann"));
    records::update(&db, &db.collections, &mut loaded)
        .await
        .unwrap();
    assert_eq!(loaded.password_hash(), hash);
    assert_eq!(loaded.token_key(), token_key);

    // A new plaintext is hashed, and the new hash verifies.
    loaded.set("password", json!("evenbetterpw"));
    records::update(&db, &db.collections, &mut loaded)
        .await
        .unwrap();
    let rehashed = loaded.password_hash();
    assert_ne!(rehashed, hash);
    assert!(cratebase_auth::verify_password("evenbetterpw", &rehashed));

    // A too-short password is rejected against the plaintext, not the hash.
    let mut short = Record::new(users);
    short.set("email", json!("c@d.co"));
    short.set("password", json!("tiny"));
    let err = records::create(&db, &db.collections, &mut short)
        .await
        .unwrap_err();
    assert_eq!(field_errors(err)["password"], codes::MIN_TEXT);
}

#[tokio::test]
async fn rule_free_lookups_and_filter_params() {
    let db = fixture().await;
    let posts = collection(&db, "posts");
    let a = create(&db, "posts", json!({"title": "Alpha", "views": 5})).await;
    let b = create(&db, "posts", json!({"title": "Beta", "views": 50})).await;

    let found = records::find_by_ids(&db, &posts, &[a.id().into(), b.id().into()])
        .await
        .unwrap();
    assert_eq!(found.len(), 2);
    assert!(records::find_by_ids(&db, &posts, &[])
        .await
        .unwrap()
        .is_empty());
    assert_eq!(records::count(&db, &posts).await.unwrap(), 2);

    let mut params = Map::new();
    params.insert("t".into(), json!("Beta"));
    let hit = records::find_first_by_filter(&db, &db.collections, &posts, "title = {:t}", &params)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hit.id(), b.id());

    let miss =
        records::find_first_by_filter(&db, &db.collections, &posts, "views > 9999", &Map::new())
            .await
            .unwrap();
    assert!(miss.is_none());
}

#[tokio::test]
async fn views_are_read_only_and_geo_distance_filters_run() {
    let db = fixture().await;
    create(
        &db,
        "posts",
        json!({"title": "Near", "loc": {"lon": 0.0, "lat": 0.0}}),
    )
    .await;
    create(
        &db,
        "posts",
        json!({"title": "Far", "loc": {"lon": 40.0, "lat": 40.0}}),
    )
    .await;
    let posts = collection(&db, "posts");

    // The SQLite engine registers `geoDistance`, so the filter compiles
    // and runs end to end.
    let page = records::list(
        &db,
        &db.collections,
        &superuser(),
        &posts,
        ListParams {
            page: 1,
            per_page: 30,
            filter: Some("geoDistance(loc.lon, loc.lat, 0, 0) < 100"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.total_items, 1);
    assert_eq!(page.items[0].get_string("title"), "Near");

    // A view collection rejects writes.
    let mut view = Collection::new("titles", CollectionType::View);
    view.view_query = "SELECT id, title FROM posts".into();
    db.collections.insert(&*db.engine, &view).await.unwrap();
    let view = collection(&db, "titles");
    let mut record = Record::new(view);
    let err = records::create(&db, &db.collections, &mut record)
        .await
        .unwrap_err();
    assert!(matches!(err, DbError::Unsupported(_)), "{err:?}");
}
