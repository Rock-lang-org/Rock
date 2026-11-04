use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_type_with_multiple_generics() {
    let input = "Type Generics, Generics2";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, parse_type) = parse_type
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(parse_type.to_string(), "Type Generics, Generics2");
    assert_eq!(rest.len(), 0);
}
