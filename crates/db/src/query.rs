//! Assembly of the record `SELECT`: the joins a compiled filter needs,
//! the `WHERE` made of the API rule AND the user filter, the `ORDER BY`,
//! and the `LIMIT`/`OFFSET` placeholders.
//!
//! The filter compiler hands back a boolean SQL fragment plus the
//! [`Join`]s it depends on; this module is what turns those into a
//! runnable statement. Joins are deduplicated by [`Join::key`] and force
//! `SELECT DISTINCT`, because a `LEFT JOIN` onto a back-relation
//! multiplies the driving row (PocketBase does exactly the same).

use cratebase_core::{codes, Collection, FieldError};
use cratebase_filter::{CompiledFilter, Dialect, FilterError, Join};
use serde_json::Value;

use crate::context::CollectionResolver;
use crate::engine::{quote_ident, Sql};
use crate::error::{DbError, DbResult};

/// Bind a filter parameter. The compiler emits JSON scalars; booleans
/// become `0`/`1` because that is how bool columns are stored, and
/// structured values travel as their JSON text (the compiler already
/// stringifies those, this is belt and braces).
pub fn to_param(value: &Value) -> Sql {
    match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Int(i64::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => Sql::Int(i),
            None => Sql::Real(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => Sql::Text(s.clone()),
        other => Sql::Text(other.to_string()),
    }
}

/// A record query under construction.
pub struct Query {
    table: String,
    joins: Vec<Join>,
    conditions: Vec<String>,
    params: Vec<Sql>,
    order_by: String,
}

impl Query {
    pub fn new(collection: &Collection) -> Self {
        Query {
            table: collection.table_name().to_string(),
            joins: Vec::new(),
            conditions: Vec::new(),
            params: Vec::new(),
            order_by: String::new(),
        }
    }

    /// Add a compiled filter fragment. Parameters are appended in
    /// placeholder order, so filters must be added in the same order they
    /// were compiled with their `param_offset`s.
    pub fn push_filter(&mut self, filter: CompiledFilter) {
        for join in filter.joins {
            if !self.joins.iter().any(|j| j.key == join.key) {
                self.joins.push(join);
            }
        }
        self.params.extend(filter.params.iter().map(to_param));
        self.conditions.push(filter.sql);
    }

    /// Add a raw condition with its own already-bound parameters.
    pub fn push_condition(&mut self, sql: impl Into<String>, params: Vec<Sql>) {
        self.conditions.push(sql.into());
        self.params.extend(params);
    }

    pub fn set_order_by(&mut self, order_by: String) {
        self.order_by = order_by;
    }

    /// Append parameters an `ORDER BY` clause already bound (a
    /// `sort=geoDistance(...)` token — see [`order_by`]) — same convention
    /// as [`push_filter`](Query::push_filter), just for a caller that
    /// already has the SQL string and only needs the params appended.
    pub fn push_order_params(&mut self, params: Vec<Sql>) {
        self.params.extend(params);
    }

    /// Bind the `LIMIT`/`OFFSET` values [`Query::select_sql`] emitted
    /// placeholders for. Call it *after* rendering the SQL (and after
    /// [`Query::count_sql`], which must not see them).
    pub fn bind_page(&mut self, limit: i64, offset: i64) {
        self.params.push(Sql::Int(limit));
        self.params.push(Sql::Int(offset));
    }

    pub fn params(&self) -> &[Sql] {
        &self.params
    }

    /// Next free `$n` index (1-based) for a caller appending its own
    /// placeholders, e.g. `LIMIT`/`OFFSET`.
    pub fn next_placeholder(&self) -> usize {
        self.params.len() + 1
    }

    fn qualified_id(&self) -> String {
        format!("{}.\"id\"", quote_ident(&self.table))
    }

    fn source_sql(&self) -> String {
        let mut sql = format!("FROM {}", quote_ident(&self.table));
        for join in &self.joins {
            sql.push(' ');
            sql.push_str(&join.sql);
        }
        sql
    }

    fn where_clause(&self) -> String {
        if self.conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.conditions.join(" AND "))
        }
    }

    /// `SELECT [DISTINCT] "table".* FROM ... WHERE ... ORDER BY ...
    /// LIMIT $n OFFSET $n+1`. The caller pushes the two bound values in
    /// that order after [`Query::params`].
    pub fn select_sql(&self) -> String {
        let distinct = if self.joins.is_empty() {
            ""
        } else {
            "DISTINCT "
        };
        let table = quote_ident(&self.table);
        let mut sql = format!(
            "SELECT {distinct}{table}.* {}{}",
            self.source_sql(),
            self.where_clause()
        );
        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(&self.order_by);
        }
        let n = self.next_placeholder();
        sql.push_str(&format!(" LIMIT ${} OFFSET ${}", n, n + 1));
        sql
    }

    /// The matching `COUNT`. With joins present it must count distinct
    /// driving rows, or a back-relation join would inflate the total.
    pub fn count_sql(&self) -> String {
        let counted = if self.joins.is_empty() {
            "COUNT(*)".to_string()
        } else {
            format!("COUNT(DISTINCT {})", self.qualified_id())
        };
        format!(
            "SELECT {counted} {}{}",
            self.source_sql(),
            self.where_clause()
        )
    }
}

/// An unusable `sort` is a plain `400` with an empty `data` object, not
/// a per-field validation error: PocketBase answers
/// `?sort=nope` with `{"status":400,"message":"Something went wrong
/// while processing your request.","data":{}}`
/// (`tests/conformance/records.test.ts`). Routing it through
/// [`DbError::Filter`] keeps `data` empty; the server layer supplies the
/// generic message.
fn invalid_sort(path: &str) -> DbError {
    DbError::Filter(FilterError::UnknownField(path.to_string()))
}

/// Compile PocketBase's `sort` parameter into an `ORDER BY` list, plus any
/// parameters it bound (only ever non-empty for a `geoDistance(...)` sort
/// call, the one `sort` construct with literal arguments of its own — see
/// below). `param_offset` is where those parameters start (1-based `$n`),
/// same convention as [`cratebase_filter::compile`]'s: pass
/// `query.params().len()` so they land right after whatever the filter
/// already bound, then [`Query::push_order_params`] the returned values in
/// the same call.
///
/// * `-field` sorts descending, a bare `field` ascending.
/// * `@random` orders randomly, `@rowid` by physical row order.
/// * `geoDistance(...)`/`-geoDistance(...)` sorts nearest/farthest first,
///   compiled by [`cratebase_filter::compile_sort_function`] — the exact
///   same SQL a `geoDistance(...) < r` filter over the same call would
///   produce, so a "nearest to X,Y" sort and a "within r of X,Y" filter
///   agree on distance.
/// * anything else goes through [`cratebase_filter::resolve_sort_path`],
///   so `author.name` and `data.key` work exactly as in filters.
///
/// An unresolvable path is a validation error on the `sort` key rather
/// than a silently ignored token, so a typo surfaces immediately.
pub fn order_by(
    resolver: &CollectionResolver<'_>,
    sort: Option<&str>,
    param_offset: usize,
) -> DbResult<(String, Vec<Sql>)> {
    let Some(sort) = sort.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok((default_order_by(resolver), Vec::new()));
    };
    let mut parts = Vec::new();
    let mut params = Vec::new();
    for token in split_sort_tokens(sort) {
        // `+field` is the explicit form of the default ascending order.
        let (path, direction) = match token.strip_prefix('-') {
            Some(rest) => (rest.trim(), "DESC"),
            None => (token.strip_prefix('+').unwrap_or(token).trim(), "ASC"),
        };
        if path.is_empty() {
            return Err(invalid_sort(token));
        }
        parts.push(match path {
            // PocketBase ignores the direction of `@random`.
            "@random" => "RANDOM()".to_string(),
            "@rowid" => format!("{} {direction}", rowid_expr(resolver)),
            _ if path.contains('(') => {
                let compiled = cratebase_filter::compile_sort_function(
                    path,
                    resolver,
                    param_offset + params.len(),
                )?;
                params.extend(compiled.params.iter().map(to_param));
                format!("{} {direction}", compiled.sql)
            }
            _ => {
                let sql = cratebase_filter::resolve_sort_path(resolver, path)?;
                format!("{sql} {direction}")
            }
        });
    }
    if parts.is_empty() {
        return Ok((default_order_by(resolver), Vec::new()));
    }
    Ok((parts.join(", "), params))
}

/// Split `sort` on commas, except commas nested inside a `geoDistance(...)`
/// call's own argument list: a bare `sort.split(',')` would otherwise cut
/// `geoDistance(loc.lon, loc.lat, 1, 2)` into five bogus tokens instead of
/// one. Depth tracking is enough here because a sort call's arguments are
/// never themselves parenthesized (see `ast::FUNCTIONS`'s "no nested calls"
/// rule, enforced by the parser [`compile_sort_function`] hands this token
/// to).
fn split_sort_tokens(sort: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    for (i, c) in sort.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth <= 0 => {
                tokens.push(sort[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    tokens.push(sort[start..].trim());
    tokens.into_iter().filter(|t| !t.is_empty()).collect()
}

/// Newest first, the order the dashboard and most clients expect. Skipped
/// entirely for a collection without a `created` field (a view collection
/// may not have one), leaving the backend's natural order.
fn default_order_by(resolver: &CollectionResolver<'_>) -> String {
    let root = resolver.root_collection();
    if root.has_field("created") {
        format!("{}.\"created\" DESC", quote_ident(root.table_name()))
    } else {
        String::new()
    }
}

fn rowid_expr(resolver: &CollectionResolver<'_>) -> String {
    let table = quote_ident(resolver.root_collection().table_name());
    match cratebase_filter::Resolver::dialect(resolver) {
        Dialect::Sqlite => format!("{table}.\"rowid\""),
        Dialect::Postgres => format!("{table}.ctid"),
    }
}

/// `field IN ($a, $b, ...)` with the ids bound from `start`.
pub fn in_placeholders(start: usize, count: usize) -> String {
    (start..start + count)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Turn a driver unique-violation into a per-field validation error.
///
/// SQLite reports `table.column`; Postgres reports the index name, which
/// only maps back to a column through the collection's own `indexes[]`
/// (PocketBase names them `idx_<hash>_<collection>`, so the name alone is
/// not enough). Falling back to the heuristic in [`crate::error`] keeps
/// something useful when neither matches.
pub fn unique_violation(collection: &Collection, detail: &str) -> DbError {
    let field = index_column(collection, detail)
        .unwrap_or_else(|| crate::error::unique_violation_field(detail));
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        field,
        FieldError::new(codes::NOT_UNIQUE, "Value must be unique."),
    );
    DbError::Validation(fields)
}

/// Resolve an index name against the collection's `indexes[]` and return
/// its first column.
fn index_column(collection: &Collection, detail: &str) -> Option<String> {
    let name = detail.split(',').next().unwrap_or(detail).trim();
    // SQLite's "posts.slug" form already names the column.
    let name = name.rsplit('.').next().unwrap_or(name).trim_matches('"');
    for stmt in &collection.indexes {
        let Some(def) = crate::schema::parse_index(stmt) else {
            continue;
        };
        if def.name != name {
            continue;
        }
        let first = def.columns_raw.split(',').next()?.trim();
        let column: String = first
            .trim_matches(|c| c == '`' || c == '"' || c == '[' || c == ']')
            .split_whitespace()
            .next()?
            .to_string();
        if collection.has_field(&column) {
            return Some(column);
        }
    }
    // SQLite already gave us a column name; trust it when it is a field.
    if collection.has_field(name) {
        return Some(name.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::CollectionStore;
    use crate::context::RequestContext;
    use cratebase_core::{CollectionType, Field, FieldKind, FieldType};
    use serde_json::json;
    use std::sync::Arc;

    fn store() -> (CollectionStore, Arc<Collection>) {
        let store = CollectionStore::new();
        let users = Collection::default_users();
        let mut posts = Collection::new("posts", CollectionType::Base);
        let pos = posts.fields.len() - 2;
        posts.fields.insert(
            pos,
            Field::new("title", FieldKind::default_for(FieldType::Text)),
        );
        posts.fields.insert(
            pos + 1,
            Field::new(
                "author",
                FieldKind::Relation {
                    collection_id: users.id.clone(),
                    cascade_delete: false,
                    min_select: 0,
                    max_select: 1,
                },
            ),
        );
        posts.indexes =
            vec!["CREATE UNIQUE INDEX `idx_posts_x` ON `posts` (`title`, `author`)".into()];
        let pos = posts.fields.len() - 2;
        posts
            .fields
            .insert(pos, Field::new("loc", FieldKind::GeoPoint {}));
        store.replace(vec![users, posts]);
        let posts = store.get("posts").unwrap();
        (store, posts)
    }

    #[test]
    fn sort_paths_and_errors() {
        let (store, posts) = store();
        let ctx = RequestContext::default();
        let r = CollectionResolver::new(posts, &store, &ctx, Dialect::Sqlite);

        assert_eq!(
            order_by(&r, None, 0).unwrap(),
            ("\"posts\".\"created\" DESC".to_string(), vec![])
        );
        assert_eq!(
            order_by(&r, Some(""), 0).unwrap().0,
            "\"posts\".\"created\" DESC"
        );
        assert_eq!(
            order_by(&r, Some("title,-created"), 0).unwrap().0,
            "\"posts\".\"title\" ASC, \"posts\".\"created\" DESC"
        );
        assert_eq!(order_by(&r, Some("@random"), 0).unwrap().0, "RANDOM()");
        assert_eq!(
            order_by(&r, Some("-@rowid"), 0).unwrap().0,
            "\"posts\".\"rowid\" DESC"
        );
        assert!(order_by(&r, Some("author.name"), 0)
            .unwrap()
            .0
            .contains("SELECT \"users\".\"name\""));
        assert_eq!(
            order_by(&r, Some("+title"), 0).unwrap().0,
            "\"posts\".\"title\" ASC"
        );
        // An unknown sort is a bare 400 with no `data` entry.
        let err = order_by(&r, Some("nope"), 0).unwrap_err();
        assert!(matches!(err, DbError::Filter(_)), "{err:?}");
        assert!(cratebase_core::AppError::from(err).body().data.is_empty());
    }

    #[test]
    fn split_sort_tokens_keeps_a_call_s_commas_together() {
        assert_eq!(
            split_sort_tokens("title,-created"),
            vec!["title", "-created"]
        );
        assert_eq!(
            split_sort_tokens("geoDistance(loc.lon, loc.lat, 1, 2)"),
            vec!["geoDistance(loc.lon, loc.lat, 1, 2)"]
        );
        assert_eq!(
            split_sort_tokens("-geoDistance(loc.lon, loc.lat, 1, 2),title"),
            vec!["-geoDistance(loc.lon, loc.lat, 1, 2)", "title"]
        );
        assert_eq!(split_sort_tokens(""), Vec::<&str>::new());
    }

    #[test]
    fn sort_geo_distance_binds_params_after_the_offset() {
        let (store, posts) = store();
        let ctx = RequestContext::default();
        let r = CollectionResolver::new(posts, &store, &ctx, Dialect::Sqlite);

        let (sql, params) = order_by(&r, Some("geoDistance(loc.lon, loc.lat, 1, 2)"), 0).unwrap();
        assert_eq!(
            sql,
            "geoDistance(json_extract(\"posts\".\"loc\", '$.lon'), json_extract(\"posts\".\"loc\", '$.lat'), $1, $2) ASC"
        );
        assert_eq!(params, vec![Sql::Int(1), Sql::Int(2)]);

        let (sql, params) = order_by(&r, Some("-geoDistance(loc.lon, loc.lat, 1, 2)"), 3).unwrap();
        assert!(sql.ends_with("$4, $5) DESC"), "{sql}");
        assert_eq!(params.len(), 2);

        // An invalid geoDistance sort (unknown field) is still a filter error.
        let err = order_by(&r, Some("geoDistance(nope.lon, loc.lat, 1, 2)"), 0).unwrap_err();
        assert!(matches!(err, DbError::Filter(_)), "{err:?}");
    }

    #[test]
    fn joins_force_distinct_and_shift_placeholders() {
        let (store, posts) = store();
        let ctx = RequestContext::default();
        let r = CollectionResolver::new(posts.clone(), &store, &ctx, Dialect::Sqlite);

        let mut q = Query::new(&posts);
        let rule = cratebase_filter::parse_and_compile("title != ''", &r, 0).unwrap();
        q.push_filter(rule);
        let user =
            cratebase_filter::parse_and_compile("title = 'x'", &r, q.params().len()).unwrap();
        q.push_filter(user);
        let (order_sql, order_params) = order_by(&r, Some("title"), q.params().len()).unwrap();
        q.set_order_by(order_sql);
        q.push_order_params(order_params);
        let sql = q.select_sql();
        assert!(sql.starts_with("SELECT \"posts\".* FROM \"posts\" WHERE"));
        assert!(sql.contains(" AND "));
        assert!(sql.contains("$1"), "{sql}");
        assert!(sql.ends_with("LIMIT $2 OFFSET $3"), "{sql}");
        assert_eq!(q.params(), &[Sql::Text("x".into())]);
        assert!(q.count_sql().starts_with("SELECT COUNT(*) FROM"));
        q.bind_page(30, 0);
        assert_eq!(
            q.params(),
            &[Sql::Text("x".into()), Sql::Int(30), Sql::Int(0)]
        );
    }

    #[test]
    fn unique_violation_resolves_the_index_to_its_first_column() {
        let (_, posts) = store();
        match unique_violation(&posts, "idx_posts_x") {
            DbError::Validation(f) => {
                assert_eq!(f.keys().next().unwrap(), "title");
                assert_eq!(f["title"].code, codes::NOT_UNIQUE);
            }
            other => panic!("{other:?}"),
        }
        match unique_violation(&posts, "posts.author") {
            DbError::Validation(f) => assert_eq!(f.keys().next().unwrap(), "author"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn params_map_json_to_sql() {
        assert_eq!(to_param(&Value::Null), Sql::Null);
        assert_eq!(to_param(&json!(true)), Sql::Int(1));
        assert_eq!(to_param(&json!(3)), Sql::Int(3));
        assert_eq!(to_param(&json!(1.5)), Sql::Real(1.5));
        assert_eq!(to_param(&json!("a")), Sql::Text("a".into()));
        assert_eq!(to_param(&json!(["a"])), Sql::Text("[\"a\"]".into()));
        assert_eq!(in_placeholders(2, 3), "$2, $3, $4");
    }
}
