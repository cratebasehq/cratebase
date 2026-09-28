//! PostGIS-accelerated geo indexes: an idempotent `CREATE INDEX ... USING
//! GIST` for every `geoPoint` field, kept in sync with collection schema
//! changes and with whether `postgis` is installed.
//!
//! The index is on `ST_MakePoint(lon, lat)::geography` — see
//! `cratebase_db::context::geo_index_expr`, the single source of truth
//! both this module (building the index) and `CollectionResolver::
//! postgis_geo_index` (building a query against it) use, so the two
//! expressions are always textually identical and Postgres actually
//! recognizes the index as usable.
//!
//! # Why best-effort, not fatal
//!
//! A missing index only costs *performance* — `crates/filter`'s
//! acceleration check falls back to a plain haversine scan whenever
//! `Resolver::postgis_geo_index` returns `None`, which it does for a
//! `geoPoint` field with no matching index just as readily as for one
//! with no PostGIS at all. So a `geoPoint` field created before
//! `postgis` was installed (`CREATE INDEX` fails with `ST_MakePoint`
//! undefined) never fails the collection write itself; it's logged and
//! [`sync_all`] retries every collection once `postgis` actually gets
//! installed (see `routes::extensions::install_extension`'s call to it).

use cratebase_core::{Collection, FieldType};
use cratebase_db::context::geo_index_expr;
use cratebase_db::engine::{quote_ident, Executor};
use cratebase_filter::Dialect;

use crate::app::App;
use crate::events::CollectionEvent;
use crate::hooks::{Event, Handler};

/// `idx_<table>_<field>_geog` — deterministic, so `CREATE INDEX IF NOT
/// EXISTS` is genuinely idempotent across repeated syncs. A field/
/// collection rename leaves the old-named index behind rather than
/// renaming it in place; that's a stale, harmless index rather than a
/// wrong one, and cleaning it up is the same "index the schema sync
/// doesn't know how to retarget" gap `crates/db/src/schema.rs`'s own
/// `indexes[]` handling already lives with.
fn index_name(collection: &Collection, field: &str) -> String {
    format!("idx_{}_{}_geog", collection.table_name(), field)
}

/// Idempotently (re)create the GiST expression index every `geoPoint`
/// field on `collection` needs for PostGIS acceleration. A no-op on
/// SQLite. See the module doc for why a failure here (most likely:
/// `postgis` isn't installed yet) is only logged.
pub(crate) async fn sync_collection(ex: &dyn Executor, collection: &Collection) {
    if ex.dialect() != Dialect::Postgres {
        return;
    }
    for field in collection.fields_of_type(FieldType::GeoPoint) {
        let col = format!(
            "{}.{}",
            quote_ident(collection.table_name()),
            quote_ident(&field.name)
        );
        let expr = geo_index_expr(&col);
        let name = index_name(collection, &field.name);
        let sql = format!(
            "CREATE INDEX IF NOT EXISTS {} ON {} USING GIST (({expr}))",
            quote_ident(&name),
            quote_ident(collection.table_name()),
        );
        if let Err(e) = ex.execute(&sql, &[]).await {
            tracing::warn!(
                error = %e,
                collection = %collection.name,
                field = %field.name,
                "failed to (re)create the PostGIS geo index for a geoPoint field; \
                 falling back to the portable haversine path until this succeeds"
            );
        }
    }
}

/// Re-sync every collection's geo indexes. Called once at boot (after
/// `App::postgis_available` is known — see `App::bootstrap`) and again
/// right after a superuser installs `postgis` through
/// `routes::extensions::install_extension`, since a `geoPoint` field
/// created before the extension existed would otherwise never get its
/// index until its collection's next schema change.
pub async fn sync_all(app: &App) {
    if !app.postgis_available() {
        return;
    }
    let snapshot = app.db().collections.all();
    for collection in snapshot.all.iter() {
        sync_collection(app.db(), collection).await;
    }
}

/// Bind the reactive hooks that keep a collection's geo index current as
/// its schema changes. Called once from [`App::bootstrap`]. Unlike most
/// of this codebase's `_`-tagged hooks, this one is deliberately bound
/// to *every* collection (no `.with_tags(...)`) — any collection might
/// gain or lose a `geoPoint` field.
pub fn bind_hooks(app: &App) {
    app.hooks()
        .on_collection_after_create_success
        .bind(Handler::new(|e: &mut CollectionEvent| {
            Box::pin(async move {
                sync_collection(&e.app, &e.collection).await;
                e.next().await
            })
        }));
    app.hooks()
        .on_collection_after_update_success
        .bind(Handler::new(|e: &mut CollectionEvent| {
            Box::pin(async move {
                sync_collection(&e.app, &e.collection).await;
                e.next().await
            })
        }));
}
