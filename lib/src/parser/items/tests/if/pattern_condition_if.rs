use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn pattern_condition_if() {
    let input = "if a = 1 then 1";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, _if_) = parse_if.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(rest.len(), 0);
}
