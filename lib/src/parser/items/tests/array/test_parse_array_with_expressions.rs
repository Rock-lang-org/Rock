use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_array_with_expressions() {
    let input = "[a + b, foo!]";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, array) = array.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(array.elements.len(), 2);
    assert_eq!(rest.len(), 0);
    // Vérifiez les expressions des éléments si nécessaire
}
