use std::collections::HashMap;

use cratebase_core::{Collection, Field, FieldType};
use cratebase_filter::{FilterError, Resolved, Resolver};
use serde_json::{Map, Value};

use crate::backend::Backend;

/// The authenticated identity for the current request, if any. Populated by
/// `cratebase-server` from a verified JWT before rules/filters are
/// evaluated.
#[derive(Debug, Clone, Default)]
pub struct AuthContext {
    pub id: String,
    pub collection_id: String,
    pub is_superuser: bool,
    /// Extra auth-record fields exposed to rules as `@request.auth.<field>`.
    pub record: Map<String, Value>,
}

/// Everything a filter/rule expression may reference beyond plain record
/// columns: the caller's identity and (for create/update rules) the
/// incoming request body.
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    pub auth: Option<AuthContext>,
    pub data: Option<Map<String, Value>>,
}

/// Resolves filter identifiers against a specific collection's schema and
/// the current request context. Shared by record listing/viewing (user
/// filters) and API rule evaluation (`listRule`, `createRule`, ...).
pub struct CollectionResolver<'a> {
    pub collection: &'a Collection,
    pub backend: Backend,
    pub ctx: &'a RequestContext,
    /// When `true` (create rules only — there is no existing DB row yet),
    /// bare field identifiers resolve against `ctx.data` instead of a SQL
    /// column, since there is no row to resolve them against yet.
    pub use_data_for_fields: bool,
    /// Target collections for relation dot-notation (`author.name`),
    /// keyed by the relation field's name on `collection`. `resolve` is
    /// synchronous and can't fetch another collection's schema on demand,
    /// so callers that support dot-notation prefetch it via
    /// [`load_related_collections`] first. Idents naming a relation field
    /// missing from this map fail with `UnknownField`.
    pub related: HashMap<String, Collection>,
}

/// Whether a field stores more than one value (a JSON array in a TEXT
/// column) rather than a plain scalar.
fn is_multiple(field: &Field) -> bool {
    field.field_type.supports_multiple() && field.options.multiple.unwrap_or(false)
}

impl<'a> Resolver for CollectionResolver<'a> {
    fn resolve(&self, ident: &str) -> Result<Resolved, FilterError> {
        if let Some(rest) = ident.strip_prefix("@request.auth.") {
            return Ok(Resolved::Value(match &self.ctx.auth {
                None => Value::Null,
                Some(auth) => match rest {
                    "id" => Value::String(auth.id.clone()),
                    "collectionId" => Value::String(auth.collection_id.clone()),
                    _ => auth.record.get(rest).cloned().unwrap_or(Value::Null),
                },
            }));
        }
        if ident == "@request.auth" {
            return Ok(Resolved::Value(Value::Bool(self.ctx.auth.is_some())));
        }
        if let Some(rest) = ident.strip_prefix("@request.data.") {
            return Ok(Resolved::Value(
                self.ctx
                    .data
                    .as_ref()
                    .and_then(|d| d.get(rest).cloned())
                    .unwrap_or(Value::Null),
            ));
        }

        // Relation dot-notation (`author.name`): traverse `author` as a
        // relation field on this collection into the target collection's
        // schema. Not a context variable, so anything past the first dot
        // that isn't `@request.*` lands here.
        if !ident.starts_with('@') {
            if let Some((relation_name, target_field)) = ident.split_once('.') {
                return self.resolve_relation(ident, relation_name, target_field);
            }
        }

        if ident == "id" || ident == "created" || ident == "updated" {
            return self.resolve_scalar(ident, ident);
        }
        if let Some(field) = self.collection.field(ident) {
            if self.use_data_for_fields {
                return self.resolve_scalar(ident, ident);
            }
            let quoted = self
                .backend
                .quote_ident(ident)
                .map_err(|_| FilterError::UnknownField(ident.to_string()))?;
            return Ok(if is_multiple(field) {
                Resolved::MultiColumn(quoted)
            } else {
                Resolved::Column(quoted)
            });
        }

        Err(FilterError::UnknownField(ident.to_string()))
    }
}

impl<'a> CollectionResolver<'a> {
    /// Resolve a builtin (`id`/`created`/`updated`) or `use_data_for_fields`
    /// scalar identifier: a plain SQL column, or a value pulled from the
    /// submitted payload when there is no table row to resolve it against
    /// yet (create rules).
    fn resolve_scalar(&self, ident: &str, field_name: &str) -> Result<Resolved, FilterError> {
        if self.use_data_for_fields {
            return Ok(Resolved::Value(
                self.ctx
                    .data
                    .as_ref()
                    .and_then(|d| d.get(field_name).cloned())
                    .unwrap_or(Value::Null),
            ));
        }
        let quoted = self
            .backend
            .quote_ident(field_name)
            .map_err(|_| FilterError::UnknownField(ident.to_string()))?;
        Ok(Resolved::Column(quoted))
    }

    /// Resolve `<relation_name>.<target_field>` through a relation field on
    /// this collection into the prefetched target collection's schema.
    fn resolve_relation(
        &self,
        ident: &str,
        relation_name: &str,
        target_field: &str,
    ) -> Result<Resolved, FilterError> {
        let unknown = || FilterError::UnknownField(ident.to_string());

        let field = self
            .collection
            .field(relation_name)
            .filter(|f| f.field_type == FieldType::Relation)
            .ok_or_else(unknown)?;
        let target = self.related.get(relation_name).ok_or_else(unknown)?;

        let target_field_exists = target_field == "id"
            || target_field == "created"
            || target_field == "updated"
            || target.field(target_field).is_some();
        if !target_field_exists {
            return Err(unknown());
        }

        let array_column = self
            .backend
            .quote_ident(relation_name)
            .map_err(|_| unknown())?;
        let target_table = self
            .backend
            .quote_ident(&target.table_name())
            .map_err(|_| unknown())?;
        let target_column = self
            .backend
            .quote_ident(target_field)
            .map_err(|_| unknown())?;

        if is_multiple(field) {
            Ok(Resolved::RelatedMulti {
                array_column,
                target_table,
                target_column,
            })
        } else {
            // A single-valued relation: the related row's field, reached
            // via a scalar correlated subquery, behaves like any other
            // column in a comparison.
            Ok(Resolved::Column(format!(
                "(SELECT {target_column} FROM {target_table} WHERE \"id\" = {array_column})"
            )))
        }
    }
}

/// Prefetch the target collections referenced by relation dot-notation
/// (`author.name`) in a filter expression, keyed by relation field name —
/// the shape [`CollectionResolver::related`] expects. `resolve` is
/// synchronous and can't load another collection's schema on demand, so
/// callers that accept a user-authored filter (list/view queries) call
/// this first.
///
/// Idents that don't actually name a relation field are silently skipped:
/// `resolve` reports those as `UnknownField` at compile time instead of
/// failing prefetch outright.
pub async fn load_related_collections(
    db: &crate::pool::Db,
    collection: &Collection,
    filter: &str,
) -> crate::error::DbResult<HashMap<String, Collection>> {
    let mut related = HashMap::new();
    let Ok(names) = cratebase_filter::relation_idents(filter) else {
        return Ok(related);
    };
    for name in names {
        let Some(field) = collection.field(&name) else {
            continue;
        };
        if field.field_type != FieldType::Relation {
            continue;
        }
        let Some(target_id) = field.options.collection_id.as_deref() else {
            continue;
        };
        if let Ok(target) = crate::collections::get_collection_by_id(db, target_id).await {
            related.insert(name, target);
        }
    }
    Ok(related)
}

/// [`load_related_collections`] guarded for an `Option<String>` rule
/// (`None`/empty are the common admin-only/public case, and skip the
/// lex + any DB call entirely rather than paying for a `relation_idents`
/// scan on an empty string every time). Shared by every rule-checking
/// call site instead of each writing this same match arm.
pub async fn load_related_collections_for_rule(
    db: &crate::pool::Db,
    collection: &Collection,
    rule: &Option<String>,
) -> crate::error::DbResult<HashMap<String, Collection>> {
    match rule {
        Some(expr) if !expr.trim().is_empty() => {
            load_related_collections(db, collection, expr).await
        }
        _ => Ok(HashMap::new()),
    }
}

/// A rule is satisfied unconditionally by superusers, denied for everyone
/// when `None` (admin-only), always satisfied when `Some("")` (public), and
/// otherwise evaluated as a filter expression against the target record.
pub enum RuleOutcome {
    /// No SQL filtering needed: allow every row (superuser or public rule).
    AllowAll,
    /// Rule can never pass for the current caller (locked, non-superuser).
    DenyAll,
    /// Apply this compiled filter as an additional WHERE condition.
    Filtered(cratebase_filter::CompiledFilter),
}

/// `related` is caller-prefetched via [`load_related_collections`] — kept
/// synchronous (no DB access) so it stays safely callable from inside a
/// transaction (e.g. `/api/batch`'s update/delete rule checks) without
/// risking a pool-acquire deadlock the way an internal prefetch here
/// would (see `crates/db/src/pool.rs`'s doc comment on why a
/// transaction-holding request can't also acquire a second pool
/// connection). Callers with a real pool connection available (every
/// non-transactional rule check) should prefetch with
/// `load_related_collections` first; callers inside a transaction (only
/// `/api/batch` today) pass an empty map, same as `createRule` already
/// does there — dot-notation in a rule checked mid-batch-transaction
/// isn't supported, consistent with that existing limitation.
pub fn evaluate_rule(
    rule: &Option<String>,
    collection: &Collection,
    backend: Backend,
    ctx: &RequestContext,
    param_offset: usize,
    related: &HashMap<String, Collection>,
) -> Result<RuleOutcome, FilterError> {
    if let Some(auth) = &ctx.auth {
        if auth.is_superuser {
            return Ok(RuleOutcome::AllowAll);
        }
    }
    match rule {
        None => Ok(RuleOutcome::DenyAll),
        Some(expr) if expr.trim().is_empty() => Ok(RuleOutcome::AllowAll),
        Some(expr) => {
            let resolver = CollectionResolver {
                collection,
                backend,
                ctx,
                use_data_for_fields: false,
                related: related.clone(),
            };
            let compiled = cratebase_filter::parse_and_compile(
                expr,
                &resolver,
                backend.dialect(),
                param_offset,
            )?;
            Ok(RuleOutcome::Filtered(compiled))
        }
    }
}

/// Evaluate a rule as a plain boolean against a fully materialized set of
/// field values (a submitted payload, or a record snapshot) rather than
/// the live table, via a FROM-less `SELECT ... WHERE <expr>`. Both
/// supported backends accept this, so it reuses the exact same
/// parser/compiler as every other rule instead of maintaining a second,
/// JSON-only expression evaluator.
async fn evaluate_bool_rule(
    db: &crate::pool::Db,
    rule: &Option<String>,
    collection: &Collection,
    ctx: &RequestContext,
    related: &HashMap<String, Collection>,
) -> crate::error::DbResult<bool> {
    if let Some(auth) = &ctx.auth {
        if auth.is_superuser {
            return Ok(true);
        }
    }
    let expr = match rule {
        None => return Ok(false),
        Some(expr) if expr.trim().is_empty() => return Ok(true),
        Some(expr) => expr,
    };

    let resolver = CollectionResolver {
        collection,
        backend: db.backend,
        ctx,
        use_data_for_fields: true,
        related: related.clone(),
    };
    let compiled = cratebase_filter::parse_and_compile(expr, &resolver, db.backend.dialect(), 0)?;

    let mut args = sqlx::any::AnyArguments::default();
    for p in compiled.params {
        crate::value::bind_filter_value(&mut args, p)
            .map_err(|e| crate::error::DbError::Sqlx(sqlx::Error::Encode(e)))?;
    }
    let sql = format!("SELECT 1 WHERE {}", compiled.sql);
    let row = sqlx::query_with(&sql, args)
        .fetch_optional(&db.pool)
        .await?;
    Ok(row.is_some())
}

/// Transaction-scoped counterpart to [`evaluate_bool_rule`]. See
/// [`crate::collections::get_collection_by_id_tx`] for why this exists.
async fn evaluate_bool_rule_tx(
    tx: &mut crate::records::RecordTx,
    backend: Backend,
    rule: &Option<String>,
    collection: &Collection,
    ctx: &RequestContext,
) -> crate::error::DbResult<bool> {
    if let Some(auth) = &ctx.auth {
        if auth.is_superuser {
            return Ok(true);
        }
    }
    let expr = match rule {
        None => return Ok(false),
        Some(expr) if expr.trim().is_empty() => return Ok(true),
        Some(expr) => expr,
    };

    let resolver = CollectionResolver {
        collection,
        backend,
        ctx,
        use_data_for_fields: true,
        related: HashMap::new(),
    };
    let compiled = cratebase_filter::parse_and_compile(expr, &resolver, backend.dialect(), 0)?;

    let mut args = sqlx::any::AnyArguments::default();
    for p in compiled.params {
        crate::value::bind_filter_value(&mut args, p)
            .map_err(|e| crate::error::DbError::Sqlx(sqlx::Error::Encode(e)))?;
    }
    let sql = format!("SELECT 1 WHERE {}", compiled.sql);
    let row = sqlx::query_with(&sql, args)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(row.is_some())
}

/// Transaction-scoped counterpart to [`evaluate_create_rule`]. See
/// [`crate::collections::get_collection_by_id_tx`] for why this exists.
pub async fn evaluate_create_rule_tx(
    tx: &mut crate::records::RecordTx,
    backend: Backend,
    rule: &Option<String>,
    collection: &Collection,
    ctx: &RequestContext,
) -> crate::error::DbResult<bool> {
    evaluate_bool_rule_tx(tx, backend, rule, collection, ctx).await
}

/// Evaluate a `createRule` as a plain boolean, since there is no existing
/// database row to attach a WHERE clause to. Bare field names resolve
/// against the submitted `ctx.data` (see `CollectionResolver::use_data_for_fields`).
/// Prefetches relation-dot-notation targets itself: this has exactly one
/// call site (a record `create` handler, called once per request), so
/// unlike [`evaluate_record_rule`] there's no hot loop for a caller to
/// hoist a shared prefetch out of.
pub async fn evaluate_create_rule(
    db: &crate::pool::Db,
    rule: &Option<String>,
    collection: &Collection,
    ctx: &RequestContext,
) -> crate::error::DbResult<bool> {
    let related = load_related_collections_for_rule(db, collection, rule).await?;
    evaluate_bool_rule(db, rule, collection, ctx, &related).await
}

/// Evaluate `listRule`/`viewRule` against a record snapshot (`ctx.data`)
/// instead of the live table. Used by realtime delivery: a `delete` event
/// fires after the row is already gone, so there is no table row left to
/// filter against — the record's last known values are all that's left to
/// evaluate the rule with, same mechanism `evaluate_create_rule` uses for
/// a row that doesn't exist yet.
///
/// `related` is caller-prefetched, unlike [`evaluate_create_rule`]:
/// `RealtimeHub::publish` calls this once per subscriber, and a naive
/// per-call prefetch would turn one published event into N extra DB
/// round-trips for N subscribers. Callers with only one evaluation to do
/// (e.g. the feature-flags plugin) just prefetch immediately before
/// calling; `publish` prefetches at most twice up front (`listRule`'s and
/// `viewRule`'s targets — every subscriber checks one or the other) and
/// reuses those two maps across every subscriber.
pub async fn evaluate_record_rule(
    db: &crate::pool::Db,
    rule: &Option<String>,
    collection: &Collection,
    ctx: &RequestContext,
    related: &HashMap<String, Collection>,
) -> crate::error::DbResult<bool> {
    evaluate_bool_rule(db, rule, collection, ctx, related).await
}
