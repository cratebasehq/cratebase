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
    /// A raw, already-safely-quoted SQL column/expression fragment.
    Column(String),
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
        Value::String(s) => Value::String(format!("%{}%", s.replace('%', "\\%").replace('_', "\\_"))),
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

    fn compile_compare(
        &mut self,
        left: &Operand,
        op: CompareOp,
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
                (Resolved::Column(c), Resolved::Value(Value::Null)) => {
                    return Ok(null_check(c, matches!(op, CompareOp::Eq)))
                }
                (Resolved::Value(Value::Null), Resolved::Column(c)) => {
                    return Ok(null_check(c, matches!(op, CompareOp::Eq)))
                }
                _ => {}
            }
        }

        let is_like = matches!(op, CompareOp::Like | CompareOp::NotLike);
        let sql_op = self.sql_op(op);

        let (l_sql, r_sql) = match (l, r) {
            (Resolved::Column(lc), Resolved::Column(rc)) => (lc, rc),
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
            Expr::Compare { left, op, right } => self.compile_compare(left, *op, right),
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
