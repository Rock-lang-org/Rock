use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_single_element_array() {
    let input = "[42]";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, array) = array.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(array.elements.len(), 1);
    assert_eq!(rest.len(), 0);
    // Vous pouvez ajouter des assertions supplémentaires pour vérifier la valeur de l'élément
}
