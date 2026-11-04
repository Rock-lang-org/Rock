use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_reference_type() {
    let input = "&A";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, parse_type) = parse_type
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(parse_type.to_string(), "&A");
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parse_reference_type_caret() {
    let input = "&^A";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, parse_type) = parse_type
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(parse_type.to_string(), "&mut A");
    assert_eq!(rest.len(), 0);
}
