//! In-process evaluation of a filter against a record snapshot, and the
//! value-comparison semantics shared with the SQL compiler (which uses
//! them to constant-fold comparisons between two bound values).
//!
//! The semantics mirror what the compiled SQL does, which in turn mirrors
//! PocketBase:
//!
//! * `null` and `""` are the same "empty" value: `x = ""` is true for
//!   both, and `x != "a"` is true when `x` is empty.
//! * Ordered comparisons (`<`, `>`, ...) and `~` against an empty value
//!   are false, like SQL `NULL`.
//! * `~` / `!~` are case-insensitive `LIKE`: `%` matches any run, `_` a
//!   single character, `\` escapes the next character, and an operand
//!   without a `%` is escaped and wrapped in `%...%`.
//! * A multi-valued field is only unpacked by `:each`; bare, it compares
//!   as the raw JSON text of the column (`["a","b"]`), matching what
//!   PocketBase's SQL does.
//! * Element operands (`:each`): `?op` is satisfied by any element, the
//!   bare operator requires every element to match (and at least one
//!   element unless the operator tolerates the missing row: `!=`, or `=`
//!   against an empty value).
//!
//! Relation paths, back-relations, `@collection.*` and function calls need
//! the database; they return [`FilterError::Unsupported`] so the caller
//! can fall back to SQL. That matters for correctness as well as
//! completeness: a bare operator through a join means "every joined row",
//! which no record snapshot can answer.

use std::cmp::Ordering;

use cratebase_core::{FieldKind, FieldType};
use serde_json::{Map, Value};

use crate::ast::{CompareOp, Expr, Modifier, Operand};
use crate::error::FilterError;
use crate::path::is_back_relation;
use crate::resolver::Resolver;
use crate::terms::{macro_value, MacroTerm};

/// An operand reduced to plain values.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalTerm {
    Scalar(Value),
    Multi(Vec<Value>),
}

impl EvalTerm {
    /// Split a value into elements for `:each` semantics: arrays yield
    /// their elements, empty values nothing, other scalars themselves.
    pub fn each(value: Value) -> EvalTerm {
        match value {
            Value::Array(items) => EvalTerm::Multi(items),
            v if is_empty(&v) => EvalTerm::Multi(vec![]),
            v => EvalTerm::Multi(vec![v]),
        }
    }

    fn map(self, f: impl Fn(Value) -> Value) -> EvalTerm {
        match self {
            EvalTerm::Scalar(v) => EvalTerm::Scalar(f(v)),
            EvalTerm::Multi(items) => EvalTerm::Multi(items.into_iter().map(f).collect()),
        }
    }
}

/// PocketBase's notion of an empty value.
pub fn is_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        _ => false,
    }
}

/// Number of elements `:length` reports for a value.
pub fn length_of(v: &Value) -> usize {
    match v {
        Value::Array(items) => items.len(),
        Value::String(s) => {
            // A JSON-encoded array stored as text.
            match serde_json::from_str::<Value>(s) {
                Ok(Value::Array(items)) => items.len(),
                _ => usize::from(!s.is_empty()),
            }
        }
        Value::Null => 0,
        _ => 1,
    }
}

pub fn lowercase(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(s.to_lowercase()),
        other => other,
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Textual form used for LIKE and lexicographic comparisons (what the
/// database would coerce the value to).
pub fn to_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn values_equal(l: &Value, r: &Value) -> bool {
    match (l, r) {
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Number(_), _)
        | (_, Value::Number(_))
        | (Value::Bool(_), _)
        | (_, Value::Bool(_)) => match (as_f64(l), as_f64(r)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        },
        _ => to_text(l) == to_text(r),
    }
}

fn order(l: &Value, r: &Value) -> Ordering {
    match (as_f64(l), as_f64(r)) {
        (Some(a), Some(b)) if !matches!((l, r), (Value::String(_), Value::String(_))) => {
            a.partial_cmp(&b).unwrap_or(Ordering::Equal)
        }
        _ => to_text(l).cmp(&to_text(r)),
    }
}

/// Escape `\`, `%` and `_` for a `LIKE ... ESCAPE '\'` pattern, leaving
/// sequences the author already escaped alone.
///
/// This is PocketBase's `escapeUnescapedChars`, which decides right to
/// left: a special character is escaped unless the character in front of it
/// is a backslash, and that backslash is then not itself escaped. So `a_b`
/// becomes `a\_b`, `a\b` becomes `a\\b` (a literal backslash) and `a\_b` is
/// already an escaped `_` and stays as it is.
pub fn escape_like(raw: &str) -> String {
    const SPECIAL: [char; 3] = ['\\', '%', '_'];
    let chars: Vec<char> = raw.chars().collect();
    let mut out = Vec::with_capacity(chars.len() + 2);
    let mut pending = false;
    for &c in chars.iter().rev() {
        if pending {
            if c != '\\' {
                out.push('\\');
            }
            pending = false;
        } else {
            pending = SPECIAL.contains(&c);
        }
        out.push(c);
    }
    if pending {
        out.push('\\');
    }
    out.iter().rev().collect()
}

/// PocketBase's LIKE operand normalization: an operand that already carries
/// a `%` is a hand-written pattern and is used verbatim; anything else is
/// escaped and wrapped in `%...%`. The compiled SQL always declares
/// `ESCAPE '\'`.
pub fn like_pattern(raw: &str) -> String {
    if raw.contains('%') {
        raw.to_string()
    } else {
        format!("%{}%", escape_like(raw))
    }
}

/// Case-insensitive SQL `LIKE` matcher (`%` any run, `_` one char, `\`
/// escaping the next character as a literal — the `ESCAPE '\'` the
/// compiler emits).
pub fn like_match(subject: &str, pattern: &str) -> bool {
    let s: Vec<char> = subject.to_lowercase().chars().collect();
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    // Classic iterative wildcard matching with backtracking on the last `%`.
    // `star` remembers the `%` position in the pattern and how far the
    // subject had been consumed when it was taken.
    let (mut si, mut pi) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    // The pattern character at `i`, and how many characters it spans: a
    // backslash makes the next character a literal.
    let literal = |i: usize| -> Option<(char, usize)> {
        match p.get(i) {
            Some('\\') => match p.get(i + 1) {
                Some(&c) => Some((c, 2)),
                // A trailing escape has nothing to escape; take it as one.
                None => Some(('\\', 1)),
            },
            Some(&c) if c == '%' || c == '_' => None,
            Some(&c) => Some((c, 1)),
            None => None,
        }
    };
    while si < s.len() {
        // How far the pattern advances if it consumes `s[si]` here.
        let step = match literal(pi) {
            Some((c, width)) => (c == s[si]).then_some(width),
            None => match p.get(pi) {
                Some('_') => Some(1),
                Some('%') => {
                    star = Some((pi, si));
                    pi += 1;
                    continue;
                }
                _ => None,
            },
        };
        match (step, star) {
            (Some(width), _) => {
                si += 1;
                pi += width;
            }
            (None, Some((sp, ss))) => {
                pi = sp + 1;
                si = ss + 1;
                star = Some((sp, ss + 1));
            }
            (None, None) => return false,
        }
    }
    while p.get(pi) == Some(&'%') {
        pi += 1;
    }
    pi == p.len()
}

/// Compare two scalar values with SQL/PocketBase semantics.
pub fn compare_scalars(l: &Value, op: CompareOp, r: &Value) -> bool {
    match op {
        CompareOp::Eq => {
            if is_empty(l) || is_empty(r) {
                is_empty(l) && is_empty(r)
            } else {
                values_equal(l, r)
            }
        }
        CompareOp::NotEq => !compare_scalars(l, CompareOp::Eq, r),
        CompareOp::Gt | CompareOp::Gte | CompareOp::Lt | CompareOp::Lte => {
            if l.is_null() || r.is_null() {
                return false;
            }
            let ord = order(l, r);
            match op {
                CompareOp::Gt => ord == Ordering::Greater,
                CompareOp::Gte => ord != Ordering::Less,
                CompareOp::Lt => ord == Ordering::Less,
                _ => ord != Ordering::Greater,
            }
        }
        CompareOp::Like | CompareOp::NotLike => {
            if l.is_null() || r.is_null() {
                return false;
            }
            let matched = like_match(&to_text(l), &like_pattern(&to_text(r)));
            if op == CompareOp::Like {
                matched
            } else {
                !matched
            }
        }
    }
}

/// Whether a comparison against a *missing* element (the `NULL` row a
/// `LEFT JOIN` over an empty array produces) is satisfied: `!= v` for a
/// non-empty `v`, and `= ""`.
pub fn null_tolerant(op: CompareOp, other: Option<&Value>) -> bool {
    match op {
        CompareOp::NotEq => other.is_none_or(|v| !is_empty(v)),
        CompareOp::Eq => other.is_some_and(is_empty),
        _ => false,
    }
}

/// Evaluate a multi-valued operand against a per-element predicate with
/// the any-of/all-of rules described in the module docs.
pub fn multi_match(
    elems: &[Value],
    any_of: bool,
    null_ok: bool,
    cond: impl Fn(&Value) -> bool,
) -> bool {
    if any_of {
        elems.iter().any(&cond) || (null_ok && elems.is_empty())
    } else if null_ok {
        elems.iter().all(&cond)
    } else {
        !elems.is_empty() && elems.iter().all(&cond)
    }
}

/// Compare two reduced operands.
pub fn compare_terms(
    l: &EvalTerm,
    op: CompareOp,
    any_of: bool,
    r: &EvalTerm,
) -> Result<bool, FilterError> {
    Ok(match (l, r) {
        (EvalTerm::Scalar(a), EvalTerm::Scalar(b)) => compare_scalars(a, op, b),
        (EvalTerm::Multi(items), EvalTerm::Scalar(v)) => {
            multi_match(items, any_of, null_tolerant(op, Some(v)), |e| {
                compare_scalars(e, op, v)
            })
        }
        (EvalTerm::Scalar(v), EvalTerm::Multi(items)) => {
            multi_match(items, any_of, null_tolerant(op, Some(v)), |e| {
                compare_scalars(v, op, e)
            })
        }
        (EvalTerm::Multi(_), EvalTerm::Multi(_)) => {
            return Err(FilterError::Unsupported(
                "comparing two multi-valued operands".into(),
            ))
        }
    })
}

/// Evaluate `expr` against a record snapshot (PocketBase-shaped JSON:
/// multi-valued fields as arrays, dates as strings) of `ctx.root()`.
pub fn evaluate(
    expr: &Expr,
    record: &Map<String, Value>,
    ctx: &dyn Resolver,
) -> Result<bool, FilterError> {
    match expr {
        // Both sides are always evaluated so an `Unsupported` sub-expression
        // surfaces deterministically regardless of the record's data.
        Expr::And(l, r) => {
            let a = evaluate(l, record, ctx)?;
            let b = evaluate(r, record, ctx)?;
            Ok(a && b)
        }
        Expr::Or(l, r) => {
            let a = evaluate(l, record, ctx)?;
            let b = evaluate(r, record, ctx)?;
            Ok(a || b)
        }
        Expr::Compare {
            left,
            op,
            any_of,
            right,
        } => {
            let mut l = eval_operand(left, record, ctx)?;
            let mut r = eval_operand(right, record, ctx)?;
            // `:lower` on one side lowercases string values on the other.
            if has_modifier(left, Modifier::Lower) {
                r = r.map(lowercase);
            }
            if has_modifier(right, Modifier::Lower) {
                l = l.map(lowercase);
            }
            compare_terms(&l, *op, *any_of, &r)
        }
    }
}

fn has_modifier(op: &Operand, m: Modifier) -> bool {
    matches!(op, Operand::Ident { modifier: Some(x), .. } if *x == m)
}

fn eval_operand(
    operand: &Operand,
    record: &Map<String, Value>,
    ctx: &dyn Resolver,
) -> Result<EvalTerm, FilterError> {
    match operand {
        Operand::Literal(lit) => Ok(EvalTerm::Scalar(crate::terms::literal_value(lit))),
        Operand::Call { name, .. } => Err(FilterError::Unsupported(format!(
            "function '{name}' requires SQL evaluation"
        ))),
        Operand::Ident { path, modifier } => {
            if path.starts_with('@') {
                return match macro_value(path, *modifier, ctx)? {
                    MacroTerm::Value { value, each } => Ok(if each {
                        EvalTerm::each(value)
                    } else {
                        EvalTerm::Scalar(value)
                    }),
                    MacroTerm::Collection { .. } => Err(FilterError::Unsupported(format!(
                        "'{path}' requires SQL evaluation"
                    ))),
                };
            }
            eval_field(path, *modifier, record, ctx)
        }
    }
}

/// Navigate a JSON value by dotted path segments.
fn json_get<'a>(mut v: &'a Value, segments: &[&str]) -> &'a Value {
    for seg in segments {
        v = match v {
            Value::Object(m) => m.get(*seg).unwrap_or(&Value::Null),
            Value::Array(items) => seg
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i))
                .unwrap_or(&Value::Null),
            _ => &Value::Null,
        };
    }
    v
}

/// Multi-valued fields may arrive as a JSON-encoded string straight from
/// a database row; normalize to a list of values.
fn array_elems(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(items) => items.clone(),
        Value::String(s) if s.is_empty() => vec![],
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(Value::Array(items)) => items,
            _ => vec![v.clone()],
        },
        Value::Null => vec![],
        other => vec![other.clone()],
    }
}

fn eval_field(
    path: &str,
    modifier: Option<Modifier>,
    record: &Map<String, Value>,
    ctx: &dyn Resolver,
) -> Result<EvalTerm, FilterError> {
    let segments: Vec<&str> = path.split('.').collect();
    let root = ctx.root();
    let head = segments[0];
    let Some(field) = root.field(head) else {
        if is_back_relation(head) {
            return Err(FilterError::Unsupported(format!(
                "back-relation '{path}' requires SQL evaluation"
            )));
        }
        return Err(FilterError::UnknownField(path.to_string()));
    };
    let raw = record.get(head).cloned().unwrap_or(Value::Null);
    let rest = &segments[1..];

    let value: Value = if rest.is_empty() {
        raw
    } else {
        match &field.kind {
            FieldKind::Json { .. } => {
                let parsed = match &raw {
                    Value::String(s) => serde_json::from_str(s).unwrap_or(raw.clone()),
                    other => other.clone(),
                };
                json_get(&parsed, rest).clone()
            }
            FieldKind::GeoPoint {} if rest.len() == 1 && matches!(rest[0], "lon" | "lat") => {
                let parsed = match &raw {
                    Value::String(s) => serde_json::from_str(s).unwrap_or(raw.clone()),
                    other => other.clone(),
                };
                json_get(&parsed, rest).clone()
            }
            FieldKind::Relation { .. } => {
                return Err(FilterError::Unsupported(format!(
                    "relation path '{path}' requires SQL evaluation"
                )))
            }
            _ => return Err(FilterError::UnknownField(path.to_string())),
        }
    };

    let multi = rest.is_empty() && field.is_multiple();
    match modifier {
        Some(Modifier::IsSet) => Err(FilterError::InvalidModifier(format!(
            "{path}:isset is only valid on @request.body fields"
        ))),
        // `:length` on a single-valued field is a no-op in PocketBase, not
        // an error: the raw value is compared.
        Some(Modifier::Length) if !multi => Ok(EvalTerm::Scalar(value)),
        Some(Modifier::Length) => Ok(EvalTerm::Scalar(Value::from(length_of(&value)))),
        Some(Modifier::Each) => {
            if !multi {
                return Err(FilterError::InvalidModifier(format!(
                    "{path}:each requires a multi-valued field"
                )));
            }
            Ok(EvalTerm::Multi(array_elems(&value)))
        }
        // Without `:each` a multi-valued field is its raw JSON text, so
        // `:lower` lowercases that text rather than each element.
        Some(Modifier::Lower) => Ok(EvalTerm::Scalar(lowercase(as_column_text(&value, multi)))),
        None => Ok(EvalTerm::Scalar(if multi {
            as_column_text(&value, true)
        } else if field.field_type() == FieldType::Json && rest.is_empty() {
            // Whole json field: compare its decoded value like
            // `json_extract(col, '$')` does.
            match &value {
                Value::String(s) => serde_json::from_str(s).unwrap_or(value.clone()),
                other => other.clone(),
            }
        } else {
            value
        })),
    }
}

/// What the database column holds for a multi-valued field: the JSON text
/// of the array, which is what a comparison without `:each` sees.
fn as_column_text(value: &Value, multi: bool) -> Value {
    if !multi {
        return value.clone();
    }
    match value {
        // An absent column is `NULL`, which stays "empty"; a stored array
        // (or a string already holding one) compares as its JSON text.
        Value::Null => Value::Null,
        Value::String(s) => Value::String(s.clone()),
        other => Value::String(other.to_string()),
    }
}
