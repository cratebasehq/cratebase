use cratebase_core::Collection;
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

        if ident == "id" || ident == "created" || ident == "updated" || self.collection.field(ident).is_some()
        {
            let quoted = self
                .backend
                .quote_ident(ident)
                .map_err(|_| FilterError::UnknownField(ident.to_string()))?;
            return Ok(Resolved::Column(quoted));
        }

        Err(FilterError::UnknownField(ident.to_string()))
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

pub fn evaluate_rule(
    rule: &Option<String>,
    collection: &Collection,
    backend: Backend,
    ctx: &RequestContext,
    param_offset: usize,
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
            };
            let compiled =
                cratebase_filter::parse_and_compile(expr, &resolver, backend.dialect(), param_offset)?;
            Ok(RuleOutcome::Filtered(compiled))
        }
    }
}
