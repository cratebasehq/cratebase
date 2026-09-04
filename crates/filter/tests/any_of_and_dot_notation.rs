//! Lexer/parser coverage for the "any of" operators (`?=`, `?!=`, ...) and
//! dotted identifiers (relation dot-notation), plus compiler coverage for
//! how a multi-valued column changes the compiled SQL.

use cratebase_filter::testing::TestResolver;
use cratebase_filter::{
    parse_and_compile, CompareOp, CompiledFilter, Expr, FilterError, Literal, Operand, Parser,
};

#[test]
fn lexes_any_of_operators() {
    for (src, op) in [
        (r#"a ?= 1"#, CompareOp::Eq),
        (r#"a ?!= 1"#, CompareOp::NotEq),
        (r#"a ?> 1"#, CompareOp::Gt),
        (r#"a ?>= 1"#, CompareOp::Gte),
        (r#"a ?< 1"#, CompareOp::Lt),
        (r#"a ?<= 1"#, CompareOp::Lte),
        (r#"a ?~ "x""#, CompareOp::Like),
        (r#"a ?!~ "x""#, CompareOp::NotLike),
    ] {
        let expr = Parser::parse(src).unwrap();
        match expr {
            Expr::Compare {
                left: Operand::Ident { path, .. },
                op: parsed_op,
                any_of,
                ..
            } => {
                assert_eq!(path, "a");
                assert_eq!(parsed_op, op);
                assert!(any_of, "expected any_of=true for {src:?}");
            }
            other => panic!("unexpected parse for {src:?}: {other:?}"),
        }
    }
}

#[test]
fn bare_operators_are_not_any_of() {
    let expr = Parser::parse("a = 1").unwrap();
    match expr {
        Expr::Compare { any_of, .. } => assert!(!any_of),
        other => panic!("unexpected parse: {other:?}"),
    }
}

#[test]
fn parses_dotted_identifiers_as_single_ident() {
    let expr = Parser::parse(r#"author.name = "Alice""#).unwrap();
    match expr {
        Expr::Compare {
            left:
                Operand::Ident {
                    path,
                    modifier: None,
                },
            op: CompareOp::Eq,
            any_of: false,
            right: Operand::Literal(Literal::Str(s)),
        } => {
            assert_eq!(path, "author.name");
            assert_eq!(s, "Alice");
        }
        other => panic!("unexpected parse: {other:?}"),
    }
}

#[test]
fn relation_dot_notation_resolves_through_the_schema() {
    let r = TestResolver::sqlite("posts");
    let c = parse_and_compile(
        r#"author.name = "Alice" && @request.auth.id != "" && tags.name ~ "x""#,
        &r,
        0,
    )
    .unwrap();
    assert!(c.sql.contains(
        r#"(SELECT "users"."name" FROM "users" WHERE "users"."id" = "posts"."author") = $1"#
    ));
    assert!(c.sql.contains(r#"JOIN "tags" AS"#));
    assert!(
        c.joins.is_empty(),
        "forward relations need no top-level join"
    );
}

#[test]
fn any_of_operator_errors_on_invalid_syntax() {
    assert!(matches!(Parser::parse("a ? 1"), Err(FilterError::Lex(..))));
}

fn compile(src: &str) -> CompiledFilter {
    parse_and_compile(src, &TestResolver::sqlite("posts"), 0).unwrap()
}

/// The `?` prefix only quantifies over something there are several of.
/// A bare multi-valued column has no elements to quantify — like
/// PocketBase it compares against the raw JSON text of the column — so
/// `?=` and `=` compile identically.
#[test]
fn any_of_on_a_bare_multi_column_is_a_text_comparison() {
    let c = compile(r#"categories ?= "tech""#);
    assert_eq!(c.sql, "\"posts\".\"categories\" = $1");
    assert_eq!(c.params, vec![serde_json::json!("tech")]);
    assert_eq!(compile(r#"categories = "tech""#).sql, c.sql);
}

#[test]
fn any_of_over_each_compiles_to_exists_over_json_each() {
    let c = compile(r#"categories:each ?= "tech""#);
    assert_eq!(
        c.sql,
        "EXISTS (SELECT 1 FROM json_each(COALESCE(\"posts\".\"categories\", '[]')) AS \"__e1\" WHERE \"__e1\".\"value\" = $1)"
    );
    assert_eq!(c.params, vec![serde_json::json!("tech")]);
}

#[test]
fn bare_operator_on_each_requires_every_element() {
    let c = compile(r#"categories:each = "tech""#);
    assert_eq!(
        c.sql,
        "(EXISTS (SELECT 1 FROM json_each(COALESCE(\"posts\".\"categories\", '[]')) AS \"__e1\" WHERE \"__e1\".\"value\" = $1) \
         AND NOT EXISTS (SELECT 1 FROM json_each(COALESCE(\"posts\".\"categories\", '[]')) AS \"__e1\" WHERE NOT (\"__e1\".\"value\" = $1)))"
    );
}

#[test]
fn plain_scalar_field_unaffected_by_any_of_support() {
    let c = compile(r#"status = "active""#);
    assert_eq!(c.sql, "\"posts\".\"status\" = $1");
}
