//! API rule evaluation.
//!
//! PocketBase encodes a rule as `Option<String>`:
//!
//! * `None` — "superusers only". Every non-superuser request is denied.
//! * `Some("")` — public. Everyone passes.
//! * `Some(expr)` — the filter expression is AND-ed into the query, so a
//!   record the rule rejects is simply not in the result set (a denied
//!   `view` is a 404, never a 403 — that is deliberate: it stops the API
//!   from confirming that a hidden id exists).
//!
//! `createRule` is the odd one out: there is no row to filter yet, so it
//! is evaluated as a plain boolean against the submitted body.
//!
//! PocketBase turns those into four *different* HTTP outcomes, and this
//! module deliberately keeps them distinguishable rather than collapsing
//! them into one error (measured against v0.40.2, see
//! `tests/conformance/KNOWN_DIVERGENCES.md`):
//!
//! | rule | result |
//! | --- | --- |
//! | `listRule` rejects the row | an **empty list**, not an error |
//! | `viewRule`/`updateRule`/`deleteRule` reject the row | **404** |
//! | `createRule` rejects the body | **400** `"Failed to create record."`, `data: {}` |
//! | any rule is `None` and the caller is not a superuser | **403** `"Only superusers can perform this action."` |
//!
//! The last row is why [`is_superuser_only`] exists: it is a decision the
//! server layer must take *before* querying, since an empty result set
//! cannot tell "denied" from "nothing matched".

use cratebase_core::Collection;
use cratebase_filter::{CompiledFilter, Dialect, Expr, FilterError};
use serde_json::Value;

use crate::context::CollectionResolver;
use crate::engine::{quote_ident, Executor, Sql};
use crate::error::DbResult;
use crate::schema::{self, PhysicalKind};

/// What a rule means for the query being built.
#[derive(Debug)]
pub enum RuleOutcome {
    /// No filtering needed (superuser, or a public rule).
    AllowAll,
    /// The rule can never pass for this caller (superusers only).
    DenyAll,
    /// AND this into the query's `WHERE`.
    Filtered(CompiledFilter),
}

impl RuleOutcome {
    pub fn is_deny_all(&self) -> bool {
        matches!(self, RuleOutcome::DenyAll)
    }

    pub fn into_filter(self) -> Option<CompiledFilter> {
        match self {
            RuleOutcome::Filtered(f) => Some(f),
            _ => None,
        }
    }
}

/// Whether `rule` is the superuser-only form (`None`) and the caller is
/// not a superuser — the `403` case in the table above. Check this before
/// calling [`crate::records::list`], which cannot distinguish a denied
/// list from an empty one.
pub fn is_superuser_only(rule: &Option<String>, ctx: &crate::context::RequestContext) -> bool {
    rule.is_none() && !ctx.is_superuser()
}

/// Compile `rule` for the current caller. `param_offset` reserves
/// placeholder slots already used by the surrounding query.
pub fn evaluate(
    rule: &Option<String>,
    resolver: &CollectionResolver<'_>,
    param_offset: usize,
) -> DbResult<RuleOutcome> {
    // Checked first: a superuser bypasses even a syntactically broken
    // rule, exactly as PocketBase does.
    if resolver.ctx().is_superuser() {
        return Ok(RuleOutcome::AllowAll);
    }
    match rule {
        None => Ok(RuleOutcome::DenyAll),
        Some(expr) if expr.trim().is_empty() => Ok(RuleOutcome::AllowAll),
        Some(expr) => Ok(RuleOutcome::Filtered(cratebase_filter::parse_and_compile(
            expr,
            resolver,
            param_offset,
        )?)),
    }
}

/// Evaluate a `createRule` against the submitted body.
///
/// Two strategies, in order:
///
/// 1. [`cratebase_filter::evaluate`] against `ctx.body` in-process. This
///    is the common case (`@request.auth.id != ""`, `status = "draft"`)
///    and costs no database round trip.
/// 2. When the expression needs the database — a relation path, a
///    back-relation, `@collection.X`, `geoDistance(...)` — the evaluator
///    reports [`FilterError::Unsupported`] and we fall back to running
///    the compiled SQL against a synthetic one-row table standing in for
///    the record that does not exist yet (see [`body_row_sql`]).
pub async fn check_create_rule(
    ex: &dyn Executor,
    resolver: &CollectionResolver<'_>,
    rule: &Option<String>,
) -> DbResult<bool> {
    let body = resolver.ctx().body.clone();
    check_rule_against_row(ex, resolver, rule, &body).await
}

/// [`check_create_rule`] generalised to any single row, given as a
/// PocketBase-shaped JSON map.
///
/// The realtime fan-out needs exactly this and cannot use a plain query:
/// a `delete` event has to be access-checked against a row that is
/// already gone, so there is nothing to `SELECT`. Feeding the in-memory
/// snapshot through the same two strategies — evaluate in process, else
/// stand the row up as a one-row derived table — answers for a deleted
/// record as readily as for an uncommitted one.
pub async fn check_rule_against_row(
    ex: &dyn Executor,
    resolver: &CollectionResolver<'_>,
    rule: &Option<String>,
    row: &serde_json::Map<String, Value>,
) -> DbResult<bool> {
    if resolver.ctx().is_superuser() {
        return Ok(true);
    }
    let expr = match rule {
        None => return Ok(false),
        Some(e) if e.trim().is_empty() => return Ok(true),
        Some(e) => e,
    };
    let ast = cratebase_filter::parse_cached(expr)?;
    match cratebase_filter::evaluate(&ast, row, resolver) {
        Ok(passed) => Ok(passed),
        Err(FilterError::Unsupported(_)) => check_via_sql(ex, resolver, &ast, row).await,
        Err(e) => Err(e.into()),
    }
}

/// Run a compiled rule against a synthetic single row built from the
/// request body:
///
/// ```sql
/// SELECT 1 FROM (SELECT $1 AS "id", $2 AS "title") AS "posts"
///   LEFT JOIN ... WHERE <rule>
/// ```
///
/// Aliasing the subquery as the collection's table name is what makes the
/// compiler's fully qualified `"posts"."title"` references resolve, and
/// keeps any joins it emitted (back-relations, `@collection.X`) valid.
async fn check_via_sql(
    ex: &dyn Executor,
    resolver: &CollectionResolver<'_>,
    expr: &Expr,
    row: &serde_json::Map<String, Value>,
) -> DbResult<bool> {
    let collection = resolver.root_collection();
    let (from, mut params) = body_row_sql(
        collection,
        row,
        cratebase_filter::Resolver::dialect(resolver),
    );
    let compiled = cratebase_filter::compile(expr, resolver, params.len())?;
    params.extend(compiled.params.iter().map(crate::query::to_param));

    let mut sql = format!("SELECT 1 FROM {from}");
    for join in &compiled.joins {
        sql.push(' ');
        sql.push_str(&join.sql);
    }
    sql.push_str(" WHERE ");
    sql.push_str(&compiled.sql);
    Ok(ex.query_one(&sql, &params).await?.is_some())
}

/// The `(SELECT ...) AS "<table>"` fragment plus its bound values: one
/// column per field, taken from `body` or the field's zero value.
///
/// Postgres cannot infer the type of a bare placeholder inside a derived
/// table, so each one is cast to the field's physical column type.
fn body_row_sql(
    collection: &Collection,
    body: &serde_json::Map<String, Value>,
    dialect: Dialect,
) -> (String, Vec<Sql>) {
    let mut columns = Vec::with_capacity(collection.fields.len());
    let mut params = Vec::with_capacity(collection.fields.len());
    for (i, field) in collection.fields.iter().enumerate() {
        let value = body.get(&field.name).cloned().unwrap_or(Value::Null);
        params.push(crate::records::column_value(field, &value));
        let cast = match dialect {
            Dialect::Sqlite => String::new(),
            Dialect::Postgres => match schema::physical_kind(field) {
                PhysicalKind::Text => "::text".into(),
                PhysicalKind::Number => "::double precision".into(),
                PhysicalKind::Bool => "::integer".into(),
            },
        };
        columns.push(format!("${}{cast} AS {}", i + 1, quote_ident(&field.name)));
    }
    (
        format!(
            "(SELECT {}) AS {}",
            columns.join(", "),
            quote_ident(collection.table_name())
        ),
        params,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::CollectionStore;
    use crate::context::{AuthContext, RequestContext};
    use crate::db::Db;
    use cratebase_core::{Collection, CollectionType, Field, FieldKind, FieldType, Record};
    use serde_json::json;
    use std::sync::Arc;

    fn posts_store() -> (CollectionStore, Arc<Collection>) {
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
        store.replace(vec![users, posts]);
        let posts = store.get("posts").unwrap();
        (store, posts)
    }

    #[test]
    fn none_denies_empty_allows_superuser_bypasses() {
        let (store, posts) = posts_store();
        let ctx = RequestContext::default();
        let r = CollectionResolver::new(posts.clone(), &store, &ctx, Dialect::Sqlite);
        assert!(evaluate(&None, &r, 0).unwrap().is_deny_all());
        assert!(matches!(
            evaluate(&Some(String::new()), &r, 0).unwrap(),
            RuleOutcome::AllowAll
        ));
        let filtered = evaluate(&Some("title != ''".into()), &r, 3).unwrap();
        match filtered {
            RuleOutcome::Filtered(f) => assert!(f.sql.contains("\"posts\".\"title\"")),
            other => panic!("{other:?}"),
        }

        assert!(is_superuser_only(&None, &ctx));
        assert!(!is_superuser_only(&Some(String::new()), &ctx));

        let su = RequestContext::superuser();
        let r = CollectionResolver::new(posts, &store, &su, Dialect::Sqlite);
        assert!(matches!(
            evaluate(&None, &r, 0).unwrap(),
            RuleOutcome::AllowAll
        ));
        assert!(!is_superuser_only(&None, &su));
    }

    #[tokio::test]
    async fn create_rule_evaluates_against_the_body() {
        let db = Db::memory().await.unwrap();
        let (store, posts) = posts_store();
        let ctx = RequestContext {
            body: json!({"title": "hello"}).as_object().cloned().unwrap(),
            ..Default::default()
        };
        let r = CollectionResolver::new(posts.clone(), &store, &ctx, Dialect::Sqlite);
        assert!(check_create_rule(&db, &r, &Some(String::new()))
            .await
            .unwrap());
        assert!(!check_create_rule(&db, &r, &None).await.unwrap());
        assert!(check_create_rule(&db, &r, &Some("title = 'hello'".into()))
            .await
            .unwrap());
        assert!(!check_create_rule(&db, &r, &Some("title = 'other'".into()))
            .await
            .unwrap());
        assert!(
            !check_create_rule(&db, &r, &Some("@request.auth.id != ''".into()))
                .await
                .unwrap()
        );

        let mut user = Record::new(store.get("users").unwrap());
        user.set_id("u1");
        let ctx = RequestContext {
            body: json!({"title": "hello", "author": "u1"})
                .as_object()
                .cloned()
                .unwrap(),
            auth: Some(AuthContext::new(user)),
            ..Default::default()
        };
        let r = CollectionResolver::new(posts, &store, &ctx, Dialect::Sqlite);
        assert!(
            check_create_rule(&db, &r, &Some("author = @request.auth.id".into()))
                .await
                .unwrap()
        );
    }

    /// A rule that reaches through a relation cannot be answered from the
    /// body alone, so it must take the SQL path and still be correct.
    #[tokio::test]
    async fn create_rule_falls_back_to_sql_for_relation_paths() {
        let db = Db::memory().await.unwrap();
        let (store, posts) = posts_store();
        db.execute(
            &crate::schema::create_table_sql(db.backend, &posts).unwrap(),
            &[],
        )
        .await
        .unwrap();
        db.execute(
            "INSERT INTO users (id, email, tokenKey, verified) VALUES ('u1', 'a@b.co', 'k', 1)",
            &[],
        )
        .await
        .unwrap();

        let ctx = RequestContext {
            body: json!({"title": "t", "author": "u1"})
                .as_object()
                .cloned()
                .unwrap(),
            ..Default::default()
        };
        let r = CollectionResolver::new(posts.clone(), &store, &ctx, Dialect::Sqlite);
        assert!(
            check_create_rule(&db, &r, &Some("author.verified = true".into()))
                .await
                .unwrap()
        );
        assert!(
            !check_create_rule(&db, &r, &Some("author.email = 'nope@x.co'".into()))
                .await
                .unwrap()
        );
    }

    /// GH #18: `createRule` combining `@request.body.<relation> =
    /// @request.auth.<relation>` with `@request.body.<relation>.<field> =
    /// @request.auth.<field>` — a child collection creating into any
    /// sibling row that shares the caller's own scope, not just the
    /// caller's default relation target. Reproduces the reported
    /// multi-outlet `customers`/`shops`/`businesses` schema.
    #[tokio::test]
    async fn create_rule_body_relation_dot_path_scopes_to_callers_business() {
        let db = Db::memory().await.unwrap();
        let store = CollectionStore::new();

        let businesses = Collection::new("businesses", CollectionType::Base);

        let mut shops = Collection::new("shops", CollectionType::Base);
        shops.fields.insert(
            0,
            Field::new(
                "business",
                FieldKind::Relation {
                    collection_id: businesses.id.clone(),
                    cascade_delete: false,
                    min_select: 0,
                    max_select: 1,
                },
            ),
        );

        let mut users = Collection::default_users();
        let pos = users.fields.len() - 2;
        users.fields.insert(
            pos,
            Field::new(
                "shop",
                FieldKind::Relation {
                    collection_id: shops.id.clone(),
                    cascade_delete: false,
                    min_select: 0,
                    max_select: 1,
                },
            ),
        );
        users.fields.insert(
            pos + 1,
            Field::new(
                "business",
                FieldKind::Relation {
                    collection_id: businesses.id.clone(),
                    cascade_delete: false,
                    min_select: 0,
                    max_select: 1,
                },
            ),
        );

        let mut customers = Collection::new("customers", CollectionType::Base);
        customers.fields.insert(
            0,
            Field::new(
                "shop",
                FieldKind::Relation {
                    collection_id: shops.id.clone(),
                    cascade_delete: false,
                    min_select: 0,
                    max_select: 1,
                },
            ),
        );

        store.replace(vec![businesses, shops, users, customers]);
        let shops = store.get("shops").unwrap();
        let customers = store.get("customers").unwrap();
        let users = store.get("users").unwrap();

        db.execute(
            &crate::schema::create_table_sql(db.backend, &shops).unwrap(),
            &[],
        )
        .await
        .unwrap();
        db.execute(
            "INSERT INTO shops (id, business) VALUES ('shop1', 'biz1')",
            &[],
        )
        .await
        .unwrap();
        db.execute(
            "INSERT INTO shops (id, business) VALUES ('shop2', 'biz1')",
            &[],
        )
        .await
        .unwrap();
        db.execute(
            "INSERT INTO shops (id, business) VALUES ('shop3', 'biz2')",
            &[],
        )
        .await
        .unwrap();

        let create_rule = Some(
            "@request.body.shop = @request.auth.shop || \
             (@request.auth.business != \"\" && @request.body.shop.business = @request.auth.business)"
                .to_string(),
        );

        let mut owner = Record::new(users);
        owner.set_id("owner1");
        owner.set("shop", Value::String("shop1".into()));
        owner.set("business", Value::String("biz1".into()));

        // Same-shop create: the plain `@request.body.shop = @request.auth.shop`
        // clause already covers this.
        let ctx = RequestContext {
            body: json!({"shop": "shop1"}).as_object().cloned().unwrap(),
            auth: Some(AuthContext::new(owner.clone())),
            ..Default::default()
        };
        let r = CollectionResolver::new(customers.clone(), &store, &ctx, Dialect::Sqlite);
        assert!(check_create_rule(&db, &r, &create_rule).await.unwrap());

        // Secondary outlet under the *same* business: this is the bug —
        // `@request.body.shop.business` must resolve against the
        // submitted `shop2`, not the caller's own row.
        let ctx = RequestContext {
            body: json!({"shop": "shop2"}).as_object().cloned().unwrap(),
            auth: Some(AuthContext::new(owner.clone())),
            ..Default::default()
        };
        let r = CollectionResolver::new(customers.clone(), &store, &ctx, Dialect::Sqlite);
        assert!(check_create_rule(&db, &r, &create_rule).await.unwrap());

        // A shop under a *different* business is still rejected.
        let ctx = RequestContext {
            body: json!({"shop": "shop3"}).as_object().cloned().unwrap(),
            auth: Some(AuthContext::new(owner.clone())),
            ..Default::default()
        };
        let r = CollectionResolver::new(customers.clone(), &store, &ctx, Dialect::Sqlite);
        assert!(!check_create_rule(&db, &r, &create_rule).await.unwrap());

        // A solo owner with no `business` set at all stays scoped to their
        // own shop only (the `@request.auth.business != ""` guard).
        let mut solo = Record::new(store.get("users").unwrap());
        solo.set_id("owner2");
        solo.set("shop", Value::String("shop1".into()));
        let ctx = RequestContext {
            body: json!({"shop": "shop2"}).as_object().cloned().unwrap(),
            auth: Some(AuthContext::new(solo)),
            ..Default::default()
        };
        let r = CollectionResolver::new(customers, &store, &ctx, Dialect::Sqlite);
        assert!(!check_create_rule(&db, &r, &create_rule).await.unwrap());
    }
}
