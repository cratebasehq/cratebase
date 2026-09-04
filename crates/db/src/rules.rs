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
    if resolver.ctx().is_superuser() {
        return Ok(true);
    }
    let expr = match rule {
        None => return Ok(false),
        Some(e) if e.trim().is_empty() => return Ok(true),
        Some(e) => e,
    };
    let ast = cratebase_filter::parse_cached(expr)?;
    let body = resolver.ctx().body.clone();
    match cratebase_filter::evaluate(&ast, &body, resolver) {
        Ok(passed) => Ok(passed),
        Err(FilterError::Unsupported(_)) => check_via_sql(ex, resolver, &ast).await,
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
) -> DbResult<bool> {
    let collection = resolver.root_collection();
    let (from, mut params) = body_row_sql(
        collection,
        &resolver.ctx().body,
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
}
