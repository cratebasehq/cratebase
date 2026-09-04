//! Compilation of a parsed [`Expr`] into a parameterized SQL fragment.
//!
//! The output mirrors PocketBase's `search.FilterData` semantics:
//!
//! * Every root column is fully qualified (`"posts"."title"`) so the
//!   fragment stays valid once joins are added.
//! * `@request.*` values and the date macros are bound as `$n`
//!   parameters (numbered from `param_offset + 1`); comparisons between
//!   two bound values are constant-folded to `1 = 1` / `1 = 0`.
//! * `= ""`/`= null` and their negations use `IS NULL`-aware SQL, `!=`
//!   against a value also matches `NULL` columns (PocketBase compiles
//!   `!=` as `IS NOT`).
//! * `~`/`!~` are `LIKE` (SQLite, case-insensitive for ASCII) or `ILIKE`
//!   (Postgres); a pattern without `%` is wrapped in `%...%`.
//! * Multi-valued operands (multi select/file/relation fields, paths
//!   through multi relations, `@request.body.x:each`) compile to
//!   `EXISTS`/`NOT EXISTS` over their elements: `?op` = any element,
//!   bare op = every element (plus at least one, unless the operator is
//!   satisfied by the missing row: `!=`, or `= ""`).
//! * Back-relations and `@collection.X` produce [`Join`]s the query
//!   builder appends (deduplicated by key, with `SELECT DISTINCT`).
//! * `geoDistance(lonA, latA, lonB, latB)` is emitted as a call to the
//!   `geoDistance` SQL function on SQLite (the engine must register it;
//!   haversine, kilometers) and as an inline haversine on Postgres.

use serde_json::Value;

use crate::ast::{CompareOp, Expr, Modifier, Operand};
use crate::error::FilterError;
use crate::eval::{self, is_empty, like_pattern, lowercase, to_text, EvalTerm};
use crate::path::{FieldRef, Join, PathResolver, SqlType};
use crate::resolver::{Dialect, Resolver};
use crate::terms::{literal_value, macro_value, number_value, MacroTerm};

/// Result of compiling a filter expression.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledFilter {
    /// A parameterized boolean SQL expression using `$n` placeholders.
    pub sql: String,
    /// Bound parameter values, in placeholder order.
    pub params: Vec<Value>,
    /// Joins the surrounding query must include (see [`Join`]).
    pub joins: Vec<Join>,
}

/// A resolved operand.
#[derive(Debug, Clone)]
enum Term {
    /// A value known at compile time.
    Value { value: Value, each: bool },
    /// A scalar SQL expression.
    Scalar { sql: String, ty: SqlType },
    /// A set of elements: `elem` is valid inside `SELECT ... FROM {from}`;
    /// `empty` tests for "no elements"; `array` is the raw array column
    /// when the set is a root-level multi-valued field.
    Multi {
        from: String,
        elem: String,
        ty: SqlType,
        empty: String,
        array: Option<String>,
    },
}

/// One side of a scalar comparison.
enum Side {
    Sql(String, SqlType),
    Val(Value),
}

struct Compiler<'a> {
    paths: PathResolver<'a>,
    resolver: &'a dyn Resolver,
    dialect: Dialect,
    offset: usize,
    params: Vec<Value>,
}

impl<'a> Compiler<'a> {
    fn new(resolver: &'a dyn Resolver, offset: usize) -> Self {
        Compiler {
            paths: PathResolver::new(resolver),
            resolver,
            dialect: resolver.dialect(),
            offset,
            params: Vec::new(),
        }
    }

    fn push_param(&mut self, v: Value) -> String {
        let v = match v {
            // Structured values travel as JSON text.
            Value::Array(_) | Value::Object(_) => Value::String(v.to_string()),
            other => other,
        };
        self.params.push(v);
        format!("${}", self.offset + self.params.len())
    }

    // --- operands ----------------------------------------------------------

    fn resolve_operand(&mut self, operand: &Operand) -> Result<Term, FilterError> {
        match operand {
            Operand::Literal(lit) => Ok(Term::Value {
                value: literal_value(lit),
                each: false,
            }),
            Operand::Call { name, args } => self.resolve_call(name, args),
            Operand::Ident { path, modifier } => {
                if path.starts_with('@') {
                    return match macro_value(path, *modifier, self.resolver)? {
                        MacroTerm::Value { value, each } => Ok(Term::Value { value, each }),
                        MacroTerm::Collection {
                            collection,
                            path: rel,
                        } => {
                            let r = self.paths.resolve_collection(&collection, &rel)?;
                            self.field_term(r, *modifier, path)
                        }
                    };
                }
                let r = self.paths.resolve(path)?;
                self.field_term(r, *modifier, path)
            }
        }
    }

    /// Apply the modifier and the multi-valued expansion to a resolved
    /// path.
    fn field_term(
        &mut self,
        r: FieldRef,
        modifier: Option<Modifier>,
        path: &str,
    ) -> Result<Term, FilterError> {
        let FieldRef {
            mut sql,
            mut sql_type,
            mut multi,
            from,
        } = r;
        let mut lower = false;
        match modifier {
            None => {}
            Some(Modifier::IsSet) => {
                return Err(FilterError::InvalidModifier(format!(
                    "{path}:isset is only valid on @request.body fields"
                )))
            }
            Some(Modifier::Length) => {
                if !multi {
                    return Err(FilterError::InvalidModifier(format!(
                        "{path}:length requires a multi-valued field"
                    )));
                }
                sql = self.paths.array_length(&sql);
                sql_type = SqlType::Number;
                multi = false;
            }
            Some(Modifier::Each) => {
                if !multi {
                    return Err(FilterError::InvalidModifier(format!(
                        "{path}:each requires a multi-valued field"
                    )));
                }
            }
            Some(Modifier::Lower) => lower = true,
        }
        let lowered = |s: String| if lower { format!("LOWER({s})") } else { s };

        if multi {
            let alias = self.paths.next_alias("e");
            let (elems, elem) = self.paths.elements(&sql, &alias);
            let (from, empty, array) = match from {
                None => (elems, self.paths.array_empty(&sql), Some(sql)),
                Some(f) => {
                    let from = format!("{f}, {elems}");
                    let empty = format!("NOT EXISTS (SELECT 1 FROM {from})");
                    (from, empty, None)
                }
            };
            return Ok(Term::Multi {
                from,
                elem: lowered(elem),
                ty: SqlType::Text,
                empty,
                array,
            });
        }
        match from {
            Some(from) => Ok(Term::Multi {
                empty: format!("NOT EXISTS (SELECT 1 FROM {from})"),
                from,
                elem: lowered(sql),
                ty: sql_type,
                array: None,
            }),
            None => Ok(Term::Scalar {
                sql: lowered(sql),
                ty: sql_type,
            }),
        }
    }

    fn resolve_call(&mut self, name: &str, args: &[Operand]) -> Result<Term, FilterError> {
        match name {
            "geoDistance" => {
                let mut parts = Vec::with_capacity(4);
                for arg in args {
                    let sql = match self.resolve_operand(arg)? {
                        Term::Value { value, .. } => {
                            let v = match &value {
                                Value::Number(_) => value,
                                Value::String(s) => {
                                    s.trim().parse::<f64>().map(number_value).unwrap_or(value)
                                }
                                _ => value,
                            };
                            self.push_param(v)
                        }
                        Term::Scalar { sql, .. } => sql,
                        Term::Multi { .. } => {
                            return Err(FilterError::Unsupported(
                                "geoDistance arguments must be scalar".into(),
                            ))
                        }
                    };
                    parts.push(sql);
                }
                let (lon_a, lat_a, lon_b, lat_b) = (&parts[0], &parts[1], &parts[2], &parts[3]);
                let sql = match self.dialect {
                    Dialect::Sqlite => format!("geoDistance({lon_a}, {lat_a}, {lon_b}, {lat_b})"),
                    Dialect::Postgres => format!(
                        "(6371 * acos(LEAST(1.0, GREATEST(-1.0, \
                         sin(radians({lat_a})) * sin(radians({lat_b})) + \
                         cos(radians({lat_a})) * cos(radians({lat_b})) * \
                         cos(radians({lon_b}) - radians({lon_a}))))))"
                    ),
                };
                Ok(Term::Scalar {
                    sql,
                    ty: SqlType::Number,
                })
            }
            other => Err(FilterError::Parse(format!("unknown function '{other}'"))),
        }
    }

    /// Turn a `:each` value into an element set by binding it as JSON.
    fn value_as_multi(&mut self, value: Value) -> Term {
        let items = match EvalTerm::each(value) {
            EvalTerm::Multi(items) => items,
            EvalTerm::Scalar(v) => vec![v],
        };
        let param = self.push_param(Value::String(Value::Array(items).to_string()));
        let alias = self.paths.next_alias("e");
        let (from, elem) = self.paths.elements(&param, &alias);
        Term::Multi {
            empty: format!("NOT EXISTS (SELECT 1 FROM {from})"),
            from,
            elem,
            ty: SqlType::Text,
            array: None,
        }
    }

    // --- comparisons -------------------------------------------------------

    fn sql_op(op: CompareOp) -> &'static str {
        match op {
            CompareOp::Eq => "=",
            CompareOp::NotEq => "<>",
            CompareOp::Gt => ">",
            CompareOp::Gte => ">=",
            CompareOp::Lt => "<",
            CompareOp::Lte => "<=",
            CompareOp::Like | CompareOp::NotLike => unreachable!("LIKE handled separately"),
        }
    }

    /// `expr = ''`-style emptiness test respecting the column type.
    fn empty_check(e: &str, ty: SqlType, is_eq: bool) -> String {
        match (ty, is_eq) {
            (SqlType::Number | SqlType::Bool, true) => format!("{e} IS NULL"),
            (SqlType::Number | SqlType::Bool, false) => format!("{e} IS NOT NULL"),
            (_, true) => format!("({e} = '' OR {e} IS NULL)"),
            (_, false) => format!("({e} <> '' AND {e} IS NOT NULL)"),
        }
    }

    /// Adapt a SQL expression and the value it is compared with so the
    /// comparison type-checks on Postgres (JSON paths are text there).
    fn coerce(&self, e: String, ty: SqlType, v: Value) -> (String, Value) {
        if self.dialect == Dialect::Postgres && ty == SqlType::Json {
            return match v {
                Value::Number(_) => (format!("CAST({e} AS double precision)"), v),
                Value::Bool(b) => (e, Value::String(b.to_string())),
                other => (e, other),
            };
        }
        (e, v)
    }

    fn null_safe_eq(&self, a: &str, b: &str, is_eq: bool) -> String {
        match (self.dialect, is_eq) {
            (Dialect::Sqlite, true) => format!("{a} IS {b}"),
            (Dialect::Sqlite, false) => format!("{a} IS NOT {b}"),
            (Dialect::Postgres, true) => format!("{a} IS NOT DISTINCT FROM {b}"),
            (Dialect::Postgres, false) => format!("{a} IS DISTINCT FROM {b}"),
        }
    }

    /// SQL for a scalar comparison between two sides.
    fn build_cond(&mut self, l: Side, op: CompareOp, r: Side) -> String {
        match op {
            CompareOp::Eq | CompareOp::NotEq => {
                let is_eq = op == CompareOp::Eq;
                match (l, r) {
                    (Side::Val(a), Side::Val(b)) => bool_sql(eval::compare_scalars(&a, op, &b)),
                    (Side::Sql(e, ty), Side::Val(v)) | (Side::Val(v), Side::Sql(e, ty))
                        if is_empty(&v) =>
                    {
                        Self::empty_check(&e, ty, is_eq)
                    }
                    (Side::Sql(a, _), Side::Sql(b, _)) => self.null_safe_eq(&a, &b, is_eq),
                    (Side::Sql(e, ty), Side::Val(v)) => {
                        let (e, v) = self.coerce(e, ty, v);
                        let p = self.push_param(v);
                        if is_eq {
                            format!("{e} = {p}")
                        } else {
                            format!("({e} <> {p} OR {e} IS NULL)")
                        }
                    }
                    (Side::Val(v), Side::Sql(e, ty)) => {
                        let (e, v) = self.coerce(e, ty, v);
                        let p = self.push_param(v);
                        if is_eq {
                            format!("{p} = {e}")
                        } else {
                            format!("({p} <> {e} OR {e} IS NULL)")
                        }
                    }
                }
            }
            CompareOp::Gt | CompareOp::Gte | CompareOp::Lt | CompareOp::Lte => {
                let sql_op = Self::sql_op(op);
                match (l, r) {
                    (Side::Val(a), Side::Val(b)) => bool_sql(eval::compare_scalars(&a, op, &b)),
                    (Side::Sql(a, _), Side::Sql(b, _)) => format!("{a} {sql_op} {b}"),
                    (Side::Sql(e, ty), Side::Val(v)) => {
                        let (e, v) = self.coerce(e, ty, v);
                        let p = self.push_param(v);
                        format!("{e} {sql_op} {p}")
                    }
                    (Side::Val(v), Side::Sql(e, ty)) => {
                        let (e, v) = self.coerce(e, ty, v);
                        let p = self.push_param(v);
                        format!("{p} {sql_op} {e}")
                    }
                }
            }
            CompareOp::Like | CompareOp::NotLike => {
                if let (Side::Val(a), Side::Val(b)) = (&l, &r) {
                    return bool_sql(eval::compare_scalars(a, op, b));
                }
                let subject = match l {
                    Side::Sql(s, _) => s,
                    Side::Val(v) => self.push_param(Value::String(to_text(&v))),
                };
                let pattern = match r {
                    Side::Sql(s, _) => format!("('%' || {s} || '%')"),
                    Side::Val(v) => self.push_param(Value::String(like_pattern(&to_text(&v)))),
                };
                let kw = match (self.dialect, op) {
                    (Dialect::Sqlite, CompareOp::Like) => "LIKE",
                    (Dialect::Sqlite, _) => "NOT LIKE",
                    (Dialect::Postgres, CompareOp::Like) => "ILIKE",
                    (Dialect::Postgres, _) => "NOT ILIKE",
                };
                format!("{subject} {kw} {pattern}")
            }
        }
    }

    /// Wrap a per-element condition in the any-of / all-of quantifiers.
    fn multi_cond(
        &self,
        from: &str,
        empty: &str,
        cond: &str,
        any_of: bool,
        null_ok: bool,
    ) -> String {
        match (any_of, null_ok) {
            (true, true) => format!("({empty} OR EXISTS (SELECT 1 FROM {from} WHERE {cond}))"),
            (true, false) => format!("EXISTS (SELECT 1 FROM {from} WHERE {cond})"),
            (false, true) => format!("NOT EXISTS (SELECT 1 FROM {from} WHERE NOT ({cond}))"),
            (false, false) => format!(
                "(EXISTS (SELECT 1 FROM {from} WHERE {cond}) AND NOT EXISTS (SELECT 1 FROM {from} WHERE NOT ({cond})))"
            ),
        }
    }

    fn compile_compare(
        &mut self,
        left: &Operand,
        op: CompareOp,
        any_of: bool,
        right: &Operand,
    ) -> Result<String, FilterError> {
        let mut l = self.resolve_operand(left)?;
        let mut r = self.resolve_operand(right)?;

        // `:lower` on one side lowercases bound strings on the other.
        if has_modifier(left, Modifier::Lower) {
            if let Term::Value { value, .. } = &mut r {
                *value = lowercase(std::mem::take(value));
            }
        }
        if has_modifier(right, Modifier::Lower) {
            if let Term::Value { value, .. } = &mut l {
                *value = lowercase(std::mem::take(value));
            }
        }

        // Two bound values: fold at compile time.
        if let (Term::Value { value: a, each: ea }, Term::Value { value: b, each: eb }) = (&l, &r) {
            let a = value_term(a.clone(), *ea);
            let b = value_term(b.clone(), *eb);
            return Ok(bool_sql(eval::compare_terms(&a, op, any_of, &b)?));
        }
        // `:each` values against columns become element sets.
        if let Term::Value { value, each: true } = &l {
            l = self.value_as_multi(value.clone());
        }
        if let Term::Value { value, each: true } = &r {
            r = self.value_as_multi(value.clone());
        }

        match (l, r) {
            (Term::Value { .. }, Term::Value { .. }) => unreachable!("folded above"),
            (Term::Scalar { sql, ty }, Term::Value { value, .. }) => {
                Ok(self.build_cond(Side::Sql(sql, ty), op, Side::Val(value)))
            }
            (Term::Value { value, .. }, Term::Scalar { sql, ty }) => {
                Ok(self.build_cond(Side::Val(value), op, Side::Sql(sql, ty)))
            }
            (Term::Scalar { sql: a, ty: ta }, Term::Scalar { sql: b, ty: tb }) => {
                Ok(self.build_cond(Side::Sql(a, ta), op, Side::Sql(b, tb)))
            }
            (
                Term::Multi {
                    from,
                    elem,
                    ty,
                    empty,
                    array,
                },
                Term::Value { value, .. },
            ) => {
                if let Some(sql) = self.multi_empty_shortcut(op, &value, array.as_deref()) {
                    return Ok(sql);
                }
                let null_ok = eval::null_tolerant(op, Some(&value));
                let cond = self.build_cond(Side::Sql(elem, ty), op, Side::Val(value));
                Ok(self.multi_cond(&from, &empty, &cond, any_of, null_ok))
            }
            (
                Term::Value { value, .. },
                Term::Multi {
                    from,
                    elem,
                    ty,
                    empty,
                    array,
                },
            ) => {
                if let Some(sql) = self.multi_empty_shortcut(op, &value, array.as_deref()) {
                    return Ok(sql);
                }
                let null_ok = eval::null_tolerant(op, Some(&value));
                let cond = self.build_cond(Side::Val(value), op, Side::Sql(elem, ty));
                Ok(self.multi_cond(&from, &empty, &cond, any_of, null_ok))
            }
            (
                Term::Multi {
                    from,
                    elem,
                    ty,
                    empty,
                    ..
                },
                Term::Scalar { sql, ty: sty },
            ) => {
                let null_ok = eval::null_tolerant(op, None);
                let cond = self.build_cond(Side::Sql(elem, ty), op, Side::Sql(sql, sty));
                Ok(self.multi_cond(&from, &empty, &cond, any_of, null_ok))
            }
            (
                Term::Scalar { sql, ty: sty },
                Term::Multi {
                    from,
                    elem,
                    ty,
                    empty,
                    ..
                },
            ) => {
                let null_ok = eval::null_tolerant(op, None);
                let cond = self.build_cond(Side::Sql(sql, sty), op, Side::Sql(elem, ty));
                Ok(self.multi_cond(&from, &empty, &cond, any_of, null_ok))
            }
            (Term::Multi { .. }, Term::Multi { .. }) => Err(FilterError::Unsupported(
                "comparing two multi-valued operands".into(),
            )),
        }
    }

    /// `tags = ""` / `tags != ""` on a root-level multi-valued field is
    /// simply an emptiness test on the stored array.
    fn multi_empty_shortcut(
        &self,
        op: CompareOp,
        value: &Value,
        array: Option<&str>,
    ) -> Option<String> {
        let array = array?;
        if !is_empty(value) {
            return None;
        }
        match op {
            CompareOp::Eq => Some(self.paths.array_empty(array)),
            CompareOp::NotEq => Some(format!("NOT {}", self.paths.array_empty(array))),
            _ => None,
        }
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<String, FilterError> {
        match expr {
            Expr::And(l, r) => {
                let l = self.compile_expr(l)?;
                let r = self.compile_expr(r)?;
                Ok(format!("({l} AND {r})"))
            }
            Expr::Or(l, r) => {
                let l = self.compile_expr(l)?;
                let r = self.compile_expr(r)?;
                Ok(format!("({l} OR {r})"))
            }
            Expr::Compare {
                left,
                op,
                any_of,
                right,
            } => self.compile_compare(left, *op, *any_of, right),
        }
    }
}

fn bool_sql(b: bool) -> String {
    if b { "1 = 1" } else { "1 = 0" }.to_string()
}

fn has_modifier(op: &Operand, m: Modifier) -> bool {
    matches!(op, Operand::Ident { modifier: Some(x), .. } if *x == m)
}

fn value_term(value: Value, each: bool) -> EvalTerm {
    if each {
        EvalTerm::each(value)
    } else {
        EvalTerm::Scalar(value)
    }
}

/// Compile a parsed filter into a parameterized SQL fragment.
/// `param_offset` reserves leading placeholder slots (the first parameter
/// becomes `$<param_offset + 1>`).
pub fn compile(
    expr: &Expr,
    resolver: &dyn Resolver,
    param_offset: usize,
) -> Result<CompiledFilter, FilterError> {
    let mut compiler = Compiler::new(resolver, param_offset);
    let sql = compiler.compile_expr(expr)?;
    Ok(CompiledFilter {
        sql,
        params: compiler.params,
        joins: compiler.paths.joins,
    })
}

/// Parse (through the AST cache) and compile a filter string.
pub fn parse_and_compile(
    src: &str,
    resolver: &dyn Resolver,
    param_offset: usize,
) -> Result<CompiledFilter, FilterError> {
    let expr = crate::cache::parse_cached(src)?;
    compile(&expr, resolver, param_offset)
}

/// Resolve a sort path (`title`, `author.name`, `data.key`) to the SQL
/// expression to order by, using the same resolution as filters. Paths
/// through multi-valued relations, back-relations and `@collection` are
/// rejected with [`FilterError::Unsupported`].
pub fn resolve_sort_path(resolver: &dyn Resolver, path: &str) -> Result<String, FilterError> {
    if path.is_empty() || path.starts_with('@') || path.contains(':') {
        return Err(FilterError::Unsupported(format!(
            "'{path}' is not a sortable field path"
        )));
    }
    let mut paths = PathResolver::new(resolver);
    let r = paths.resolve(path)?;
    if r.from.is_some() {
        return Err(FilterError::Unsupported(format!(
            "cannot sort by '{path}': it goes through a multi-valued relation"
        )));
    }
    if !paths.joins.is_empty() {
        return Err(FilterError::Unsupported(format!(
            "cannot sort by '{path}': back-relations are not sortable"
        )));
    }
    Ok(r.sql)
}
