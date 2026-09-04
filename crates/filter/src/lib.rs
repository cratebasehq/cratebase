//! PocketBase's filter / API-rule expression language for Cratebase.
//!
//! Parses strings like
//!
//! ```text
//! status = "active" && (author = @request.auth.id || tags.name ?= "public")
//! ```
//!
//! into an [`Expr`], compiles them into parameterized SQL for SQLite and
//! Postgres ([`compile`]), and can evaluate them in-process against a
//! record snapshot for realtime subscriptions ([`evaluate`]).
//!
//! # Grammar
//!
//! * Operators: `= != > >= < <= ~ !~` and their "any of" forms
//!   `?= ?!= ?> ?>= ?< ?<= ?~ ?!~`; `&&`, `||`, parentheses.
//! * Identifiers: dotted field paths through relations (`author.name`),
//!   back-relations (`comments_via_post.title`), JSON paths
//!   (`data.some.key`), geo coordinates (`loc.lat`); optional modifier
//!   `:isset | :length | :each | :lower`.
//! * Macros: `@now @second @minute @hour @weekday @day @month @year
//!   @yesterday @tomorrow @todayStart @todayEnd @monthStart @monthEnd
//!   @yearStart @yearEnd`, `@request.auth[.path]`, `@request.body.path`
//!   (alias `@request.data.path`), `@request.query.path`,
//!   `@request.headers.name`, `@request.method`, `@request.context`,
//!   `@collection.name.path`.
//! * Function calls: `geoDistance(lonA, latA, lonB, latB)`.
//!
//! # Semantics the compiler mirrors from PocketBase
//!
//! * `null` and `""` are the same "empty" value; `!=` also matches
//!   `NULL` columns.
//! * `~` wraps the pattern in `%...%` unless it already contains `%`.
//! * Multi-valued operands: `?op` is any-element, the bare operator is
//!   every-element (see [`eval`] for the exact rules).
//! * Back-relations and `@collection.X` are `LEFT JOIN`s shared by every
//!   reference to the same collection, so several conditions constrain
//!   the same joined row.
//!
//! The host implements [`Resolver`] to supply the schema and the request
//! context; [`parse_cached`] keeps parsed rules in a bounded LRU.

mod ast;
mod cache;
mod compiler;
mod error;
pub mod eval;
mod lexer;
mod macros;
mod parser;
mod path;
mod resolver;
mod terms;

#[doc(hidden)]
pub mod testing;

pub use ast::{CompareOp, Expr, Literal, Modifier, Operand, FUNCTIONS};
pub use cache::{parse_cached, CACHE_CAPACITY};
pub use compiler::{compile, parse_and_compile, resolve_sort_path, CompiledFilter};
pub use error::FilterError;
pub use eval::evaluate;
pub use macros::{date_macro, DATE_MACROS};
pub use parser::Parser;
pub use path::{Join, MAX_DEPTH};
pub use resolver::{Dialect, RequestPath, Resolver};

#[cfg(test)]
mod tests;
