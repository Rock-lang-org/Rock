use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_ident_error() {
    let input = "123";
    let tokens = lex_test(input);
    let config = Config::default();

    let result = ident.process(ParseCtx::from(&tokens, &config));

    assert!(result.is_err());
}
