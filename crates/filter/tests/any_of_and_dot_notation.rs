//! Lexer/parser coverage for the "any of" operators (`?=`, `?!=`, ...) and
//! dotted identifiers (relation dot-notation), plus compiler coverage for
//! how a resolver-declared multi-valued column changes the compiled SQL.

use cratebase_filter::{
    parse_and_compile, CompareOp, CompiledFilter, Dialect, Expr, FilterError, Literal, Operand,
    Parser, Resolved, Resolver,
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
                left: Operand::Ident(name),
                op: parsed_op,
                any_of,
                ..
            } => {
                assert_eq!(name, "a");
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
            left: Operand::Ident(name),
            op: CompareOp::Eq,
            any_of: false,
            right: Operand::Literal(Literal::Str(s)),
        } => {
            assert_eq!(name, "author.name");
            assert_eq!(s, "Alice");
        }
        other => panic!("unexpected parse: {other:?}"),
    }
}

#[test]
fn relation_idents_finds_dotted_field_heads() {
    let idents = cratebase_filter::relation_idents(
        r#"author.name = "Alice" && @request.auth.id != "" && category.slug ~ "x""#,
    )
    .unwrap();
    assert_eq!(idents, vec!["author".to_string(), "category".to_string()]);
}

#[test]
fn relation_idents_ignores_plain_and_request_fields() {
    let idents =
        cratebase_filter::relation_idents(r#"status = "active" && @request.auth.id = owner"#)
            .unwrap();
    assert!(idents.is_empty());
}

#[test]
fn any_of_operator_errors_on_invalid_syntax() {
    assert!(matches!(Parser::parse("a ? 1"), Err(FilterError::Lex(_))));
}

/// A resolver that treats `tags` as a multi-valued JSON-array column and
/// every other identifier as a plain scalar column, mirroring how
/// `cratebase-db`'s `CollectionResolver` distinguishes them.
struct MultiValueResolver;

impl Resolver for MultiValueResolver {
    fn resolve(&self, ident: &str) -> Result<Resolved, FilterError> {
        match ident {
            "tags" => Ok(Resolved::MultiColumn("\"tags\"".to_string())),
            other => Ok(Resolved::Column(format!("\"{other}\""))),
        }
    }
}

fn compile(src: &str) -> CompiledFilter {
    parse_and_compile(src, &MultiValueResolver, Dialect::Sqlite, 0).unwrap()
}

#[test]
fn any_of_compiles_to_exists_over_json_each() {
    let c = compile(r#"tags ?= "rust""#);
    assert_eq!(
        c.sql,
        "EXISTS (SELECT 1 FROM json_each(COALESCE(\"tags\", '[]')) AS __elem WHERE __elem.value = $1)"
    );
    assert_eq!(c.params, vec![serde_json::json!("rust")]);
}

#[test]
fn bare_operator_on_multi_column_requires_every_element() {
    let c = compile(r#"tags = "rust""#);
    assert_eq!(
        c.sql,
        "NOT EXISTS (SELECT 1 FROM json_each(COALESCE(\"tags\", '[]')) AS __elem WHERE NOT (__elem.value = $1))"
    );
}

#[test]
fn plain_scalar_field_unaffected_by_any_of_support() {
    let c = compile(r#"status = "active""#);
    assert_eq!(c.sql, "\"status\" = $1");
}
