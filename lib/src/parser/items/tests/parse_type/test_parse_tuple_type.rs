use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_tuple_type() {
    let input = "(A, B)";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, parse_type) = parse_type
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();
    assert_eq!(parse_type.to_string(), "(A, B)");
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parenthesized_type_groups_nested_generic_arguments() {
    let input = "Result (Result T, E), E";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, parse_type) = parse_type
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();
    let ParseType::Application(outer) = parse_type else {
        panic!("expected outer nominal type");
    };
    let ParseType::Application(inner) = &outer.args[0] else {
        panic!("expected grouped nested nominal type");
    };

    assert_eq!(outer.constructor.type_name(), "Result");
    assert_eq!(outer.args.len(), 2);
    assert_eq!(inner.constructor.type_name(), "Result");
    assert_eq!(inner.args.len(), 2);
    assert_eq!(rest.len(), 0);
}
