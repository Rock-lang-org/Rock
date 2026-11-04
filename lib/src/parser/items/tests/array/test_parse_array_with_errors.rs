use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_array_with_errors() {
    let input = "[1, 2,";
    let tokens = lex_test(input);
    let config = Config::default();

    let result = array.process(ParseCtx::from(&tokens, &config));

    assert!(result.is_err());
}
