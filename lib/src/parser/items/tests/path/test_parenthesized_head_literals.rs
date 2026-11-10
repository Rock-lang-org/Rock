use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

fn parse_expression_debug(source: &str) -> String {
    let tokens = lex_test(source);
    let config = Config::default();
    let (rest, expr) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap_or_else(|_| panic!("parse failed for {source:?}"));
    assert_eq!(rest.len(), 0, "unparsed tokens for {source:?}");
    format!("{expr:?}")
}

/// A number after a comma in a parenthesized path head is a literal type argument.
#[test]
fn test_parenthesized_path_head_accepts_literal_after_comma() {
    let rendered = parse_expression_debug("(Buf I64, 3)::new [1, 2, 3]");
    assert!(rendered.contains("ConstValue"), "{rendered}");
}

#[test]
fn test_parenthesized_path_head_accepts_several_literals_after_commas() {
    let rendered = parse_expression_debug("(Grid I64, 3, 4)::new");
    assert_eq!(rendered.matches("ConstValue").count(), 2, "{rendered}");
}

/// Numbers that are not preceded by a comma stay expression arguments.
#[test]
fn test_parenthesized_path_head_keeps_numbers_as_expressions() {
    for source in [
        "(1).show!",
        "(1)",
        "keep (Maybe::Some 7)",
        "(Maybe::Some 7, 3)",
    ] {
        let rendered = parse_expression_debug(source);
        assert!(
            !rendered.contains("ConstValue"),
            "{source:?} produced a literal type argument: {rendered}"
        );
    }
}

#[test]
fn test_parenthesized_tuples_and_calls_with_numbers_are_unchanged() {
    for source in ["(a, 3)", "(Foo, 3)", "(f a, 3)", "keep (f a, 3)", "keep (a, 3), 4"] {
        let rendered = parse_expression_debug(source);
        assert!(
            !rendered.contains("ConstValue"),
            "{source:?} produced a literal type argument: {rendered}"
        );
    }
}
