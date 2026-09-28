//! SQLite side of the full-text search evidence the Postgres suite
//! (`crates/db/tests/postgres_search.rs`) already has: an `EXPLAIN QUERY
//! PLAN` showing the FTS5 shadow table is actually used, and a
//! micro-benchmark comparing `search()` against a `~` (`LIKE`) scan over
//! 50k rows. Runs against an in-memory SQLite database — no
//! `TEST_POSTGRES_URL` or external service needed.

use std::sync::Arc;
use std::time::Instant;

use cratebase_core::{Collection, CollectionType, Field, FieldKind};
use cratebase_db::context::{CollectionResolver, RequestContext};
use cratebase_db::{Backend, CollectionStore, Db, Executor};
use cratebase_filter::Dialect;

fn searchable_posts() -> Collection {
    let mut posts = Collection::new("posts", CollectionType::Base);
    let pos = posts.fields.len() - 2;
    let mut title = Field::new(
        "title",
        FieldKind::Text {
            min: 0,
            max: 0,
            pattern: String::new(),
            autogenerate_pattern: String::new(),
            primary_key: false,
        },
    );
    title.searchable = true;
    posts.fields.splice(pos..pos, [title]);
    posts
}

async fn setup() -> Db {
    let db = Db::memory().await.unwrap();
    assert_eq!(db.backend, Backend::Sqlite);
    let posts = searchable_posts();
    db.collections.insert(&*db.engine, &posts).await.unwrap();
    db
}

fn resolver<'a>(
    posts: &Arc<Collection>,
    store: &'a CollectionStore,
    ctx: &'a RequestContext,
) -> CollectionResolver<'a> {
    CollectionResolver::new(posts.clone(), store, ctx, Dialect::Sqlite)
}

fn superuser() -> RequestContext {
    RequestContext {
        superuser: true,
        ..Default::default()
    }
}

/// The generated `posts_fts` FTS5 shadow table exists (`schema::sync_*`
/// creates it alongside the base table's triggers), and `EXPLAIN QUERY
/// PLAN` for a `search()` predicate actually names it — proving SQLite's
/// planner reaches into the FTS5 index rather than a full scan of `posts`
/// itself, the SQLite analogue of the GIN-index evidence in
/// `postgres_search.rs::tsvector_column_and_gin_index_are_created_and_used_by_the_planner`.
#[tokio::test]
async fn fts5_shadow_table_is_created_and_used_by_the_query_planner() {
    let db = setup().await;

    let tables = db
        .query(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'posts_fts'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(tables.len(), 1, "expected the posts_fts FTS5 shadow table");

    let posts = db.collections.get("posts").unwrap();
    let ctx = superuser();
    let r = resolver(&posts, &db.collections, &ctx);

    let compiled = cratebase_db::query::search_condition(&r, "treasure", 0).unwrap();
    assert!(
        compiled.sql.contains("posts_fts") && compiled.sql.contains("MATCH"),
        "{}",
        compiled.sql
    );
    let mut query = cratebase_db::query::Query::new(&posts);
    query.push_filter(compiled);
    let sql = format!("EXPLAIN QUERY PLAN {}", query.select_sql());
    query.bind_page(1_000_000, 0);
    let rows = db.query(&sql, query.params()).await.unwrap();
    let plan: String = rows
        .iter()
        .filter_map(|row| row.get_str("detail"))
        .collect::<Vec<_>>()
        .join("\n");
    eprintln!("fts5_shadow_table_is_created_and_used_by_the_query_planner: plan:\n{plan}");
    assert!(
        plan.to_lowercase().contains("posts_fts"),
        "expected the query plan to reference the posts_fts virtual table, got:\n{plan}"
    );
}

/// Micro-benchmark: `search()` (FTS5-backed) vs a `~` (`LIKE`) scan over
/// 50k rows, same query. Numbers are printed with `eprintln!` (run with
/// `--nocapture` to see them) rather than asserted on, since absolute
/// timings vary by machine — mirrors
/// `postgres_search.rs::search_vs_like_micro_benchmark_50k_rows`.
#[tokio::test]
async fn search_vs_like_micro_benchmark_50k_rows() {
    let db = setup().await;

    db.execute("BEGIN", &[]).await.unwrap();
    for i in 0..50_000u32 {
        let title = format!(
            "{} number {i}",
            ["a quiet afternoon", "weekly news roundup", "cooking with garlic", "notes on rust programming", "traveling through spain"]
                [(i % 5) as usize]
        );
        db.execute(
            "INSERT INTO \"posts\" (\"id\", \"title\", \"created\", \"updated\") VALUES ($1, $2, '', '')",
            &[
                cratebase_db::engine::Sql::from(format!("bulk_{i}")),
                cratebase_db::engine::Sql::from(title),
            ],
        )
        .await
        .unwrap();
    }
    for id in ["needle_1", "needle_2", "needle_3"] {
        db.execute(
            "INSERT INTO \"posts\" (\"id\", \"title\", \"created\", \"updated\") VALUES ($1, 'an article about xenolithography techniques', '', '')",
            &[cratebase_db::engine::Sql::from(id)],
        )
        .await
        .unwrap();
    }
    db.execute("COMMIT", &[]).await.unwrap();
    db.execute("ANALYZE", &[]).await.unwrap();

    let posts = db.collections.get("posts").unwrap();
    let ctx = superuser();

    // Each query is run `RUNS` times and the *minimum* is reported (the
    // standard micro-benchmark convention: the minimum is the closest
    // measurement to the query's true cost, everything above it is some
    // amount of scheduler/cache/allocator noise) — a single untimed
    // sample each is not enough to fairly compare the two since
    // whichever runs first also pays for warming SQLite's page cache.
    const RUNS: u32 = 5;

    let mut search_best = std::time::Duration::MAX;
    let mut search_rows = 0;
    for _ in 0..RUNS {
        let r = resolver(&posts, &db.collections, &ctx);
        let search_compiled = cratebase_db::query::search_condition(&r, "xenolithography", 0).unwrap();
        let mut q1 = cratebase_db::query::Query::new(&posts);
        q1.push_filter(search_compiled);
        let sql1 = q1.select_sql();
        q1.bind_page(1_000_000, 0);
        let started = Instant::now();
        let rows1 = db.query(&sql1, q1.params()).await.unwrap();
        search_best = search_best.min(started.elapsed());
        search_rows = rows1.len();
    }
    assert_eq!(search_rows, 3);

    // `~` — a portable case-insensitive LIKE scan, PocketBase/Cratebase's
    // pre-existing substring operator, over the same column.
    let mut like_best = std::time::Duration::MAX;
    let mut like_rows = 0;
    for _ in 0..RUNS {
        let r = resolver(&posts, &db.collections, &ctx);
        let like_filter = "title ~ \"xenolithography\"";
        let like_compiled = cratebase_filter::parse_and_compile(like_filter, &r, 0).unwrap();
        let mut q2 = cratebase_db::query::Query::new(&posts);
        q2.push_filter(like_compiled);
        let sql2 = q2.select_sql();
        q2.bind_page(1_000_000, 0);
        let started = Instant::now();
        let rows2 = db.query(&sql2, q2.params()).await.unwrap();
        like_best = like_best.min(started.elapsed());
        like_rows = rows2.len();
    }
    assert_eq!(like_rows, 3);

    eprintln!(
        "search_vs_like_micro_benchmark_50k_rows (sqlite, best of {RUNS}): search()={:?} ~ (LIKE)={:?} \
         ({:.1}x)",
        search_best,
        like_best,
        like_best.as_secs_f64() / search_best.as_secs_f64().max(1e-9)
    );
}
