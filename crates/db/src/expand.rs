//! Relation expansion (`?expand=author,comments_via_post.author`).
//!
//! The rule that shapes this module: **one query per target collection
//! per level**, never one per record. A page of 500 posts whose `author`
//! is expanded costs exactly one extra `WHERE id IN (...)`, and a second
//! level costs one more.
//!
//! Expanded records go through the target collection's `viewRule` for
//! non-superusers. A record the rule hides is simply left out of the
//! `expand` object rather than failing the request — that is what
//! PocketBase does, and it keeps a list from becoming an oracle for
//! hidden ids.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cratebase_core::{Collection, Field, FieldType, Record, SerializeOptions};
use cratebase_filter::Dialect;
use futures::future::BoxFuture;
use indexmap::IndexMap;
use serde_json::Value;

use crate::collections::CollectionStore;
use crate::context::{CollectionResolver, RequestContext};
use crate::engine::{quote_ident, Executor, Sql};
use crate::error::DbResult;
use crate::query::{in_placeholders, Query};
use crate::records::RowDecoder;
use crate::{rules, schema};

/// PocketBase's `maxExpandDepth`.
pub const MAX_DEPTH: usize = 6;
/// PocketBase caps a back-relation expansion at 1000 rows per parent.
pub const MAX_BACK_RELATION_ROWS: usize = 1000;

/// Expand `spec` (comma-separated, `.`-nested paths) onto `records`.
/// `depth` is the current nesting level; callers pass `0`.
pub async fn resolve(
    ex: &dyn Executor,
    store: &CollectionStore,
    ctx: &RequestContext,
    records: &mut [Record],
    spec: &str,
    depth: usize,
) -> DbResult<()> {
    let paths = parse_spec(spec);
    resolve_paths(ex, store, ctx, records, paths, depth).await
}

/// `"a.b, c"` → `[["a", "b"], ["c"]]`, dropping empty segments.
fn parse_spec(spec: &str) -> Vec<Vec<String>> {
    spec.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            p.split('.')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|p: &Vec<String>| !p.is_empty())
        .collect()
}

/// What one expand key at this level refers to.
enum Target {
    /// A relation field on the records' own collection.
    Forward {
        field: Field,
        collection: Arc<Collection>,
    },
    /// `<collection>_via_<field>`: rows of another collection whose
    /// relation field points back here.
    Back {
        collection: Arc<Collection>,
        field: Field,
    },
}

fn classify(store: &CollectionStore, root: &Collection, key: &str) -> Option<Target> {
    if let Some(field) = root.field(key) {
        if field.field_type() != FieldType::Relation {
            return None;
        }
        let collection = store.get_by_id(field.relation_collection_id()?)?;
        return Some(Target::Forward {
            field: field.clone(),
            collection,
        });
    }
    let (name, field_name) = key.split_once("_via_")?;
    let collection = store.get(name)?;
    let field = collection.field(field_name)?.clone();
    if field.relation_collection_id() != Some(root.id.as_str()) {
        return None;
    }
    Some(Target::Back { collection, field })
}

fn resolve_paths<'a>(
    ex: &'a dyn Executor,
    store: &'a CollectionStore,
    ctx: &'a RequestContext,
    records: &'a mut [Record],
    paths: Vec<Vec<String>>,
    depth: usize,
) -> BoxFuture<'a, DbResult<()>> {
    Box::pin(async move {
        if records.is_empty() || paths.is_empty() || depth >= MAX_DEPTH {
            return Ok(());
        }
        let root = records[0].collection().clone();

        // Group the requested paths by their first segment, keeping the
        // order the client asked for.
        let mut groups: IndexMap<String, Vec<Vec<String>>> = IndexMap::new();
        for path in paths {
            let (head, rest) = path.split_first().expect("non-empty path");
            groups.entry(head.clone()).or_default().push(rest.to_vec());
        }

        // Resolve each key, then fetch once per target collection.
        let mut targets: IndexMap<String, Target> = IndexMap::new();
        for key in groups.keys() {
            // An unresolvable path is skipped rather than failing the
            // whole request, matching PocketBase's tolerance for expands
            // that no longer exist.
            if let Some(target) = classify(store, &root, key) {
                targets.insert(key.clone(), target);
            }
        }

        let forward_ids = forward_ids_by_collection(records, &targets);
        let mut fetched: HashMap<String, Vec<Record>> = HashMap::new();
        for (collection_id, ids) in forward_ids {
            let Some(collection) = store.get_by_id(&collection_id) else {
                continue;
            };
            let rows = fetch_by_ids(ex, store, ctx, &collection, &ids).await?;
            fetched.insert(collection_id, rows);
        }

        for (key, target) in &targets {
            let nested: Vec<Vec<String>> = groups
                .get(key)
                .map(|paths| paths.iter().filter(|p| !p.is_empty()).cloned().collect())
                .unwrap_or_default();
            match target {
                Target::Forward { field, collection } => {
                    let pool = fetched.get(&collection.id).cloned().unwrap_or_default();
                    attach_forward(ex, store, ctx, records, key, field, pool, nested, depth)
                        .await?;
                }
                Target::Back { collection, field } => {
                    attach_back(
                        ex, store, ctx, records, key, collection, field, nested, depth,
                    )
                    .await?;
                }
            }
        }
        Ok(())
    })
}

/// Union of the ids every forward relation at this level needs, keyed by
/// target collection — so two relation fields pointing at the same
/// collection still cost one query.
fn forward_ids_by_collection(
    records: &[Record],
    targets: &IndexMap<String, Target>,
) -> IndexMap<String, Vec<String>> {
    let mut out: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut seen: HashMap<String, HashSet<String>> = HashMap::new();
    for (key, target) in targets {
        let Target::Forward { collection, .. } = target else {
            continue;
        };
        for record in records {
            for id in record.get_string_list(key) {
                if id.is_empty() {
                    continue;
                }
                if seen
                    .entry(collection.id.clone())
                    .or_default()
                    .insert(id.clone())
                {
                    out.entry(collection.id.clone()).or_default().push(id);
                }
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
async fn attach_forward(
    ex: &dyn Executor,
    store: &CollectionStore,
    ctx: &RequestContext,
    records: &mut [Record],
    key: &str,
    field: &Field,
    mut pool: Vec<Record>,
    nested: Vec<Vec<String>>,
    depth: usize,
) -> DbResult<()> {
    if pool.is_empty() {
        return Ok(());
    }
    if !nested.is_empty() {
        resolve_paths(ex, store, ctx, &mut pool, nested, depth + 1).await?;
    }
    let by_id: HashMap<&str, Value> = pool.iter().map(|r| (r.id(), serialize(r, ctx))).collect();

    for record in records.iter_mut() {
        let wanted = record.get_string_list(key);
        if field.is_multiple() {
            let items: Vec<Value> = wanted
                .iter()
                .filter_map(|id| by_id.get(id.as_str()).cloned())
                .collect();
            if !items.is_empty() {
                record.set_expand(key, Value::Array(items));
            }
        } else if let Some(value) = wanted.first().and_then(|id| by_id.get(id.as_str())) {
            record.set_expand(key, value.clone());
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn attach_back(
    ex: &dyn Executor,
    store: &CollectionStore,
    ctx: &RequestContext,
    records: &mut [Record],
    key: &str,
    collection: &Arc<Collection>,
    field: &Field,
    nested: Vec<Vec<String>>,
    depth: usize,
) -> DbResult<()> {
    let parent_ids: Vec<String> = {
        let mut seen = HashSet::new();
        records
            .iter()
            .map(|r| r.id().to_string())
            .filter(|id| !id.is_empty() && seen.insert(id.clone()))
            .collect()
    };
    if parent_ids.is_empty() {
        return Ok(());
    }
    let mut children = fetch_back(ex, store, ctx, collection, field, &parent_ids).await?;
    if children.is_empty() {
        return Ok(());
    }
    if !nested.is_empty() {
        resolve_paths(ex, store, ctx, &mut children, nested, depth + 1).await?;
    }

    // Index the children by the parent they point at.
    let mut by_parent: HashMap<String, Vec<Value>> = HashMap::new();
    for child in &children {
        for parent in child.get_string_list(&field.name) {
            let bucket = by_parent.entry(parent).or_default();
            if bucket.len() < MAX_BACK_RELATION_ROWS {
                bucket.push(serialize(child, ctx));
            }
        }
    }

    // PocketBase collapses a back-relation to a single object when the
    // referencing column carries a UNIQUE index (there can only ever be
    // one row per parent).
    let single = !field.is_multiple() && has_unique_index(collection, &field.name);
    for record in records.iter_mut() {
        let Some(items) = by_parent.get(record.id()) else {
            continue;
        };
        if single {
            if let Some(first) = items.first() {
                record.set_expand(key, first.clone());
            }
        } else {
            record.set_expand(key, Value::Array(items.clone()));
        }
    }
    Ok(())
}

fn has_unique_index(collection: &Collection, column: &str) -> bool {
    collection.indexes.iter().any(|stmt| {
        schema::parse_index(stmt).is_some_and(|def| {
            def.unique
                && def
                    .columns_raw
                    .split(',')
                    .map(|c| c.trim().trim_matches(|ch| ch == '`' || ch == '"'))
                    .eq(std::iter::once(column))
        })
    })
}

/// Serialize an expanded record. Hidden fields never leave the server;
/// an email is shown to a superuser or to the record's own owner, which
/// is the same test the top-level serializer applies.
fn serialize(record: &Record, ctx: &RequestContext) -> Value {
    let show_email = ctx.is_superuser() || ctx.auth_id() == Some(record.id());
    record.to_json(SerializeOptions {
        with_hidden: false,
        show_email,
        with_custom_data: false,
    })
}

/// One `WHERE id IN (...)` per collection, with its `viewRule` applied.
async fn fetch_by_ids(
    ex: &dyn Executor,
    store: &CollectionStore,
    ctx: &RequestContext,
    collection: &Arc<Collection>,
    ids: &[String],
) -> DbResult<Vec<Record>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let resolver = CollectionResolver::new(collection.clone(), store, ctx, ex.dialect());
    let rule = rules::evaluate(&collection.view_rule, &resolver, 0)?;
    if rule.is_deny_all() {
        return Ok(vec![]);
    }
    let mut query = Query::new(collection);
    if let Some(filter) = rule.into_filter() {
        query.push_filter(filter);
    }
    let start = query.next_placeholder();
    query.push_condition(
        format!(
            "{}.\"id\" IN ({})",
            quote_ident(collection.table_name()),
            in_placeholders(start, ids.len())
        ),
        ids.iter().map(|id| Sql::Text(id.clone())).collect(),
    );

    let sql = query.select_sql();
    query.bind_page(ids.len() as i64, 0);
    let rows = ex.query(&sql, query.params()).await?;
    Ok(decode(collection, &rows))
}

/// Decode a result set with one shared column map.
fn decode(collection: &Arc<Collection>, rows: &[crate::engine::Row]) -> Vec<Record> {
    let Some(first) = rows.first() else {
        return vec![];
    };
    let decoder = RowDecoder::new(collection, &first.columns);
    rows.iter().map(|r| decoder.decode(r)).collect()
}

/// One query for every parent of a back-relation, with the referencing
/// collection's `viewRule` applied.
async fn fetch_back(
    ex: &dyn Executor,
    store: &CollectionStore,
    ctx: &RequestContext,
    collection: &Arc<Collection>,
    field: &Field,
    parent_ids: &[String],
) -> DbResult<Vec<Record>> {
    let resolver = CollectionResolver::new(collection.clone(), store, ctx, ex.dialect());
    let rule = rules::evaluate(&collection.view_rule, &resolver, 0)?;
    if rule.is_deny_all() {
        return Ok(vec![]);
    }
    let mut query = Query::new(collection);
    if let Some(filter) = rule.into_filter() {
        query.push_filter(filter);
    }
    let start = query.next_placeholder();
    let list = in_placeholders(start, parent_ids.len());
    let column = format!(
        "{}.{}",
        quote_ident(collection.table_name()),
        quote_ident(&field.name)
    );
    let condition = if field.is_multiple() {
        let array = crate::records::json_array(&column);
        match ex.dialect() {
            Dialect::Sqlite => {
                format!("EXISTS (SELECT 1 FROM json_each({array}) WHERE \"value\" IN ({list}))")
            }
            Dialect::Postgres => format!(
                "EXISTS (SELECT 1 FROM jsonb_array_elements_text(({array})::jsonb) \
                 AS __e(\"value\") WHERE __e.\"value\" IN ({list}))"
            ),
        }
    } else {
        format!("{column} IN ({list})")
    };
    query.push_condition(
        condition,
        parent_ids.iter().map(|id| Sql::Text(id.clone())).collect(),
    );

    let sql = query.select_sql();
    let cap = (MAX_BACK_RELATION_ROWS * parent_ids.len()) as i64;
    query.bind_page(cap, 0);
    let rows = ex.query(&sql, query.params()).await?;
    Ok(decode(collection, &rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_parsing() {
        assert_eq!(
            parse_spec("author, comments_via_post.author , "),
            vec![
                vec!["author".to_string()],
                vec!["comments_via_post".to_string(), "author".to_string()],
            ]
        );
        assert!(parse_spec("  ").is_empty());
        assert!(parse_spec(",,").is_empty());
    }
}
