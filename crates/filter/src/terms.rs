//! Resolution of the `@`-prefixed operands shared by the SQL compiler and
//! the in-process evaluator: date macros, `@request.*` values and the
//! `@collection.X` prefix.

use serde_json::Value;

use crate::ast::{Literal, Modifier};
use crate::error::FilterError;
use crate::eval::{length_of, lowercase};
use crate::macros::date_macro;
use crate::resolver::{RequestPath, Resolver};

/// What an `@identifier` reduces to.
#[derive(Debug, Clone, PartialEq)]
pub enum MacroTerm {
    /// A value known at compile time (bound as a parameter). `each` is set
    /// by the `:each` modifier and means "treat the value as a list".
    Value { value: Value, each: bool },
    /// `@collection.<name>.<field path>`: needs a join, handled by the
    /// path resolver.
    Collection { collection: String, path: String },
}

pub fn literal_value(lit: &Literal) -> Value {
    match lit {
        Literal::Str(s) => Value::String(s.clone()),
        Literal::Num(n) => number_value(*n),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Null => Value::Null,
    }
}

/// Integral numbers become JSON integers so they bind as integers.
pub fn number_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

/// Resolve `@...` identifiers. `path` includes the leading `@`.
pub fn macro_value(
    path: &str,
    modifier: Option<Modifier>,
    ctx: &dyn Resolver,
) -> Result<MacroTerm, FilterError> {
    let segments: Vec<&str> = path[1..].split('.').collect();
    match segments[0] {
        "request" => {
            let Some(req) = RequestPath::parse(&segments[1..]) else {
                return Err(FilterError::UnknownMacro(path.to_string()));
            };
            if modifier == Some(Modifier::IsSet) {
                return match &req {
                    RequestPath::Body(key) => Ok(MacroTerm::Value {
                        value: Value::Bool(ctx.body_has(key)),
                        each: false,
                    }),
                    _ => Err(FilterError::InvalidModifier(format!(
                        "{path}:isset is only valid on @request.body fields"
                    ))),
                };
            }
            let value = ctx.request_value(&req);
            Ok(apply_value_modifier(value, modifier))
        }
        "collection" => {
            // Modifiers apply to the resolved field and are handled by the
            // caller.
            if segments.len() < 3 {
                return Err(FilterError::UnknownMacro(path.to_string()));
            }
            Ok(MacroTerm::Collection {
                collection: segments[1].to_string(),
                path: segments[2..].join("."),
            })
        }
        name if segments.len() == 1 => match date_macro(name, ctx.now()) {
            Some(value) => {
                if let Some(m) = modifier {
                    return Err(FilterError::InvalidModifier(format!(
                        "{path}:{} is not supported on date macros",
                        m.as_str()
                    )));
                }
                Ok(MacroTerm::Value { value, each: false })
            }
            None => Err(FilterError::UnknownMacro(path.to_string())),
        },
        _ => Err(FilterError::UnknownMacro(path.to_string())),
    }
}

fn apply_value_modifier(value: Value, modifier: Option<Modifier>) -> MacroTerm {
    match modifier {
        None => MacroTerm::Value { value, each: false },
        Some(Modifier::Each) => MacroTerm::Value { value, each: true },
        Some(Modifier::Length) => MacroTerm::Value {
            value: Value::from(length_of(&value)),
            each: false,
        },
        Some(Modifier::Lower) => MacroTerm::Value {
            value: match value {
                Value::Array(items) => Value::Array(items.into_iter().map(lowercase).collect()),
                other => lowercase(other),
            },
            each: false,
        },
        // `:isset` is handled by the caller.
        Some(Modifier::IsSet) => MacroTerm::Value { value, each: false },
    }
}
