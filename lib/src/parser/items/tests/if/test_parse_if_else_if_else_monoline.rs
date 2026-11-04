use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_if_else_if_else_monoline() {
    let input = "if true then 1 else if false then 2 else 3";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, _if_) = parse_if.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(rest.len(), 0);
}
