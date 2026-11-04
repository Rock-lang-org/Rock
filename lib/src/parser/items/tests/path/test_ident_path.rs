use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_ident_path() {
    let input = "ident::Type::ident";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, ident) = ident_path
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(ident.path.len(), 3);
    assert_eq!(rest.len(), 0);
}
