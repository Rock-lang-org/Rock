use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_ident() {
    let input = "ident";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, ident) = ident.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(ident.name, "ident");
    assert_eq!(rest.len(), 0);
}
