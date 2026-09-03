//! A small filter expression language: parses strings like
//! `status = "active" && (owner = @request.auth.id || public = true)` and
//! compiles them into parameterized SQL usable against both SQLite and
//! Postgres. Shared by record list/view queries and API access rules.

mod ast;
mod compiler;
mod error;
mod lexer;
mod parser;

pub use ast::{CompareOp, Expr, Literal, Operand};
pub use compiler::{compile, parse_and_compile, CompiledFilter, Dialect, Resolved, Resolver};
pub use error::FilterError;
pub use parser::Parser;

/// Scan a filter expression for relation dot-notation (`author.name`),
/// returning the distinct relation field names referenced (the segment
/// before the first dot), in first-seen order. `@request.*` context
/// variables are excluded since they are not relation fields.
///
/// `Resolver::resolve` is synchronous, so a relation identifier can't load
/// its target collection's schema on demand — callers that support
/// relation dot-notation use this first to prefetch every referenced
/// relation's target collection before compiling.
pub fn relation_idents(src: &str) -> Result<Vec<String>, FilterError> {
    let tokens = lexer::Lexer::new(src).tokenize()?;
    let mut out = Vec::new();
    for token in tokens {
        if let lexer::Token::Ident(name) = token {
            if name.starts_with('@') {
                continue;
            }
            if let Some((head, rest)) = name.split_once('.') {
                if !rest.is_empty() && !out.iter().any(|h: &String| h == head) {
                    out.push(head.to_string());
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestResolver;
    impl Resolver for TestResolver {
        fn resolve(&self, ident: &str) -> Result<Resolved, FilterError> {
            match ident {
                "@request.auth.id" => Ok(Resolved::Value(serde_json::json!("user-1"))),
                other => Ok(Resolved::Column(format!("\"{other}\""))),
            }
        }
    }

    fn compile_str(src: &str) -> CompiledFilter {
        parse_and_compile(src, &TestResolver, Dialect::Postgres, 0).unwrap()
    }

    #[test]
    fn simple_eq() {
        let c = compile_str(r#"status = "active""#);
        assert_eq!(c.sql, "\"status\" = $1");
        assert_eq!(c.params, vec![serde_json::json!("active")]);
    }

    #[test]
    fn and_or_precedence() {
        let c = compile_str(r#"a = 1 && b = 2 || c = 3"#);
        assert_eq!(c.sql, "((\"a\" = $1 AND \"b\" = $2) OR \"c\" = $3)");
    }

    #[test]
    fn parens_override_precedence() {
        let c = compile_str(r#"a = 1 && (b = 2 || c = 3)"#);
        assert_eq!(c.sql, "(\"a\" = $1 AND (\"b\" = $2 OR \"c\" = $3))");
    }

    #[test]
    fn contains_operator_wraps_wildcards() {
        let c = compile_str(r#"title ~ "hello""#);
        assert_eq!(c.sql, "\"title\" ILIKE $1");
        assert_eq!(c.params, vec![serde_json::json!("%hello%")]);
    }

    #[test]
    fn sqlite_dialect_uses_like() {
        let c = parse_and_compile(r#"title ~ "hi""#, &TestResolver, Dialect::Sqlite, 0).unwrap();
        assert_eq!(c.sql, "\"title\" LIKE $1");
    }

    #[test]
    fn null_comparison() {
        let c = compile_str("deleted_at = null");
        assert_eq!(c.sql, "\"deleted_at\" IS NULL");
        assert!(c.params.is_empty());

        let c = compile_str("deleted_at != null");
        assert_eq!(c.sql, "\"deleted_at\" IS NOT NULL");
    }

    #[test]
    fn context_variable_becomes_param() {
        let c = compile_str("owner = @request.auth.id");
        assert_eq!(c.sql, "\"owner\" = $1");
        assert_eq!(c.params, vec![serde_json::json!("user-1")]);
    }

    #[test]
    fn offset_shifts_placeholders() {
        let c = parse_and_compile("a = 1", &TestResolver, Dialect::Postgres, 2).unwrap();
        assert_eq!(c.sql, "\"a\" = $3");
    }

    #[test]
    fn empty_filter_errors() {
        assert!(matches!(Parser::parse(""), Err(FilterError::Empty)));
    }

    #[test]
    fn unknown_operator_errors() {
        assert!(Parser::parse("a % 1").is_err());
    }
}
