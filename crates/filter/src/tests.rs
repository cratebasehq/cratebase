use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::testing::TestResolver;
use super::*;

fn sqlite(src: &str) -> CompiledFilter {
    parse_and_compile(src, &TestResolver::sqlite("posts"), 0).unwrap()
}

fn postgres(src: &str) -> CompiledFilter {
    parse_and_compile(src, &TestResolver::postgres("posts"), 0).unwrap()
}

fn with(src: &str, r: &TestResolver) -> CompiledFilter {
    parse_and_compile(src, r, 0).unwrap()
}

fn err(src: &str) -> FilterError {
    parse_and_compile(src, &TestResolver::sqlite("posts"), 0).unwrap_err()
}

const CATS: &str = "json_each(COALESCE(\"posts\".\"categories\", '[]')) AS \"__e1\"";
const CATS_EMPTY: &str = "(\"posts\".\"categories\" IS NULL OR \"posts\".\"categories\" = '' OR \"posts\".\"categories\" = '[]')";
const TAGS: &str = "json_each(COALESCE(\"posts\".\"tags\", '[]')) AS \"__e1\" JOIN \"tags\" AS \"__r2\" ON \"__r2\".\"id\" = \"__e1\".\"value\"";

// --- lexer / parser -----------------------------------------------------------

#[test]
fn parses_every_operator() {
    for (src, op, any) in [
        ("a = 1", CompareOp::Eq, false),
        ("a != 1", CompareOp::NotEq, false),
        ("a > 1", CompareOp::Gt, false),
        ("a >= 1", CompareOp::Gte, false),
        ("a < 1", CompareOp::Lt, false),
        ("a <= 1", CompareOp::Lte, false),
        ("a ~ 1", CompareOp::Like, false),
        ("a !~ 1", CompareOp::NotLike, false),
        ("a ?= 1", CompareOp::Eq, true),
        ("a ?!= 1", CompareOp::NotEq, true),
        ("a ?> 1", CompareOp::Gt, true),
        ("a ?>= 1", CompareOp::Gte, true),
        ("a ?< 1", CompareOp::Lt, true),
        ("a ?<= 1", CompareOp::Lte, true),
        ("a ?~ 1", CompareOp::Like, true),
        ("a ?!~ 1", CompareOp::NotLike, true),
    ] {
        match Parser::parse(src).unwrap() {
            Expr::Compare {
                op: parsed, any_of, ..
            } => {
                assert_eq!(parsed, op, "{src}");
                assert_eq!(any_of, any, "{src}");
            }
            other => panic!("{src}: {other:?}"),
        }
    }
}

#[test]
fn parses_literals_and_escapes() {
    let lit = |src: &str| match Parser::parse(src).unwrap() {
        Expr::Compare {
            right: Operand::Literal(l),
            ..
        } => l,
        other => panic!("{other:?}"),
    };
    assert_eq!(lit(r#"a = "it's""#), Literal::Str("it's".into()));
    assert_eq!(lit(r#"a = 'it\'s'"#), Literal::Str("it's".into()));
    assert_eq!(
        lit(r#"a = "say \"hi\"""#),
        Literal::Str("say \"hi\"".into())
    );
    assert_eq!(
        lit(r#"a = "back\\slash""#),
        Literal::Str("back\\slash".into())
    );
    assert_eq!(lit(r#"a = "héllo ✓""#), Literal::Str("héllo ✓".into()));
    assert_eq!(lit("a = -5"), Literal::Num(-5.0));
    assert_eq!(lit("a = 1.25"), Literal::Num(1.25));
    assert_eq!(lit("a = true"), Literal::Bool(true));
    assert_eq!(lit("a = false"), Literal::Bool(false));
    assert_eq!(lit("a = null"), Literal::Null);
}

#[test]
fn parses_modifiers_and_calls() {
    let left = |src: &str| match Parser::parse(src).unwrap() {
        Expr::Compare { left, .. } => left,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        left("@request.body.x:isset = true"),
        Operand::Ident {
            path: "@request.body.x".into(),
            modifier: Some(Modifier::IsSet)
        }
    );
    assert_eq!(
        left("tags:length > 1"),
        Operand::Ident {
            path: "tags".into(),
            modifier: Some(Modifier::Length)
        }
    );
    assert_eq!(
        left("tags:each ?= 'a'"),
        Operand::Ident {
            path: "tags".into(),
            modifier: Some(Modifier::Each)
        }
    );
    assert_eq!(
        left("title:lower = 'a'"),
        Operand::Ident {
            path: "title".into(),
            modifier: Some(Modifier::Lower)
        }
    );
    assert_eq!(
        left("geoDistance(loc.lon, loc.lat, 1, -2.5) < 10"),
        Operand::Call {
            name: "geoDistance".into(),
            args: vec![
                Operand::ident("loc.lon"),
                Operand::ident("loc.lat"),
                Operand::Literal(Literal::Num(1.0)),
                Operand::Literal(Literal::Num(-2.5)),
            ]
        }
    );
    assert_eq!(
        left("comments_via_post.title = 'x'"),
        Operand::ident("comments_via_post.title")
    );
}

#[test]
fn parse_errors() {
    assert_eq!(Parser::parse("   ").unwrap_err(), FilterError::Empty);
    assert_eq!(
        Parser::parse("a % 1").unwrap_err(),
        FilterError::Lex(2, '%')
    );
    assert!(matches!(Parser::parse("a ? 1"), Err(FilterError::Lex(..))));
    assert!(matches!(
        Parser::parse("a = \"open"),
        Err(FilterError::Parse(_))
    ));
    assert!(matches!(
        Parser::parse("a = 1 &&"),
        Err(FilterError::Parse(_))
    ));
    assert!(matches!(
        Parser::parse("(a = 1"),
        Err(FilterError::Parse(_))
    ));
    assert!(matches!(
        Parser::parse("a = 1 b = 2"),
        Err(FilterError::Parse(_))
    ));
    assert!(matches!(
        Parser::parse("a:bogus = 1"),
        Err(FilterError::InvalidModifier(_))
    ));
    assert!(matches!(
        Parser::parse("geoDistance(1, 2) < 1"),
        Err(FilterError::Parse(_))
    ));
    assert!(matches!(
        Parser::parse("nope(1) = 1"),
        Err(FilterError::Parse(_))
    ));
    assert!(matches!(Parser::parse("a. = 1"), Err(FilterError::Lex(..))));
}

#[test]
fn precedence_and_parentheses() {
    let c = sqlite(r#"views = 1 && published = true || title = "c""#);
    assert_eq!(
        c.sql,
        "((\"posts\".\"views\" = $1 AND \"posts\".\"published\" = $2) OR \"posts\".\"title\" = $3)"
    );
    assert_eq!(c.params, vec![json!(1), json!(true), json!("c")]);
    let c = sqlite(r#"views = 1 && (published = true || title = "c")"#);
    assert_eq!(
        c.sql,
        "(\"posts\".\"views\" = $1 AND (\"posts\".\"published\" = $2 OR \"posts\".\"title\" = $3))"
    );
}

// --- cache --------------------------------------------------------------------

#[test]
fn parse_cache_returns_shared_ast() {
    let a = parse_cached("title = 'cache-hit' && views > 1").unwrap();
    let b = parse_cached("title = 'cache-hit' && views > 1").unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    let c = parse_cached("title = 'cache-miss'").unwrap();
    assert!(!Arc::ptr_eq(&a, &c));
    assert_eq!(parse_cached("").unwrap_err(), FilterError::Empty);
}

// --- scalar comparisons -------------------------------------------------------

#[test]
fn scalar_operators() {
    assert_eq!(sqlite("title = 'x'").sql, "\"posts\".\"title\" = $1");
    assert_eq!(
        sqlite("title != 'x'").sql,
        "(\"posts\".\"title\" <> $1 OR \"posts\".\"title\" IS NULL)"
    );
    assert_eq!(sqlite("views > 5").sql, "\"posts\".\"views\" > $1");
    assert_eq!(sqlite("views >= 5").sql, "\"posts\".\"views\" >= $1");
    assert_eq!(sqlite("views < 5").sql, "\"posts\".\"views\" < $1");
    assert_eq!(sqlite("views <= 5").sql, "\"posts\".\"views\" <= $1");
    assert_eq!(sqlite("views > 5").params, vec![json!(5)]);
    assert_eq!(sqlite("views > 5.5").params, vec![json!(5.5)]);
    assert_eq!(sqlite("views > -1").params, vec![json!(-1)]);
    assert_eq!(sqlite("id = 'abc'").sql, "\"posts\".\"id\" = $1");
}

#[test]
fn like_wraps_unless_wildcard_present() {
    let c = sqlite("title ~ 'hello'");
    assert_eq!(c.sql, "\"posts\".\"title\" LIKE $1");
    assert_eq!(c.params, vec![json!("%hello%")]);
    assert_eq!(sqlite("title ~ 'he%'").params, vec![json!("he%")]);
    assert_eq!(sqlite("title ~ 'a_b'").params, vec![json!("%a_b%")]);
    assert_eq!(
        sqlite("title !~ 'x'").sql,
        "\"posts\".\"title\" NOT LIKE $1"
    );
    assert_eq!(postgres("title ~ 'x'").sql, "\"posts\".\"title\" ILIKE $1");
    assert_eq!(
        postgres("title !~ 'x'").sql,
        "\"posts\".\"title\" NOT ILIKE $1"
    );
    assert_eq!(sqlite("title ~ 5").params, vec![json!("%5%")]);
    // Column on the pattern side.
    assert_eq!(
        sqlite("title ~ author.name").sql,
        "\"posts\".\"title\" LIKE ('%' || (SELECT \"users\".\"name\" FROM \"users\" WHERE \"users\".\"id\" = \"posts\".\"author\") || '%')"
    );
}

#[test]
fn null_and_empty_string_are_equivalent() {
    let empty = "(\"posts\".\"title\" = '' OR \"posts\".\"title\" IS NULL)";
    assert_eq!(sqlite("title = null").sql, empty);
    assert_eq!(sqlite("title = ''").sql, empty);
    assert_eq!(sqlite("null = title").sql, empty);
    assert_eq!(
        sqlite("title != ''").sql,
        "(\"posts\".\"title\" <> '' AND \"posts\".\"title\" IS NOT NULL)"
    );
    assert_eq!(sqlite("title != null").sql, sqlite("title != ''").sql);
    // Typed columns only test for NULL (keeps Postgres happy).
    assert_eq!(sqlite("views = null").sql, "\"posts\".\"views\" IS NULL");
    assert_eq!(
        sqlite("published != null").sql,
        "\"posts\".\"published\" IS NOT NULL"
    );
    assert!(sqlite("title = ''").params.is_empty());
    // Ordered comparisons against null bind the null.
    assert_eq!(sqlite("views > null").sql, "\"posts\".\"views\" > $1");
    assert_eq!(sqlite("views > null").params, vec![Value::Null]);
}

#[test]
fn column_vs_column_is_null_safe() {
    assert_eq!(
        sqlite("title = author.name").sql,
        "\"posts\".\"title\" IS (SELECT \"users\".\"name\" FROM \"users\" WHERE \"users\".\"id\" = \"posts\".\"author\")"
    );
    assert!(sqlite("title != author.name")
        .sql
        .contains("\"posts\".\"title\" IS NOT (SELECT"));
    assert!(postgres("title = author.name")
        .sql
        .contains("IS NOT DISTINCT FROM"));
    assert!(postgres("title != author.name")
        .sql
        .contains("\"title\" IS DISTINCT FROM"));
    assert_eq!(
        sqlite("views > categories:length").sql,
        "\"posts\".\"views\" > json_array_length(COALESCE(\"posts\".\"categories\", '[]'))"
    );
}

#[test]
fn value_vs_value_is_constant_folded() {
    assert_eq!(sqlite("1 = 1").sql, "1 = 1");
    assert_eq!(sqlite("1 > 2").sql, "1 = 0");
    assert_eq!(sqlite("'a' != 'b'").sql, "1 = 1");
    assert_eq!(sqlite("'Hello' ~ 'ELL'").sql, "1 = 1");
    assert_eq!(sqlite("'Hello' !~ 'ELL'").sql, "1 = 0");
    assert_eq!(sqlite("'' = null").sql, "1 = 1");
    assert!(sqlite("1 = 1").params.is_empty());
}

#[test]
fn param_offset_shifts_placeholders() {
    let c = parse_and_compile(
        "title = 'a' && views > 1",
        &TestResolver::sqlite("posts"),
        2,
    )
    .unwrap();
    assert_eq!(
        c.sql,
        "(\"posts\".\"title\" = $3 AND \"posts\".\"views\" > $4)"
    );
}

// --- multi-valued fields ------------------------------------------------------

#[test]
fn multi_select_any_of_and_all_of() {
    assert_eq!(
        sqlite("categories ?= 'tech'").sql,
        format!("EXISTS (SELECT 1 FROM {CATS} WHERE \"__e1\".\"value\" = $1)")
    );
    assert_eq!(
        sqlite("categories = 'tech'").sql,
        format!("(EXISTS (SELECT 1 FROM {CATS} WHERE \"__e1\".\"value\" = $1) AND NOT EXISTS (SELECT 1 FROM {CATS} WHERE NOT (\"__e1\".\"value\" = $1)))")
    );
    // `!=` tolerates the missing row: every element differs (vacuous on empty).
    assert_eq!(
        sqlite("categories != 'tech'").sql,
        format!("NOT EXISTS (SELECT 1 FROM {CATS} WHERE NOT ((\"__e1\".\"value\" <> $1 OR \"__e1\".\"value\" IS NULL)))")
    );
    assert_eq!(
        sqlite("categories ?!= 'tech'").sql,
        format!("({CATS_EMPTY} OR EXISTS (SELECT 1 FROM {CATS} WHERE (\"__e1\".\"value\" <> $1 OR \"__e1\".\"value\" IS NULL)))")
    );
    assert_eq!(
        sqlite("categories ?~ 'te'").sql,
        format!("EXISTS (SELECT 1 FROM {CATS} WHERE \"__e1\".\"value\" LIKE $1)")
    );
    // Value on the left.
    assert_eq!(
        sqlite("'tech' ?= categories").sql,
        format!("EXISTS (SELECT 1 FROM {CATS} WHERE $1 = \"__e1\".\"value\")")
    );
    // Multi-valued file fields behave the same.
    assert!(sqlite("attachments ?= 'a.png'")
        .sql
        .contains("json_each(COALESCE(\"posts\".\"attachments\", '[]'))"));
}

#[test]
fn multi_field_compared_to_empty_is_an_emptiness_test() {
    assert_eq!(sqlite("categories = ''").sql, CATS_EMPTY);
    assert_eq!(sqlite("categories = null").sql, CATS_EMPTY);
    assert_eq!(sqlite("categories ?= ''").sql, CATS_EMPTY);
    assert_eq!(sqlite("categories != ''").sql, format!("NOT {CATS_EMPTY}"));
    assert_eq!(
        sqlite("categories ?!= null").sql,
        format!("NOT {CATS_EMPTY}")
    );
    assert_eq!(
        postgres("categories = ''").sql,
        "(\"posts\".\"categories\" IS NULL OR \"posts\".\"categories\"::text = '' OR \"posts\".\"categories\"::text = '[]')"
    );
}

#[test]
fn postgres_multi_uses_jsonb_array_elements() {
    assert_eq!(
        postgres("categories ?= 'tech'").sql,
        "EXISTS (SELECT 1 FROM jsonb_array_elements_text(COALESCE(\"posts\".\"categories\", '[]')::jsonb) AS \"__e1\"(\"value\") WHERE \"__e1\".\"value\" = $1)"
    );
}

#[test]
fn modifiers_each_length_lower_isset() {
    assert_eq!(
        sqlite("categories:each ?= 'tech'").sql,
        sqlite("categories ?= 'tech'").sql
    );
    assert_eq!(
        sqlite("categories:length > 1").sql,
        "json_array_length(COALESCE(\"posts\".\"categories\", '[]')) > $1"
    );
    assert_eq!(
        postgres("categories:length > 1").sql,
        "jsonb_array_length(COALESCE(\"posts\".\"categories\", '[]')::jsonb) > $1"
    );
    assert_eq!(
        sqlite("tags:length = 0").sql,
        "json_array_length(COALESCE(\"posts\".\"tags\", '[]')) = $1"
    );
    let c = sqlite("title:lower = 'FoO'");
    assert_eq!(c.sql, "LOWER(\"posts\".\"title\") = $1");
    assert_eq!(c.params, vec![json!("foo")]);
    let c = sqlite("title:lower ~ 'FOO'");
    assert_eq!(c.sql, "LOWER(\"posts\".\"title\") LIKE $1");
    assert_eq!(c.params, vec![json!("%foo%")]);
    assert_eq!(sqlite("'FOO' = title:lower").params, vec![json!("foo")]);
    assert_eq!(
        sqlite("categories:lower ?= 'TECH'").sql,
        format!("EXISTS (SELECT 1 FROM {CATS} WHERE LOWER(\"__e1\".\"value\") = $1)")
    );
    assert_eq!(
        sqlite("categories:lower ?= 'TECH'").params,
        vec![json!("tech")]
    );

    assert!(matches!(
        err("title:each = 'a'"),
        FilterError::InvalidModifier(_)
    ));
    assert!(matches!(
        err("title:length = 1"),
        FilterError::InvalidModifier(_)
    ));
    assert!(matches!(
        err("title:isset = true"),
        FilterError::InvalidModifier(_)
    ));
    assert!(matches!(
        err("@request.auth.id:isset = true"),
        FilterError::InvalidModifier(_)
    ));
    assert!(matches!(
        err("@now:lower = ''"),
        FilterError::InvalidModifier(_)
    ));

    let r = TestResolver::sqlite("posts").with_body(json!({"title": "x", "tags": ["a", "b"]}));
    assert_eq!(with("@request.body.title:isset = true", &r).sql, "1 = 1");
    assert_eq!(with("@request.body.missing:isset = true", &r).sql, "1 = 0");
    assert_eq!(with("@request.body.missing:isset = false", &r).sql, "1 = 1");
    assert_eq!(with("@request.body.tags:length = 2", &r).sql, "1 = 1");
    assert_eq!(with("@request.body.tags:each ?= 'a'", &r).sql, "1 = 1");
    assert_eq!(with("@request.body.tags:each = 'a'", &r).sql, "1 = 0");
    assert_eq!(with("@request.body.tags:each != 'zzz'", &r).sql, "1 = 1");
    assert_eq!(with("@request.body.title:lower = 'X'", &r).sql, "1 = 1");
    // A `:each` value against a column enumerates the bound JSON array.
    let c = with("@request.body.tags:each ?= title", &r);
    assert_eq!(
        c.sql,
        "EXISTS (SELECT 1 FROM json_each(COALESCE($1, '[]')) AS \"__e1\" WHERE \"__e1\".\"value\" IS \"posts\".\"title\")"
    );
    assert_eq!(c.params, vec![json!("[\"a\",\"b\"]")]);
    assert!(matches!(
        parse_and_compile("@request.body.tags:each ?= categories", &r, 0).unwrap_err(),
        FilterError::Unsupported(_)
    ));
}

// --- relations ----------------------------------------------------------------

#[test]
fn single_relation_depth_1_to_3() {
    assert_eq!(
        sqlite("author.name = 'Alice'").sql,
        "(SELECT \"users\".\"name\" FROM \"users\" WHERE \"users\".\"id\" = \"posts\".\"author\") = $1"
    );
    assert_eq!(
        sqlite("author.company.name = 'ACME'").sql,
        "(SELECT \"companies\".\"name\" FROM \"companies\" WHERE \"companies\".\"id\" = (SELECT \"users\".\"company\" FROM \"users\" WHERE \"users\".\"id\" = \"posts\".\"author\")) = $1"
    );
    let c = with(
        "post.author.company.name = 'ACME'",
        &TestResolver::sqlite("comments"),
    );
    assert_eq!(
        c.sql,
        "(SELECT \"companies\".\"name\" FROM \"companies\" WHERE \"companies\".\"id\" = (SELECT \"users\".\"company\" FROM \"users\" WHERE \"users\".\"id\" = (SELECT \"posts\".\"author\" FROM \"posts\" WHERE \"posts\".\"id\" = \"comments\".\"post\"))) = $1"
    );
    assert!(c.joins.is_empty());
    // A single relation compared directly is a plain column.
    assert_eq!(sqlite("author = 'u1'").sql, "\"posts\".\"author\" = $1");
    assert_eq!(sqlite("author.id = 'u1'").sql, "(SELECT \"users\".\"id\" FROM \"users\" WHERE \"users\".\"id\" = \"posts\".\"author\") = $1");
}

#[test]
fn multi_relation_paths() {
    assert_eq!(
        sqlite("tags.name ?= 'rust'").sql,
        format!("EXISTS (SELECT 1 FROM {TAGS} WHERE \"__r2\".\"name\" = $1)")
    );
    assert_eq!(
        sqlite("tags.name = 'rust'").sql,
        format!("(EXISTS (SELECT 1 FROM {TAGS} WHERE \"__r2\".\"name\" = $1) AND NOT EXISTS (SELECT 1 FROM {TAGS} WHERE NOT (\"__r2\".\"name\" = $1)))")
    );
    assert_eq!(
        sqlite("tags.name ?!= 'rust'").sql,
        format!("(NOT EXISTS (SELECT 1 FROM {TAGS}) OR EXISTS (SELECT 1 FROM {TAGS} WHERE (\"__r2\".\"name\" <> $1 OR \"__r2\".\"name\" IS NULL)))")
    );
    // Empty compare through a relation: every related name is empty.
    assert_eq!(
        sqlite("tags.name = ''").sql,
        format!("NOT EXISTS (SELECT 1 FROM {TAGS} WHERE NOT ((\"__r2\".\"name\" = '' OR \"__r2\".\"name\" IS NULL)))")
    );
    // Multi relation then single relation: subquery relative to the row alias.
    assert_eq!(
        sqlite("tags.owner.name ?= 'Bob'").sql,
        format!("EXISTS (SELECT 1 FROM {TAGS} WHERE (SELECT \"users\".\"name\" FROM \"users\" WHERE \"users\".\"id\" = \"__r2\".\"owner\") = $1)")
    );
    // Multi of multi.
    assert_eq!(
        sqlite("tags.related.name ?= 'x'").sql,
        format!("EXISTS (SELECT 1 FROM {TAGS}, json_each(COALESCE(\"__r2\".\"related\", '[]')) AS \"__e3\" JOIN \"tags\" AS \"__r4\" ON \"__r4\".\"id\" = \"__e3\".\"value\" WHERE \"__r4\".\"name\" = $1)")
    );
    // Multi relation ending in a multi select.
    assert_eq!(
        sqlite("tags.aliases ?= 'a'").sql,
        format!("EXISTS (SELECT 1 FROM {TAGS}, json_each(COALESCE(\"__r2\".\"aliases\", '[]')) AS \"__e3\" WHERE \"__e3\".\"value\" = $1)")
    );
    // Multi relation compared directly: element semantics on the ids.
    assert_eq!(
        sqlite("tags ?= 't1'").sql,
        "EXISTS (SELECT 1 FROM json_each(COALESCE(\"posts\".\"tags\", '[]')) AS \"__e1\" WHERE \"__e1\".\"value\" = $1)"
    );
    // Multi relation against a column of the root row.
    assert_eq!(
        sqlite("tags.name ?= title").sql,
        format!("EXISTS (SELECT 1 FROM {TAGS} WHERE \"__r2\".\"name\" IS \"posts\".\"title\")")
    );
    assert!(sqlite("tags.name = 'x'").joins.is_empty());
    assert!(postgres("tags.name ?= 'x'").sql.contains("jsonb_array_elements_text(COALESCE(\"posts\".\"tags\", '[]')::jsonb) AS \"__e1\"(\"value\") JOIN \"tags\" AS \"__r2\""));
}

#[test]
fn back_relations_emit_one_left_join() {
    let c = sqlite("comments_via_post.title = 'x' && comments_via_post.author = 'u1'");
    assert_eq!(
        c.sql,
        "(\"posts_comments_via_post\".\"title\" = $1 AND \"posts_comments_via_post\".\"author\" = $2)"
    );
    assert_eq!(c.joins.len(), 1);
    assert_eq!(c.joins[0].key, "posts_comments_via_post");
    assert_eq!(
        c.joins[0].sql,
        "LEFT JOIN \"comments\" AS \"posts_comments_via_post\" ON \"posts_comments_via_post\".\"post\" = \"posts\".\"id\""
    );
    // Back-relation through a multi-valued relation field.
    let c = sqlite("memberships_via_projects.team = 'core'");
    assert_eq!(
        c.joins[0].sql,
        "LEFT JOIN \"memberships\" AS \"posts_memberships_via_projects\" ON EXISTS (SELECT 1 FROM json_each(COALESCE(\"posts_memberships_via_projects\".\"projects\", '[]')) AS \"__e1\" WHERE \"__e1\".\"value\" = \"posts\".\"id\")"
    );
    assert!(postgres("memberships_via_projects.team = 'core'").joins[0]
        .sql
        .contains("jsonb_array_elements_text"));
    // Continue through forward relations after the join.
    assert_eq!(
        sqlite("comments_via_post.author.name = 'x'").sql,
        "(SELECT \"users\".\"name\" FROM \"users\" WHERE \"users\".\"id\" = \"posts_comments_via_post\".\"author\") = $1"
    );
    assert_eq!(
        sqlite("comments_via_post.tags.name ?= 'x'").sql,
        "EXISTS (SELECT 1 FROM json_each(COALESCE(\"posts_comments_via_post\".\"tags\", '[]')) AS \"__e1\" JOIN \"tags\" AS \"__r2\" ON \"__r2\".\"id\" = \"__e1\".\"value\" WHERE \"__r2\".\"name\" = $1)"
    );
    // Back-relation reached through a single relation joins on the subquery.
    let c = sqlite("author.posts_via_author.title = 'x'");
    assert_eq!(c.joins[0].key, "posts_author_posts_via_author");
    assert_eq!(
        c.joins[0].sql,
        "LEFT JOIN \"posts\" AS \"posts_author_posts_via_author\" ON \"posts_author_posts_via_author\".\"author\" = \"posts\".\"author\""
    );
    // Back-relation inside a multi relation becomes an inner join of the element set.
    let c = sqlite("tags.posts_via_tags.title ?= 'x'");
    assert_eq!(
        c.sql,
        format!("EXISTS (SELECT 1 FROM {TAGS} JOIN \"posts\" AS \"__b3\" ON EXISTS (SELECT 1 FROM json_each(COALESCE(\"__b3\".\"tags\", '[]')) AS \"__e4\" WHERE \"__e4\".\"value\" = \"__r2\".\"id\") WHERE \"__b3\".\"title\" = $1)")
    );
    assert!(c.joins.is_empty());

    assert_eq!(
        err("comments_via_title.x = 1"),
        FilterError::UnknownField("comments_via_title.x".into())
    );
    assert_eq!(
        err("comments_via_post = 'x'"),
        FilterError::UnknownField("comments_via_post".into())
    );
    // users.company points at companies, not posts.
    assert_eq!(
        err("users_via_company.name = 'x'"),
        FilterError::UnknownField("users_via_company.name".into())
    );
    assert_eq!(
        err("nope_via_post.title = 'x'"),
        FilterError::UnknownField("nope_via_post.title".into())
    );
}

#[test]
fn collection_macro_shares_one_join() {
    let r = TestResolver::sqlite("posts").with_auth(json!({"id": "u1"}));
    let c = with(
        "@collection.memberships.user = @request.auth.id && @collection.memberships.team = title",
        &r,
    );
    assert_eq!(
        c.sql,
        "(\"__collection_memberships\".\"user\" = $1 AND \"__collection_memberships\".\"team\" IS \"posts\".\"title\")"
    );
    assert_eq!(c.params, vec![json!("u1")]);
    assert_eq!(c.joins.len(), 1);
    assert_eq!(c.joins[0].key, "__collection_memberships");
    assert_eq!(
        c.joins[0].sql,
        "LEFT JOIN \"memberships\" AS \"__collection_memberships\" ON 1=1"
    );
    // Lookup by id works too, and multi-valued fields keep element semantics.
    let by_id = format!(
        "@collection.{}.roles ?= 'admin'",
        r.collection("memberships").unwrap().id
    );
    assert_eq!(
        with(&by_id, &r).sql,
        "EXISTS (SELECT 1 FROM json_each(COALESCE(\"__collection_memberships\".\"roles\", '[]')) AS \"__e1\" WHERE \"__e1\".\"value\" = $1)"
    );
    assert_eq!(
        with("@collection.memberships.roles:length > 0", &r).sql,
        "json_array_length(COALESCE(\"__collection_memberships\".\"roles\", '[]')) > $1"
    );
    // Relations from the joined collection.
    assert_eq!(
        with("@collection.memberships.user.name = 'x'", &r).sql,
        "(SELECT \"users\".\"name\" FROM \"users\" WHERE \"users\".\"id\" = \"__collection_memberships\".\"user\") = $1"
    );
    assert_eq!(
        err("@collection.nope.x = 1"),
        FilterError::UnknownField("@collection.nope.x".into())
    );
    assert_eq!(
        err("@collection.memberships.nope = 1"),
        FilterError::UnknownField("@collection.memberships.nope".into())
    );
    assert_eq!(
        err("@collection.memberships = 1"),
        FilterError::UnknownMacro("@collection.memberships".into())
    );
}

#[test]
fn unknown_fields_and_depth_limit() {
    assert_eq!(err("nope = 1"), FilterError::UnknownField("nope".into()));
    assert_eq!(
        err("author.nope = 1"),
        FilterError::UnknownField("author.nope".into())
    );
    assert_eq!(
        err("title.foo = 1"),
        FilterError::UnknownField("title.foo".into())
    );
    assert_eq!(
        err("loc.foo = 1"),
        FilterError::UnknownField("loc.foo".into())
    );
    assert_eq!(
        err("collectionName = 'posts'"),
        FilterError::UnknownField("collectionName".into())
    );

    let r = TestResolver::sqlite("tags");
    let six = "related.related.related.related.related.related.name = 'x'";
    assert!(parse_and_compile(six, &r, 0).is_ok());
    let seven = "related.related.related.related.related.related.related.name = 'x'";
    assert!(matches!(
        parse_and_compile(seven, &r, 0),
        Err(FilterError::Unsupported(_))
    ));
}

// --- macros and @request ------------------------------------------------------

#[test]
fn date_macros_use_the_resolver_clock() {
    let param = |src: &str| sqlite(src).params[0].clone();
    assert_eq!(
        param("published_at < @now"),
        json!("2026-09-03 12:44:06.146Z")
    );
    assert_eq!(param("views = @second"), json!(6));
    assert_eq!(param("views = @minute"), json!(44));
    assert_eq!(param("views = @hour"), json!(12));
    assert_eq!(param("views = @weekday"), json!(4)); // Thursday
    assert_eq!(param("views = @day"), json!(3));
    assert_eq!(param("views = @month"), json!(9));
    assert_eq!(param("views = @year"), json!(2026));
    assert_eq!(
        param("published_at > @yesterday"),
        json!("2026-09-02 12:44:06.146Z")
    );
    assert_eq!(
        param("published_at < @tomorrow"),
        json!("2026-09-04 12:44:06.146Z")
    );
    assert_eq!(
        param("published_at >= @todayStart"),
        json!("2026-09-03 00:00:00.000Z")
    );
    assert_eq!(
        param("published_at <= @todayEnd"),
        json!("2026-09-03 23:59:59.999Z")
    );
    assert_eq!(
        param("published_at >= @monthStart"),
        json!("2026-09-01 00:00:00.000Z")
    );
    assert_eq!(
        param("published_at <= @monthEnd"),
        json!("2026-09-30 23:59:59.999Z")
    );
    assert_eq!(
        param("published_at >= @yearStart"),
        json!("2026-01-01 00:00:00.000Z")
    );
    assert_eq!(
        param("published_at <= @yearEnd"),
        json!("2026-12-31 23:59:59.999Z")
    );
    assert_eq!(
        sqlite("published_at < @now").sql,
        "\"posts\".\"published_at\" < $1"
    );
    assert_eq!(
        err("published_at < @nope"),
        FilterError::UnknownMacro("@nope".into())
    );
    assert_eq!(
        err("views = @request"),
        FilterError::UnknownMacro("@request".into())
    );
    assert_eq!(
        err("views = @request.foo"),
        FilterError::UnknownMacro("@request.foo".into())
    );
    assert_eq!(
        err("views = @request.body"),
        FilterError::UnknownMacro("@request.body".into())
    );

    // Month/year boundaries.
    let dec = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2024, 12, 15, 1, 2, 3).unwrap();
    assert_eq!(
        date_macro("monthEnd", dec).unwrap(),
        json!("2024-12-31 23:59:59.999Z")
    );
    let feb = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2024, 2, 1, 0, 0, 0).unwrap();
    assert_eq!(
        date_macro("monthEnd", feb).unwrap(),
        json!("2024-02-29 23:59:59.999Z")
    );
    assert_eq!(date_macro("weekday", feb).unwrap(), json!(4));
    assert_eq!(DATE_MACROS.len(), 16);
    for name in DATE_MACROS {
        assert!(date_macro(name, feb).is_some(), "{name}");
    }
}

#[test]
fn request_paths() {
    let r = TestResolver::sqlite("posts")
        .with_auth(
            json!({"id": "u1", "verified": true, "collectionName": "users", "email": "A@B.c"}),
        )
        .with_body(json!({"title": "T", "nested": {"k": "v"}}))
        .with_query(json!({"page": 2}))
        .with_headers(json!({"x_token": "tok"}))
        .with_method("POST")
        .with_context("realtime");
    let c = with("author = @request.auth.id", &r);
    assert_eq!(c.sql, "\"posts\".\"author\" = $1");
    assert_eq!(c.params, vec![json!("u1")]);
    assert_eq!(with("@request.auth.id != ''", &r).sql, "1 = 1");
    assert_eq!(with("@request.auth = true", &r).sql, "1 = 1");
    assert_eq!(with("@request.auth.verified = true", &r).sql, "1 = 1");
    assert_eq!(
        with("@request.auth.collectionName = 'users'", &r).sql,
        "1 = 1"
    );
    assert_eq!(with("@request.auth.email:lower = 'a@b.c'", &r).sql, "1 = 1");
    assert_eq!(
        with("title = @request.body.title", &r).params,
        vec![json!("T")]
    );
    assert_eq!(
        with("title = @request.data.title", &r).params,
        vec![json!("T")]
    );
    assert_eq!(
        with("title = @request.body.nested.k", &r).params,
        vec![json!("v")]
    );
    assert_eq!(with("@request.query.page = 2", &r).sql, "1 = 1");
    assert_eq!(with("@request.headers.x_token = 'tok'", &r).sql, "1 = 1");
    assert_eq!(with("@request.method = 'POST'", &r).sql, "1 = 1");
    assert_eq!(with("@request.method = 'GET'", &r).sql, "1 = 0");
    assert_eq!(with("@request.context = 'realtime'", &r).sql, "1 = 1");
    // Missing values are empty.
    assert_eq!(with("@request.body.nope = ''", &r).sql, "1 = 1");
    assert_eq!(
        with("title = @request.body.nope", &r).sql,
        "(\"posts\".\"title\" = '' OR \"posts\".\"title\" IS NULL)"
    );

    // Unauthenticated: @request.auth.* is empty, like PocketBase.
    let anon = TestResolver::sqlite("posts");
    assert_eq!(with("@request.auth.id != ''", &anon).sql, "1 = 0");
    assert_eq!(with("@request.auth = true", &anon).sql, "1 = 0");
    assert_eq!(
        with("author = @request.auth.id", &anon).sql,
        "(\"posts\".\"author\" = '' OR \"posts\".\"author\" IS NULL)"
    );
    assert_eq!(
        with("@request.auth.id != '' && author = @request.auth.id", &anon).sql,
        "(1 = 0 AND (\"posts\".\"author\" = '' OR \"posts\".\"author\" IS NULL))"
    );
}

// --- json, geo ----------------------------------------------------------------

#[test]
fn json_paths() {
    assert_eq!(
        sqlite("data.some.key = 'v'").sql,
        "json_extract(\"posts\".\"data\", '$.some.key') = $1"
    );
    assert_eq!(
        sqlite("data.items.0 = 1").sql,
        "json_extract(\"posts\".\"data\", '$.items[0]') = $1"
    );
    assert_eq!(
        postgres("data.some.key = 'v'").sql,
        "(\"posts\".\"data\"::jsonb #>> '{some,key}') = $1"
    );
    assert_eq!(
        postgres("data.items.0 > 1").sql,
        "CAST((\"posts\".\"data\"::jsonb #>> '{items,0}') AS double precision) > $1"
    );
    let c = postgres("data.flag = true");
    assert_eq!(c.sql, "(\"posts\".\"data\"::jsonb #>> '{flag}') = $1");
    assert_eq!(c.params, vec![json!("true")]);
    assert_eq!(sqlite("data.flag = true").params, vec![json!(true)]);
    assert_eq!(
        sqlite("data = ''").sql,
        "(json_extract(\"posts\".\"data\", '$') = '' OR json_extract(\"posts\".\"data\", '$') IS NULL)"
    );
    assert_eq!(
        sqlite("data = 'x'").sql,
        "json_extract(\"posts\".\"data\", '$') = $1"
    );
    assert_eq!(
        postgres("data = 'x'").sql,
        "(\"posts\".\"data\"::jsonb #>> '{}') = $1"
    );
    assert_eq!(
        sqlite("author.company.name = data.a").sql,
        "(SELECT \"companies\".\"name\" FROM \"companies\" WHERE \"companies\".\"id\" = (SELECT \"users\".\"company\" FROM \"users\" WHERE \"users\".\"id\" = \"posts\".\"author\")) IS json_extract(\"posts\".\"data\", '$.a')"
    );
}

#[test]
fn geo_distance() {
    let c = sqlite("geoDistance(loc.lon, loc.lat, 10.5, 20) < 100");
    assert_eq!(
        c.sql,
        "geoDistance(json_extract(\"posts\".\"loc\", '$.lon'), json_extract(\"posts\".\"loc\", '$.lat'), $1, $2) < $3"
    );
    assert_eq!(c.params, vec![json!(10.5), json!(20), json!(100)]);
    let c = postgres("geoDistance(loc.lon, loc.lat, 10.5, 20) < 100");
    assert_eq!(
        c.sql,
        "(6371 * acos(LEAST(1.0, GREATEST(-1.0, sin(radians((\"posts\".\"loc\"::jsonb->>'lat')::double precision)) * sin(radians($2)) + cos(radians((\"posts\".\"loc\"::jsonb->>'lat')::double precision)) * cos(radians($2)) * cos(radians($1) - radians((\"posts\".\"loc\"::jsonb->>'lon')::double precision)))))) < $3"
    );
    // Query values that look numeric are bound as numbers.
    let r = TestResolver::sqlite("posts").with_query(json!({"lon": "1.5", "lat": "2"}));
    let c = with(
        "geoDistance(loc.lon, loc.lat, @request.query.lon, @request.query.lat) <= 5",
        &r,
    );
    assert_eq!(c.params, vec![json!(1.5), json!(2), json!(5)]);
    assert_eq!(
        sqlite("loc.lat > 1").sql,
        "json_extract(\"posts\".\"loc\", '$.lat') > $1"
    );
    assert!(matches!(
        err("geoDistance(tags.name, 1, 2, 3) < 1"),
        FilterError::Unsupported(_)
    ));
}

// --- sort ---------------------------------------------------------------------

#[test]
fn sort_paths() {
    let r = TestResolver::sqlite("posts");
    assert_eq!(
        resolve_sort_path(&r, "title").unwrap(),
        "\"posts\".\"title\""
    );
    assert_eq!(
        resolve_sort_path(&r, "created").unwrap(),
        "\"posts\".\"created\""
    );
    assert_eq!(
        resolve_sort_path(&r, "author.name").unwrap(),
        "(SELECT \"users\".\"name\" FROM \"users\" WHERE \"users\".\"id\" = \"posts\".\"author\")"
    );
    assert_eq!(
        resolve_sort_path(&r, "data.key").unwrap(),
        "json_extract(\"posts\".\"data\", '$.key')"
    );
    assert_eq!(
        resolve_sort_path(&r, "categories").unwrap(),
        "\"posts\".\"categories\""
    );
    assert!(matches!(
        resolve_sort_path(&r, "tags.name"),
        Err(FilterError::Unsupported(_))
    ));
    assert!(matches!(
        resolve_sort_path(&r, "comments_via_post.title"),
        Err(FilterError::Unsupported(_))
    ));
    assert!(matches!(
        resolve_sort_path(&r, "@random"),
        Err(FilterError::Unsupported(_))
    ));
    assert!(matches!(
        resolve_sort_path(&r, "title:lower"),
        Err(FilterError::Unsupported(_))
    ));
    assert_eq!(
        resolve_sort_path(&r, "nope").unwrap_err(),
        FilterError::UnknownField("nope".into())
    );
}

// --- evaluator ----------------------------------------------------------------

fn record() -> Map<String, Value> {
    json!({
        "id": "p1",
        "title": "Hello World",
        "status": "",
        "author": "u1",
        "tags": ["t1", "t2"],
        "categories": ["tech", "news"],
        "views": 5,
        "published": true,
        "data": {"a": {"b": 1}, "flag": true, "list": ["x", "y"]},
        "loc": {"lon": 1.5, "lat": 2},
        "published_at": "2026-09-01 00:00:00.000Z",
    })
    .as_object()
    .cloned()
    .unwrap()
}

fn eval(src: &str, r: &TestResolver) -> Result<bool, FilterError> {
    evaluate(&Parser::parse(src).unwrap(), &record(), r)
}

#[test]
fn evaluator_table() {
    let r = TestResolver::sqlite("posts")
        .with_auth(json!({"id": "u1"}))
        .with_body(json!({"title": "x", "tags": ["a", "b"]}));
    let cases: &[(&str, bool)] = &[
        ("title = 'Hello World'", true),
        ("title = 'hello world'", false),
        ("title:lower = 'hello world'", true),
        ("'HELLO WORLD' = title:lower", true),
        ("'HELLO WORLD' = title", false),
        ("title != 'x'", true),
        ("title ~ 'ell'", true),
        ("title ~ 'ELL'", true),
        ("title ~ 'H%'", true),
        ("title ~ 'x%'", false),
        ("title ~ 'H_llo%'", true),
        ("title !~ 'x'", true),
        ("title !~ 'ell'", false),
        ("views > 3", true),
        ("views >= 5", true),
        ("views < 5", false),
        ("views <= 5", true),
        ("views = '5'", true),
        ("views = 5.0", true),
        ("views > -1", true),
        ("status = ''", true),
        ("status = null", true),
        ("status != ''", false),
        ("status != 'x'", true),
        ("status > ''", false),
        ("title != ''", true),
        ("attachments = ''", true),
        ("attachments = 'x'", false),
        ("attachments != 'x'", true),
        ("attachments ?= 'x'", false),
        ("attachments ?!= 'x'", true),
        ("attachments:length = 0", true),
        ("categories ?= 'tech'", true),
        ("categories = 'tech'", false),
        ("categories != 'life'", true),
        ("categories != 'tech'", false),
        ("categories ?!= 'tech'", true),
        ("categories ?~ 'ew'", true),
        ("categories ~ 'e'", true),
        ("categories:each ?= 'news'", true),
        ("categories:length = 2", true),
        ("categories:length > 2", false),
        ("categories = ''", false),
        ("categories != ''", true),
        ("categories:lower ?= 'TECH'", true),
        ("'tech' ?= categories", true),
        ("tags ?= 't2'", true),
        ("tags = 't1'", false),
        ("published = true", true),
        ("published != true", false),
        ("published = 1", true),
        ("data.a.b = 1", true),
        ("data.flag = true", true),
        ("data.list.1 = 'y'", true),
        ("data.nope = ''", true),
        ("loc.lat = 2", true),
        ("loc.lon > 1", true),
        ("published_at < @now", true),
        ("published_at >= @todayStart", false),
        ("published_at >= @monthStart", true),
        ("@request.auth.id = author", true),
        ("author = @request.auth.id", true),
        ("@request.auth.id != ''", true),
        ("@request.auth = true", true),
        ("@request.body.title:isset = true", true),
        ("@request.body.nope:isset = true", false),
        ("@request.body.tags:each ?= 'a'", true),
        ("@request.body.tags:each = 'a'", false),
        ("@request.body.tags:length = 2", true),
        ("@request.body.title = 'x' && views > 10", false),
        ("@request.body.title = 'x' || views > 10", true),
        (
            "(title = 'a' || title = 'Hello World') && published = true",
            true,
        ),
        ("@request.method = 'GET'", true),
        ("1 = 1", true),
        ("'' = null", true),
        ("id = 'p1'", true),
    ];
    for (src, expected) in cases {
        assert_eq!(eval(src, &r), Ok(*expected), "{src}");
    }
    let anon = TestResolver::sqlite("posts");
    assert_eq!(eval("@request.auth.id != ''", &anon), Ok(false));
    assert_eq!(eval("author = @request.auth.id", &anon), Ok(false));
    assert_eq!(eval("status = @request.auth.id", &anon), Ok(true));
}

#[test]
fn evaluator_reports_unsupported_and_unknown() {
    let r = TestResolver::sqlite("posts");
    assert!(matches!(
        eval("author.name = 'x'", &r),
        Err(FilterError::Unsupported(_))
    ));
    assert!(matches!(
        eval("tags.name ?= 'x'", &r),
        Err(FilterError::Unsupported(_))
    ));
    assert!(matches!(
        eval("comments_via_post.title = 'x'", &r),
        Err(FilterError::Unsupported(_))
    ));
    assert!(matches!(
        eval("@collection.memberships.user = 'x'", &r),
        Err(FilterError::Unsupported(_))
    ));
    assert!(matches!(
        eval("geoDistance(loc.lon, loc.lat, 1, 2) < 5", &r),
        Err(FilterError::Unsupported(_))
    ));
    // Unsupported surfaces even when the other branch decides the result.
    assert!(matches!(
        eval("views = 5 || author.name = 'x'", &r),
        Err(FilterError::Unsupported(_))
    ));
    assert_eq!(
        eval("nope = 1", &r),
        Err(FilterError::UnknownField("nope".into()))
    );
    assert_eq!(
        eval("title.x = 1", &r),
        Err(FilterError::UnknownField("title.x".into()))
    );
    assert!(matches!(
        eval("title:each = 'a'", &r),
        Err(FilterError::InvalidModifier(_))
    ));
    assert!(matches!(
        eval("title:isset = true", &r),
        Err(FilterError::InvalidModifier(_))
    ));
    assert_eq!(
        eval("views = @nope", &r),
        Err(FilterError::UnknownMacro("@nope".into()))
    );
}

#[test]
fn evaluator_agrees_with_constant_folding() {
    // Every value-vs-value comparison folds to the evaluator's answer.
    let r = TestResolver::sqlite("posts")
        .with_auth(json!({"id": "u1", "name": "Ann"}))
        .with_body(json!({"tags": ["a", "b"], "n": 3, "empty": []}));
    for src in [
        "@request.auth.id = 'u1'",
        "@request.auth.id != 'u1'",
        "@request.auth.name ~ 'an'",
        "@request.auth.name !~ 'zz'",
        "@request.auth.missing = ''",
        "@request.auth.missing != ''",
        "@request.body.n > 2",
        "@request.body.n >= 4",
        "@request.body.n < '10'",
        "@request.body.tags:each ?= 'b'",
        "@request.body.tags:each = 'b'",
        "@request.body.tags:each != 'z'",
        "@request.body.tags:each ?!= 'a'",
        "@request.body.empty:each = ''",
        "@request.body.empty:each != 'a'",
        "@request.body.empty:each ?= 'a'",
        "@request.body.empty:each ?!= 'a'",
        "@request.body.tags:length = 2",
        "@weekday = 4",
        "@now > @yesterday",
        "@todayEnd > @now",
    ] {
        let folded = with(src, &r).sql == "1 = 1";
        let evaluated = eval(src, &r).unwrap();
        assert_eq!(folded, evaluated, "{src}");
    }
}

#[test]
fn like_matcher_semantics() {
    use super::eval::like_match;
    assert!(like_match("Hello", "%ell%"));
    assert!(like_match("Hello", "h%"));
    assert!(like_match("Hello", "%o"));
    assert!(like_match("Hello", "h_llo"));
    assert!(like_match("Hello", "%"));
    assert!(like_match("", "%"));
    assert!(!like_match("", "_"));
    assert!(!like_match("Hello", "hello_"));
    assert!(like_match("a%b", "a%b"));
    assert!(like_match("héllo", "H_LLO"));
    assert!(like_match("abcabc", "%abc"));
    assert!(!like_match("abcab", "%abc"));
}

#[test]
fn request_path_parsing() {
    assert_eq!(RequestPath::parse(&["auth"]), Some(RequestPath::Auth(None)));
    assert_eq!(
        RequestPath::parse(&["auth", "id"]),
        Some(RequestPath::Auth(Some("id".into())))
    );
    assert_eq!(
        RequestPath::parse(&["auth", "a", "b"]),
        Some(RequestPath::Auth(Some("a.b".into())))
    );
    assert_eq!(
        RequestPath::parse(&["body", "x"]),
        Some(RequestPath::Body("x".into()))
    );
    assert_eq!(
        RequestPath::parse(&["data", "x", "y"]),
        Some(RequestPath::Body("x.y".into()))
    );
    assert_eq!(
        RequestPath::parse(&["query", "q"]),
        Some(RequestPath::Query("q".into()))
    );
    assert_eq!(
        RequestPath::parse(&["headers", "x_h"]),
        Some(RequestPath::Headers("x_h".into()))
    );
    assert_eq!(RequestPath::parse(&["method"]), Some(RequestPath::Method));
    assert_eq!(RequestPath::parse(&["context"]), Some(RequestPath::Context));
    assert_eq!(RequestPath::parse(&["method", "x"]), None);
    assert_eq!(RequestPath::parse(&["body"]), None);
    assert_eq!(RequestPath::parse(&["nope"]), None);
    assert_eq!(RequestPath::parse(&[]), None);
}
