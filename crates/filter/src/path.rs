//! Resolution of dotted field paths against the collection schema:
//! `title`, `author.name`, `author.company.name`, `tags.name`,
//! `comments_via_post.title`, `data.some.key`, `loc.lat`, and the same
//! rooted at `@collection.X`.
//!
//! Each path walks the schema one segment at a time:
//!
//! * a plain field ends the walk (its column expression is returned);
//! * a single-valued `relation` field steps into the target collection
//!   through a correlated scalar subquery
//!   `(SELECT "users"."name" FROM "users" WHERE "users"."id" = "posts"."author")`;
//! * a multi-valued `relation` field steps into the target through a
//!   `json_each` element set that the compiler wraps in `EXISTS`;
//! * a `<collection>_via_<field>` segment is a back-relation: rows of
//!   `<collection>` whose relation `<field>` points at the current row.
//!   At the root it becomes a `LEFT JOIN` (deduplicated by alias, so two
//!   references share the same joined row, like PocketBase); inside an
//!   element set it becomes an inner join of that set;
//! * a `json` field consumes the remaining segments as a JSON path, a
//!   `geoPoint` field accepts `lon`/`lat`.
//!
//! Every column reference is fully qualified with its table or alias,
//! since the surrounding query may carry joins.

use std::sync::Arc;

use cratebase_core::{Collection, FieldKind, FieldType};

use crate::error::FilterError;
use crate::resolver::{Dialect, Resolver};

/// Maximum number of relation hops in one path (PocketBase's
/// `maxNestedRels`).
pub const MAX_DEPTH: usize = 6;

/// A `LEFT JOIN` the surrounding query must include for the compiled SQL
/// to be valid. The query builder deduplicates joins by `key` and adds
/// `SELECT DISTINCT` whenever any join is present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Join {
    /// The full `LEFT JOIN "table" AS "alias" ON ...` fragment.
    pub sql: String,
    /// Deduplication key (the join alias).
    pub key: String,
}

/// The SQL type a resolved expression produces, used to keep Postgres'
/// strict typing happy (e.g. no `numeric = ''`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlType {
    Text,
    Number,
    Bool,
    /// A value extracted from a JSON document (text in Postgres, typed in
    /// SQLite).
    Json,
}

impl SqlType {
    fn of(t: FieldType) -> SqlType {
        match t {
            FieldType::Number => SqlType::Number,
            FieldType::Bool => SqlType::Bool,
            _ => SqlType::Text,
        }
    }
}

/// A resolved path.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldRef {
    /// SQL expression for the value, relative to `from` when present.
    pub sql: String,
    pub sql_type: SqlType,
    /// Whether the terminal field stores a JSON array (multi-valued
    /// select/file/relation).
    pub multi: bool,
    /// Set when the path went through a multi-valued relation: the FROM
    /// clause enumerating the related rows that `sql` refers to.
    pub from: Option<String>,
    /// Whether the path went through a root-level `LEFT JOIN` (a
    /// back-relation or `@collection.X`), i.e. `sql` can stand for several
    /// rows. Bare operators over such a path mean *every* row (see
    /// [`PathResolver::resolve_multi_match`]).
    pub joined: bool,
}

/// Prefix PocketBase gives the aliases of the correlated copy of a path it
/// builds for a multi-match subquery.
const MM_PREFIX: &str = "__mm_";

/// Whether a segment has the `<collection>_via_<field>` shape.
pub fn is_back_relation(segment: &str) -> bool {
    segment
        .split_once("_via_")
        .is_some_and(|(c, f)| !c.is_empty() && !f.is_empty())
}

pub fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// The current row while walking a path.
enum Row {
    /// A table or alias in scope: columns are `"alias"."col"`.
    Table { alias: String },
    /// A row addressed by id: columns are scalar subqueries.
    ById { key: String },
}

/// The collection under the cursor: the root is borrowed from the
/// resolver, everything else is a shared handle from the index.
enum Coll<'a> {
    Root(&'a Collection),
    Shared(Arc<Collection>),
}

impl Coll<'_> {
    fn get(&self) -> &Collection {
        match self {
            Coll::Root(c) => c,
            Coll::Shared(c) => c,
        }
    }
}

struct Cursor<'a> {
    collection: Coll<'a>,
    row: Row,
    /// Deterministic prefix for join aliases (`posts`, `posts_author`, ...).
    name: String,
}

impl Cursor<'_> {
    fn column(&self, field: &str) -> String {
        match &self.row {
            Row::Table { alias } => format!("{}.{}", quote(alias), quote(field)),
            Row::ById { key } => {
                let t = quote(self.collection.get().table_name());
                format!(
                    "(SELECT {t}.{f} FROM {t} WHERE {t}.\"id\" = {key})",
                    f = quote(field)
                )
            }
        }
    }

    fn key(&self) -> String {
        match &self.row {
            Row::Table { alias } => format!("{}.\"id\"", quote(alias)),
            Row::ById { key } => key.clone(),
        }
    }
}

/// Resolves paths for one compilation, owning the alias counter and the
/// collected joins.
pub struct PathResolver<'a> {
    resolver: &'a dyn Resolver,
    dialect: Dialect,
    pub joins: Vec<Join>,
    alias_seq: usize,
    /// While `Some`, the resolver is building the correlated `__mm_` copy of
    /// a path: root-level joins are appended here instead of to `joins`, and
    /// every root alias is prefixed, so the copy can live inside a
    /// subquery next to the original.
    mm: Option<String>,
}

/// The correlated copy of a path that went through a root-level join,
/// used to express "*every* joined row satisfies the comparison".
#[derive(Debug, Clone, PartialEq)]
pub struct MultiMatchRef {
    /// `"posts" AS "__mm_posts" LEFT JOIN "comments" AS "__mm_..." ON ...`,
    /// including any element sets the path needed.
    pub from: String,
    /// `"__mm_posts"."id" = "posts"."id"`, tying the copy to the outer row.
    pub correlation: String,
    /// The resolved path relative to `from`.
    pub field: FieldRef,
}

impl<'a> PathResolver<'a> {
    pub fn new(resolver: &'a dyn Resolver) -> Self {
        PathResolver {
            resolver,
            dialect: resolver.dialect(),
            joins: Vec::new(),
            alias_seq: 0,
            mm: None,
        }
    }

    pub(crate) fn next_alias(&mut self, prefix: &str) -> String {
        self.alias_seq += 1;
        format!("__{prefix}{}", self.alias_seq)
    }

    /// Root-level alias for `base`, prefixed while building a `__mm_` copy.
    fn root_alias(&self, base: &str) -> String {
        match self.mm {
            Some(_) => format!("{MM_PREFIX}{base}"),
            None => base.to_string(),
        }
    }

    fn add_join(&mut self, join: Join) {
        if let Some(mm) = &mut self.mm {
            mm.push(' ');
            mm.push_str(&join.sql);
            return;
        }
        if !self.joins.iter().any(|j| j.key == join.key) {
            self.joins.push(join);
        }
    }

    /// `FROM`-fragment enumerating the elements of a JSON array
    /// expression under `alias`, and the element reference.
    pub fn elements(&self, array_expr: &str, alias: &str) -> (String, String) {
        let a = quote(alias);
        let from = match self.dialect {
            Dialect::Sqlite => format!("json_each(COALESCE({array_expr}, '[]')) AS {a}"),
            Dialect::Postgres => format!(
                "jsonb_array_elements_text(COALESCE({array_expr}, '[]')::jsonb) AS {a}(\"value\")"
            ),
        };
        (from, format!("{a}.\"value\""))
    }

    /// `json_array_length` of an array column.
    pub fn array_length(&self, array_expr: &str) -> String {
        match self.dialect {
            Dialect::Sqlite => format!("json_array_length(COALESCE({array_expr}, '[]'))"),
            Dialect::Postgres => {
                format!("jsonb_array_length(COALESCE({array_expr}, '[]')::jsonb)")
            }
        }
    }

    /// SQL testing whether an array column holds no elements.
    pub fn array_empty(&self, array_expr: &str) -> String {
        match self.dialect {
            Dialect::Sqlite => {
                format!("({array_expr} IS NULL OR {array_expr} = '' OR {array_expr} = '[]')")
            }
            Dialect::Postgres => format!(
                "({array_expr} IS NULL OR {array_expr}::text = '' OR {array_expr}::text = '[]')"
            ),
        }
    }

    fn json_extract(&self, col: &str, segments: &[&str]) -> String {
        match self.dialect {
            Dialect::Sqlite => {
                let mut path = String::from("$");
                for seg in segments {
                    if seg.bytes().all(|b| b.is_ascii_digit()) {
                        path.push_str(&format!("[{seg}]"));
                    } else {
                        path.push('.');
                        path.push_str(seg);
                    }
                }
                format!("json_extract({col}, '{path}')")
            }
            Dialect::Postgres => format!("({col}::jsonb #>> '{{{}}}')", segments.join(",")),
        }
    }

    /// The join condition for back-relation `back_alias.field` → current
    /// row key, honoring single/multi storage of the relation field.
    fn back_relation_on(
        &mut self,
        back_alias: &str,
        field: &str,
        multi: bool,
        key: &str,
    ) -> String {
        let col = format!("{}.{}", quote(back_alias), quote(field));
        if multi {
            let e = self.next_alias("e");
            let (from, elem) = self.elements(&col, &e);
            format!("EXISTS (SELECT 1 FROM {from} WHERE {elem} = {key})")
        } else {
            format!("{col} = {key}")
        }
    }

    /// Resolve a path rooted at the filter's collection.
    pub fn resolve(&mut self, path: &str) -> Result<FieldRef, FilterError> {
        let root = self.resolver.root();
        let alias = self.root_alias(root.table_name());
        let cursor = Cursor {
            collection: Coll::Root(root),
            row: Row::Table {
                alias: alias.clone(),
            },
            name: alias,
        };
        self.walk(cursor, path, path, None, false)
    }

    /// Resolve `@collection.<name>.<path>`: a `LEFT JOIN "<name>" AS
    /// "__collection_<name>" ON 1=1` plus the path relative to it.
    pub fn resolve_collection(&mut self, name: &str, path: &str) -> Result<FieldRef, FilterError> {
        let full = format!("@collection.{name}.{path}");
        let Some(collection) = self.resolver.collection(name) else {
            return Err(FilterError::UnknownField(full));
        };
        let alias = self.root_alias(&format!("__collection_{}", collection.table_name()));
        self.add_join(Join {
            sql: format!(
                "LEFT JOIN {} AS {} ON 1=1",
                quote(collection.table_name()),
                quote(&alias)
            ),
            key: alias.clone(),
        });
        let cursor = Cursor {
            collection: Coll::Shared(collection),
            row: Row::Table {
                alias: alias.clone(),
            },
            name: alias,
        };
        self.walk(cursor, path, &full, None, true)
    }

    /// Resolve `@request.body.<relation>.<rest>` (also the deprecated
    /// `@request.data.` alias): `key_sql` is the already-bound placeholder
    /// for the relation field's *submitted* value — a single id, or (when
    /// `multi`) a JSON array of ids. The walk starts addressed by that
    /// value rather than a column on the root row, so `updateRule` sees
    /// the newly submitted related row, not whatever the record currently
    /// points at. Unlike `resolve_collection` this never joins the root
    /// query: the value is a bound literal, independent of the outer row.
    pub fn resolve_body_relation(
        &mut self,
        key_sql: String,
        multi: bool,
        target: Arc<Collection>,
        rest: &str,
        full_path: &str,
    ) -> Result<FieldRef, FilterError> {
        if rest.is_empty() {
            return Err(FilterError::UnknownField(full_path.to_string()));
        }
        let name = "__body".to_string();
        if multi {
            let e = self.next_alias("e");
            let r = self.next_alias("r");
            let (elems, elem) = self.elements(&key_sql, &e);
            let piece = format!(
                "{elems} JOIN {} AS {} ON {}.\"id\" = {elem}",
                quote(target.table_name()),
                quote(&r),
                quote(&r)
            );
            let cursor = Cursor {
                collection: Coll::Shared(target),
                row: Row::Table { alias: r },
                name,
            };
            self.walk(cursor, rest, full_path, Some(piece), false)
        } else {
            let cursor = Cursor {
                collection: Coll::Shared(target),
                row: Row::ById { key: key_sql },
                name,
            };
            self.walk(cursor, rest, full_path, None, false)
        }
    }

    /// Resolve the same path a second time under `__mm_`-prefixed aliases,
    /// correlated to the outer row. PocketBase pairs a bare comparison over
    /// a joined path with a `NOT EXISTS` over this copy so the comparison
    /// means "and no joined row fails it" rather than "some joined row
    /// passes it". `self.joins` is left untouched.
    ///
    /// `collection` is the `@collection.<name>` prefix, if the path had one.
    pub fn resolve_multi_match(
        &mut self,
        collection: Option<&str>,
        path: &str,
    ) -> Result<MultiMatchRef, FilterError> {
        let table = self.resolver.root().table_name().to_string();
        let alias = format!("{MM_PREFIX}{table}");
        let outer = self
            .mm
            .replace(format!("{} AS {}", quote(&table), quote(&alias)));
        let field = match collection {
            Some(name) => self.resolve_collection(name, path),
            None => self.resolve(path),
        };
        let from = std::mem::replace(&mut self.mm, outer).unwrap_or_default();
        Ok(MultiMatchRef {
            from,
            correlation: format!("{}.\"id\" = {}.\"id\"", quote(&alias), quote(&table)),
            field: field?,
        })
    }

    fn walk<'c>(
        &mut self,
        mut cursor: Cursor<'c>,
        path: &str,
        full_path: &str,
        mut from: Option<String>,
        mut joined: bool,
    ) -> Result<FieldRef, FilterError> {
        let segments: Vec<&str> = path.split('.').collect();
        let unknown = || FilterError::UnknownField(full_path.to_string());
        let mut hops = 0usize;

        for (i, seg) in segments.iter().enumerate() {
            let last = i + 1 == segments.len();
            let field = cursor.collection.get().field(seg).cloned();

            let Some(field) = field else {
                // Back-relation `<collection>_via_<field>`.
                let Some((back_name, back_field)) = seg.split_once("_via_") else {
                    return Err(unknown());
                };
                if last {
                    return Err(unknown());
                }
                let back = self.resolver.collection(back_name).ok_or_else(unknown)?;
                let rel = back.field(back_field).ok_or_else(unknown)?;
                let points_here = matches!(
                    &rel.kind,
                    FieldKind::Relation { collection_id, .. }
                        if *collection_id == cursor.collection.get().id
                );
                if !points_here {
                    return Err(unknown());
                }
                hops += 1;
                if hops > MAX_DEPTH {
                    return Err(FilterError::Unsupported(format!(
                        "'{full_path}' exceeds the maximum relation depth of {MAX_DEPTH}"
                    )));
                }
                let key = cursor.key();
                let multi = rel.is_multiple();
                let table = quote(back.table_name());
                let alias = match &from {
                    None => format!("{}_{}", cursor.name, seg),
                    Some(_) => self.next_alias("b"),
                };
                let on = self.back_relation_on(&alias, back_field, multi, &key);
                match &mut from {
                    None => {
                        joined = true;
                        self.add_join(Join {
                            sql: format!("LEFT JOIN {table} AS {} ON {on}", quote(&alias)),
                            key: alias.clone(),
                        })
                    }
                    Some(f) => {
                        f.push_str(&format!(" JOIN {table} AS {} ON {on}", quote(&alias)));
                    }
                }
                cursor = Cursor {
                    collection: Coll::Shared(back),
                    row: Row::Table {
                        alias: alias.clone(),
                    },
                    name: alias,
                };
                continue;
            };

            let col = cursor.column(seg);
            if last {
                return Ok(match field.field_type() {
                    FieldType::Json => FieldRef {
                        sql: self.json_root(&col),
                        sql_type: SqlType::Json,
                        multi: false,
                        from,
                        joined,
                    },
                    t => FieldRef {
                        sql: col,
                        sql_type: SqlType::of(t),
                        multi: field.is_multiple(),
                        from,
                        joined,
                    },
                });
            }

            let rest = &segments[i + 1..];
            match &field.kind {
                FieldKind::Relation { collection_id, .. } => {
                    hops += 1;
                    if hops > MAX_DEPTH {
                        return Err(FilterError::Unsupported(format!(
                            "'{full_path}' exceeds the maximum relation depth of {MAX_DEPTH}"
                        )));
                    }
                    let target = self
                        .resolver
                        .collection(collection_id)
                        .ok_or_else(unknown)?;
                    let name = format!("{}_{}", cursor.name, seg);
                    if field.is_multiple() {
                        let e = self.next_alias("e");
                        let r = self.next_alias("r");
                        let (elems, elem) = self.elements(&col, &e);
                        let piece = format!(
                            "{elems} JOIN {} AS {} ON {}.\"id\" = {elem}",
                            quote(target.table_name()),
                            quote(&r),
                            quote(&r)
                        );
                        from = Some(match from {
                            None => piece,
                            Some(f) => format!("{f}, {piece}"),
                        });
                        cursor = Cursor {
                            collection: Coll::Shared(target),
                            row: Row::Table { alias: r },
                            name,
                        };
                    } else {
                        cursor = Cursor {
                            collection: Coll::Shared(target),
                            row: Row::ById { key: col },
                            name,
                        };
                    }
                }
                FieldKind::Json { .. } => {
                    return Ok(FieldRef {
                        sql: self.json_extract(&col, rest),
                        sql_type: SqlType::Json,
                        multi: false,
                        from,
                        joined,
                    });
                }
                FieldKind::GeoPoint {} => {
                    if rest.len() != 1 || !matches!(rest[0], "lon" | "lat") {
                        return Err(unknown());
                    }
                    let sql = match self.dialect {
                        Dialect::Sqlite => self.json_extract(&col, rest),
                        Dialect::Postgres => {
                            format!("({col}::jsonb->>'{}')::double precision", rest[0])
                        }
                    };
                    return Ok(FieldRef {
                        sql,
                        sql_type: SqlType::Number,
                        multi: false,
                        from,
                        joined,
                    });
                }
                _ => return Err(unknown()),
            }
        }
        // `path` was empty (e.g. `@collection.X.` handled earlier).
        Err(unknown())
    }

    /// Whole json fields compare their decoded value, like PocketBase's
    /// `JSON_EXTRACT(col, '$')`.
    pub fn json_root(&self, col: &str) -> String {
        match self.dialect {
            Dialect::Sqlite => format!("json_extract({col}, '$')"),
            Dialect::Postgres => format!("({col}::jsonb #>> '{{}}')"),
        }
    }
}
