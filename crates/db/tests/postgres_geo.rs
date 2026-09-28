//! PostGIS-accelerated geo queries on Postgres: the GiST expression index
//! (`crates/server/src/geo.rs` builds it in the real server; this test
//! builds the textually-identical DDL directly, since `crates/db` has no
//! dependency on the server crate) is actually used for a radius filter,
//! and the accelerated (`ST_DWithin`) result set agrees with the portable
//! haversine path.
//!
//! Skips (rather than fails) when `TEST_POSTGRES_URL` is unset or
//! `postgis` can't be installed, matching this crate's other
//! Postgres-only tests' convention.

use std::sync::Arc;

use cratebase_core::{Collection, CollectionType, Field, FieldKind};
use cratebase_db::context::{geo_index_expr, CollectionResolver, RequestContext};
use cratebase_db::engine::{quote_ident, Sql};
use cratebase_db::{Backend, CollectionStore, Db, Executor};
use cratebase_filter::Dialect;

static ONE_TEST_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// San Francisco-ish query point.
const QUERY_LON: f64 = -122.4194;
const QUERY_LAT: f64 = 37.7749;
const RADIUS_KM: f64 = 5.0;

async fn setup(url: &str) -> Option<Db> {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::connect(url, &dir.path().to_string_lossy())
        .await
        .unwrap();
    assert_eq!(db.backend, Backend::Postgres);
    db.execute("DROP SCHEMA public CASCADE", &[]).await.unwrap();
    db.execute("CREATE SCHEMA public", &[]).await.unwrap();
    db.bootstrap().await.unwrap();

    if db
        .execute("CREATE EXTENSION IF NOT EXISTS postgis", &[])
        .await
        .is_err()
    {
        return None;
    }

    let mut stores = Collection::new("stores", CollectionType::Base);
    let pos = stores.fields.len() - 2;
    stores
        .fields
        .insert(pos, Field::new("loc", FieldKind::GeoPoint {}));
    db.collections.insert(&*db.engine, &stores).await.unwrap();

    // The exact expression `CollectionResolver::postgis_geo_index` compiles
    // a query against — see that method's doc for why it has to match.
    let col = format!("{}.{}", quote_ident("stores"), quote_ident("loc"));
    let expr = geo_index_expr(&col);
    db.execute(
        &format!(
            "CREATE INDEX IF NOT EXISTS \"idx_stores_loc_geog\" ON \"stores\" USING GIST (({expr}))"
        ),
        &[],
    )
    .await
    .unwrap();

    // Bulk-insert enough scattered rows that the planner has a real
    // reason to prefer the index over a sequential scan, plus a handful
    // of known-near/known-far points for the correctness check.
    db.execute(
        r#"INSERT INTO "stores" ("id", "loc", "created", "updated")
           SELECT
               'bulk_' || gs::text,
               json_build_object(
                   'lon', (random() * 360 - 180),
                   'lat', (random() * 180 - 90)
               )::text,
               '', ''
           FROM generate_series(1, 50000) AS gs"#,
        &[],
    )
    .await
    .unwrap();

    // Well inside 5km of the query point.
    for (id, lon, lat) in [
        ("near_1", -122.4184, 37.7755),
        ("near_2", -122.4150, 37.7700),
        ("near_3", -122.4230, 37.7800),
    ] {
        insert_point(&db, id, lon, lat).await;
    }
    // Well outside 5km (Oakland-ish and further).
    for (id, lon, lat) in [
        ("far_1", -122.2712, 37.8044), // Oakland, ~13km away
        ("far_2", -73.9857, 40.7484),  // New York
    ] {
        insert_point(&db, id, lon, lat).await;
    }

    db.execute("ANALYZE \"stores\"", &[]).await.unwrap();
    Some(db)
}

async fn insert_point(db: &Db, id: &str, lon: f64, lat: f64) {
    db.execute(
        r#"INSERT INTO "stores" ("id", "loc", "created", "updated") VALUES ($1, $2, '', '')"#,
        &[
            Sql::from(id),
            Sql::from(format!("{{\"lon\":{lon},\"lat\":{lat}}}")),
        ],
    )
    .await
    .unwrap();
}

fn resolver<'a>(
    stores: &Arc<Collection>,
    store: &'a CollectionStore,
    ctx: &'a RequestContext,
) -> CollectionResolver<'a> {
    CollectionResolver::new(stores.clone(), store, ctx, Dialect::Postgres)
}

/// Run the compiled filter's `SELECT` through the real query builder
/// (mirroring `cratebase_db::records::list`) and return the matching ids,
/// sorted for a stable comparison.
async fn matching_ids(
    db: &Db,
    stores: &Arc<Collection>,
    ctx: &RequestContext,
    filter: &str,
) -> Vec<String> {
    let r = resolver(stores, &db.collections, ctx);
    let compiled = cratebase_filter::parse_and_compile(filter, &r, 0).unwrap();
    let mut query = cratebase_db::query::Query::new(stores);
    query.push_filter(compiled);
    let sql = query.select_sql();
    query.bind_page(1_000_000, 0);
    let rows = db.query(&sql, query.params()).await.unwrap();
    let mut ids: Vec<String> = rows
        .iter()
        .map(|row| row.get_str("id").unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn postgis_radius_filter_matches_haversine_and_uses_the_index() {
    let _guard = ONE_TEST_AT_A_TIME.lock().await;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping postgis_radius_filter_matches_haversine_and_uses_the_index: \
             TEST_POSTGRES_URL not set"
        );
        return;
    };
    let Some(db) = setup(&url).await else {
        eprintln!(
            "skipping postgis_radius_filter_matches_haversine_and_uses_the_index: \
             postgis not installable"
        );
        return;
    };
    let stores = db.collections.get("stores").unwrap();
    let filter = format!("geoDistance(loc.lon, loc.lat, {QUERY_LON}, {QUERY_LAT}) < {RADIUS_KM}");

    let accelerated_ctx = RequestContext {
        superuser: true,
        postgis_available: true,
        ..Default::default()
    };
    let haversine_ctx = RequestContext {
        superuser: true,
        postgis_available: false,
        ..Default::default()
    };

    let accelerated = matching_ids(&db, &stores, &accelerated_ctx, &filter).await;
    let haversine = matching_ids(&db, &stores, &haversine_ctx, &filter).await;

    // Both paths agree on the exact same set of rows (well clear of the
    // radius boundary, so haversine-vs-geography rounding can't matter).
    assert_eq!(
        accelerated, haversine,
        "accelerated and haversine paths must agree"
    );
    assert_eq!(
        accelerated,
        vec!["near_1", "near_2", "near_3"],
        "expected only the three near_* points within {RADIUS_KM}km"
    );

    // The accelerated path's own compiled SQL actually is `ST_DWithin`,
    // not just an answer that happens to match.
    let r = resolver(&stores, &db.collections, &accelerated_ctx);
    let compiled = cratebase_filter::parse_and_compile(&filter, &r, 0).unwrap();
    assert!(compiled.sql.starts_with("ST_DWithin("), "{}", compiled.sql);

    // EXPLAIN proves the planner actually uses the GiST index against a
    // table with real rows in it, not just that the query returns the
    // right answer.
    let mut query = cratebase_db::query::Query::new(&stores);
    query.push_filter(compiled);
    let explain_sql = format!("EXPLAIN {}", query.select_sql());
    query.bind_page(1_000_000, 0);
    let rows = db.query(&explain_sql, query.params()).await.unwrap();
    let plan: String = rows
        .iter()
        .filter_map(|r| r.get_str("QUERY PLAN"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains("idx_stores_loc_geog") || plan.to_lowercase().contains("index"),
        "expected the plan to use the GiST index, got:\n{plan}"
    );
    assert!(
        !plan.contains("Seq Scan"),
        "expected an index scan, not a sequential scan, got:\n{plan}"
    );

    db.close().await.unwrap();
}

#[tokio::test]
async fn postgis_nearest_sort_matches_haversine_order() {
    let _guard = ONE_TEST_AT_A_TIME.lock().await;
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping postgis_nearest_sort_matches_haversine_order: TEST_POSTGRES_URL not set"
        );
        return;
    };
    let Some(db) = setup(&url).await else {
        eprintln!("skipping postgis_nearest_sort_matches_haversine_order: postgis not installable");
        return;
    };
    let stores = db.collections.get("stores").unwrap();
    let sort_expr = format!("geoDistance(loc.lon, loc.lat, {QUERY_LON}, {QUERY_LAT})");
    // Only compare among the five hand-placed points — comparing full
    // KNN vs. full haversine order over 50,000 random rows would be
    // exact but needlessly slow; the near_*/far_* set already exercises
    // both near and far distances.

    async fn sorted_hand_placed_ids(
        db: &Db,
        stores: &Arc<Collection>,
        ctx: &RequestContext,
        sort_expr: &str,
    ) -> Vec<String> {
        let r = resolver(stores, &db.collections, ctx);
        let (order_sql, order_params) =
            cratebase_db::query::order_by(&r, Some(sort_expr), 0).unwrap();
        let sql = format!(
            "SELECT \"id\" FROM \"stores\" WHERE \"id\" IN \
             ('near_1','near_2','near_3','far_1','far_2') ORDER BY {order_sql}"
        );
        let rows = db.query(&sql, &order_params).await.unwrap();
        rows.iter()
            .map(|r| r.get_str("id").unwrap().to_string())
            .collect()
    }

    let accelerated_ctx = RequestContext {
        superuser: true,
        postgis_available: true,
        ..Default::default()
    };
    let haversine_ctx = RequestContext {
        superuser: true,
        postgis_available: false,
        ..Default::default()
    };

    let accelerated = sorted_hand_placed_ids(&db, &stores, &accelerated_ctx, &sort_expr).await;
    let haversine = sorted_hand_placed_ids(&db, &stores, &haversine_ctx, &sort_expr).await;
    assert_eq!(
        accelerated, haversine,
        "KNN and haversine sorts must agree on order"
    );
    // The exact order among near_1/near_3/near_2 depends on their precise
    // coordinates (not hand-verified here); what matters is that both
    // compilation paths agree with each other (asserted above) and that
    // all three near_* points sort before both far_* points.
    assert_eq!(
        accelerated[3..],
        ["far_1".to_string(), "far_2".to_string()],
        "the far_* points must still sort last: {accelerated:?}"
    );
    assert!(
        accelerated[..3].iter().all(|id| id.starts_with("near_")),
        "the first three must be the near_* points: {accelerated:?}"
    );

    db.close().await.unwrap();
}
