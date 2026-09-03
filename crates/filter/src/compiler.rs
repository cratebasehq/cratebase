use serde_json::Value;

use crate::ast::{CompareOp, Expr, Literal, Operand};
use crate::error::FilterError;

/// Target SQL dialect. Only the two backends Cratebase supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

/// What a filter identifier resolves to.
pub enum Resolved {
    /// A raw, already-safely-quoted SQL column/expression fragment
    /// holding a single scalar value.
    Column(String),
    /// Like [`Resolved::Column`], but the expression is a JSON array
    /// stored as TEXT (a multi-valued select/relation/file field). "Any
    /// of" operators (`?=`, `?!=`, ...) test each decoded element; bare
    /// operators require every element to satisfy the comparison.
    MultiColumn(String),
    /// Relation dot-notation (`author.name`) reached through a
    /// *multi-valued* relation field: `array_column` is the JSON array of
    /// related ids on the current table; `target_table`/`target_column`
    /// name the related row's field being compared. Compiled as an
    /// `EXISTS`/`NOT EXISTS` join, with the same any-of/all semantics as
    /// [`Resolved::MultiColumn`].
    RelatedMulti {
        array_column: String,
        target_table: String,
        target_column: String,
    },
    /// A literal value supplied out-of-band (e.g. `@request.auth.id`),
    /// bound as a query parameter rather than interpolated.
    Value(Value),
}

/// Maps filter identifiers (record field names, `@request.auth.*`, ...) to
/// SQL columns or bound values. Implemented by `cratebase-db` /
/// `cratebase-server` with knowledge of the collection schema and current
/// request context.
pub trait Resolver {
    fn resolve(&self, ident: &str) -> Result<Resolved, FilterError>;
}

/// Result of compiling a filter expression: a parameterized SQL fragment
/// (using `$1`, `$2`, ... placeholders, valid for both the sqlite and
/// postgres sqlx drivers) plus the bound parameter values in order.
pub struct CompiledFilter {
    pub sql: String,
    pub params: Vec<Value>,
}

fn literal_to_json(lit: &Literal) -> Value {
    match lit {
        Literal::Str(s) => Value::String(s.clone()),
        Literal::Num(n) => serde_json::Number::from_f64(*n)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Null => Value::Null,
    }
}

fn wrap_like(v: Value) -> Value {
    match v {
        Value::String(s) => {
            Value::String(format!("%{}%", s.replace('%', "\\%").replace('_', "\\_")))
        }
        other => other,
    }
}

struct Compiler<'a> {
    resolver: &'a dyn Resolver,
    dialect: Dialect,
    offset: usize,
    params: Vec<Value>,
}

impl<'a> Compiler<'a> {
    fn resolve_operand(&self, op: &Operand) -> Result<Resolved, FilterError> {
        match op {
            Operand::Ident(name) => self.resolver.resolve(name),
            Operand::Literal(l) => Ok(Resolved::Value(literal_to_json(l))),
        }
    }

    fn push_param(&mut self, v: Value) -> String {
        self.params.push(v);
        format!("${}", self.offset + self.params.len())
    }

    fn sql_op(&self, op: CompareOp) -> &'static str {
        use CompareOp::*;
        use Dialect::*;
        match (op, self.dialect) {
            (Eq, _) => "=",
            (NotEq, _) => "<>",
            (Gt, _) => ">",
            (Gte, _) => ">=",
            (Lt, _) => "<",
            (Lte, _) => "<=",
            (Like, Postgres) => "ILIKE",
            (Like, Sqlite) => "LIKE",
            (NotLike, Postgres) => "NOT ILIKE",
            (NotLike, Sqlite) => "NOT LIKE",
        }
    }

    /// FROM-fragment + qualified value-column reference for enumerating a
    /// JSON array expression's decoded elements. `array_expr` may be a
    /// plain column reference or an arbitrary SQL expression (e.g. a
    /// relation subquery).
    fn array_elements(&self, array_expr: &str) -> (String, String) {
        let value = "__elem.value".to_string();
        let from = match self.dialect {
            Dialect::Sqlite => format!("json_each(COALESCE({array_expr}, '[]')) AS __elem"),
            Dialect::Postgres => format!(
                "jsonb_array_elements_text(COALESCE({array_expr}, '[]')::jsonb) AS __elem(value)"
            ),
        };
        (from, value)
    }

    /// Compile a multi-valued column compared to a literal: `?=` and its
    /// siblings test whether *any* decoded array element satisfies the
    /// comparison; the bare operators require *every* element to (a
    /// vacuously true, hence satisfied, condition for an empty array).
    fn compile_array_compare(
        &mut self,
        array_expr: &str,
        op: CompareOp,
        any_of: bool,
        value: Value,
        is_like: bool,
    ) -> String {
        let (from, elem) = self.array_elements(array_expr);
        let value = if is_like { wrap_like(value) } else { value };
        let param = self.push_param(value);
        let sql_op = self.sql_op(op);
        if any_of {
            format!("EXISTS (SELECT 1 FROM {from} WHERE {elem} {sql_op} {param})")
        } else {
            format!("NOT EXISTS (SELECT 1 FROM {from} WHERE NOT ({elem} {sql_op} {param}))")
        }
    }

    /// Same as [`Compiler::compile_array_compare`], but the compared value
    /// comes from joining each related id to `target_table` and reading
    /// `target_column` off it (relation dot-notation through a
    /// multi-valued relation field).
    #[allow(clippy::too_many_arguments)]
    fn compile_related_multi_compare(
        &mut self,
        array_column: &str,
        target_table: &str,
        target_column: &str,
        op: CompareOp,
        any_of: bool,
        value: Value,
        is_like: bool,
    ) -> String {
        let (from, elem) = self.array_elements(array_column);
        let join = format!("{from} JOIN {target_table} AS __rel ON __rel.\"id\" = {elem}");
        let value = if is_like { wrap_like(value) } else { value };
        let param = self.push_param(value);
        let sql_op = self.sql_op(op);
        let target = format!("__rel.{target_column}");
        if any_of {
            format!("EXISTS (SELECT 1 FROM {join} WHERE {target} {sql_op} {param})")
        } else {
            format!("NOT EXISTS (SELECT 1 FROM {join} WHERE NOT ({target} {sql_op} {param}))")
        }
    }

    fn compile_compare(
        &mut self,
        left: &Operand,
        op: CompareOp,
        any_of: bool,
        right: &Operand,
    ) -> Result<String, FilterError> {
        let l = self.resolve_operand(left)?;
        let r = self.resolve_operand(right)?;

        // NULL comparisons need `IS [NOT] NULL`, not `= $1`.
        if matches!(op, CompareOp::Eq | CompareOp::NotEq) {
            let null_check = |col: &str, is_eq: bool| {
                if is_eq {
                    format!("{col} IS NULL")
                } else {
                    format!("{col} IS NOT NULL")
                }
            };
            match (&l, &r) {
                (Resolved::Column(c), Resolved::Value(Value::Null))
                | (Resolved::MultiColumn(c), Resolved::Value(Value::Null)) => {
                    return Ok(null_check(c, matches!(op, CompareOp::Eq)))
                }
                (Resolved::Value(Value::Null), Resolved::Column(c))
                | (Resolved::Value(Value::Null), Resolved::MultiColumn(c)) => {
                    return Ok(null_check(c, matches!(op, CompareOp::Eq)))
                }
                _ => {}
            }
        }

        let is_like = matches!(op, CompareOp::Like | CompareOp::NotLike);

        // A multi-valued operand compared against a literal compiles to an
        // EXISTS/NOT EXISTS test over its decoded elements rather than a
        // plain scalar comparison.
        match (&l, &r) {
            (Resolved::MultiColumn(col), Resolved::Value(v)) => {
                return Ok(self.compile_array_compare(col, op, any_of, v.clone(), is_like));
            }
            (Resolved::Value(v), Resolved::MultiColumn(col)) => {
                return Ok(self.compile_array_compare(col, op, any_of, v.clone(), is_like));
            }
            (
                Resolved::RelatedMulti {
                    array_column,
                    target_table,
                    target_column,
                },
                Resolved::Value(v),
            )
            | (
                Resolved::Value(v),
                Resolved::RelatedMulti {
                    array_column,
                    target_table,
                    target_column,
                },
            ) => {
                return Ok(self.compile_related_multi_compare(
                    array_column,
                    target_table,
                    target_column,
                    op,
                    any_of,
                    v.clone(),
                    is_like,
                ));
            }
            _ => {}
        }

        let sql_op = self.sql_op(op);

        let (l_sql, r_sql) = match (l, r) {
            (Resolved::Column(lc), Resolved::Column(rc)) => (lc, rc),
            (Resolved::MultiColumn(lc), Resolved::Column(rc)) => (lc, rc),
            (Resolved::Column(lc), Resolved::MultiColumn(rc)) => (lc, rc),
            (Resolved::MultiColumn(lc), Resolved::MultiColumn(rc)) => (lc, rc),
            (Resolved::Column(lc), Resolved::Value(v)) => {
                let v = if is_like { wrap_like(v) } else { v };
                (lc, self.push_param(v))
            }
            (Resolved::Value(v), Resolved::Column(rc)) => {
                let v = if is_like { wrap_like(v) } else { v };
                (self.push_param(v), rc)
            }
            (Resolved::Value(a), Resolved::Value(b)) => {
                let a = if is_like { wrap_like(a) } else { a };
                (self.push_param(a), self.push_param(b))
            }
            (Resolved::MultiColumn(_), Resolved::Value(_))
            | (Resolved::Value(_), Resolved::MultiColumn(_)) => {
                unreachable!("MultiColumn/Value combos are handled earlier in compile_compare")
            }
            (Resolved::RelatedMulti { .. }, _) | (_, Resolved::RelatedMulti { .. }) => {
                return Err(FilterError::Parse(
                    "relation dot-notation through a multi-valued field can only be \
                     compared to a literal value"
                        .to_string(),
                ));
            }
        };
        Ok(format!("{l_sql} {sql_op} {r_sql}"))
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

/// Compile a parsed filter [`Expr`] into a parameterized SQL fragment.
/// `offset` lets the caller reserve leading placeholder slots (e.g. when the
/// filter is appended after other bound parameters in the same query).
pub fn compile(
    expr: &Expr,
    resolver: &dyn Resolver,
    dialect: Dialect,
    offset: usize,
) -> Result<CompiledFilter, FilterError> {
    let mut compiler = Compiler {
        resolver,
        dialect,
        offset,
        params: Vec::new(),
    };
    let sql = compiler.compile_expr(expr)?;
    Ok(CompiledFilter {
        sql,
        params: compiler.params,
    })
}

/// Parse and compile a filter string in one step.
pub fn parse_and_compile(
    src: &str,
    resolver: &dyn Resolver,
    dialect: Dialect,
    offset: usize,
) -> Result<CompiledFilter, FilterError> {
    let expr = crate::parser::Parser::parse(src)?;
    compile(&expr, resolver, dialect, offset)
}
