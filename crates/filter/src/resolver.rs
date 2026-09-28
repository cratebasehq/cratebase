//! The contract between the filter compiler and its host: the schema it
//! compiles against and the per-request values (`@request.*`).

use std::sync::Arc;

use cratebase_core::Collection;
use serde_json::Value;

/// Target SQL dialect. Only the two backends Cratebase supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

/// A `@request.*` reference, already split into its namespace and the
/// remaining dotted path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestPath {
    /// `@request.auth` (`None`) or `@request.auth.<path>`.
    Auth(Option<String>),
    /// `@request.body.<path>` (also reachable as the deprecated
    /// `@request.data.<path>`).
    Body(String),
    /// `@request.query.<path>`.
    Query(String),
    /// `@request.headers.<name>`; the name is passed as written in the
    /// rule (PocketBase convention: lowercase, `-` replaced by `_`).
    Headers(String),
    /// `@request.method`.
    Method,
    /// `@request.context`.
    Context,
}

impl RequestPath {
    /// Parse the segments after `@request.`. Returns `None` for unknown
    /// namespaces.
    pub fn parse(segments: &[&str]) -> Option<RequestPath> {
        let (ns, rest) = segments.split_first()?;
        let rest = rest.join(".");
        match *ns {
            "auth" => Some(RequestPath::Auth(if rest.is_empty() {
                None
            } else {
                Some(rest)
            })),
            "body" | "data" if !rest.is_empty() => Some(RequestPath::Body(rest)),
            "query" if !rest.is_empty() => Some(RequestPath::Query(rest)),
            "headers" if !rest.is_empty() => Some(RequestPath::Headers(rest)),
            "method" if rest.is_empty() => Some(RequestPath::Method),
            "context" if rest.is_empty() => Some(RequestPath::Context),
            _ => None,
        }
    }
}

/// Everything the compiler needs from its host. Implemented by the
/// records layer with the process-wide collection index and the current
/// request context.
pub trait Resolver: Send + Sync {
    /// The collection the filter is evaluated against.
    fn root(&self) -> &Collection;

    /// Lookup another collection by name or id (for `@collection.X`,
    /// relation targets and back-relations).
    fn collection(&self, name_or_id: &str) -> Option<Arc<Collection>>;

    /// Value of a `@request.*` path. Missing values are `Value::Null`
    /// (which the compiler treats like PocketBase's empty value).
    fn request_value(&self, path: &RequestPath) -> Value;

    /// Whether a body key is present (for `@request.body.x:isset`).
    fn body_has(&self, key: &str) -> bool;

    fn dialect(&self) -> Dialect;

    /// Current time for the date macros; overridable for deterministic
    /// tests.
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }

    /// The `geography` SQL expression to compare/order by for a
    /// root-level `geoPoint` field named `field`, when — and only when —
    /// doing so would actually be faster: `dialect() == Dialect::Postgres`,
    /// the `postgis` extension is installed, `field` really is a
    /// `geoPoint` field on [`root`](Resolver::root), and a matching GiST
    /// expression index exists (see `crates/server/src/geo.rs`, which
    /// creates it idempotently on collection sync). `None` — the default,
    /// and every existing [`Resolver`] gets it for free — means "compile
    /// `geoDistance(...)` the portable way", the haversine calculation
    /// [`crate::compiler`]'s module doc describes; hosts that can answer
    /// this (`cratebase_db::context::CollectionResolver`) override it so
    /// a `geoDistance(field.lon, field.lat, x, y) < r` radius filter
    /// compiles to `ST_DWithin` and a `sort=geoDistance(...)` to a `<->`
    /// KNN order instead — both index-backed, and, for the filter case,
    /// only when the comparison is `<`/`<=` (the shape an index can
    /// actually help with).
    fn postgis_geo_index(&self, _field: &str) -> Option<String> {
        None
    }
}
