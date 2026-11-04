use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_enum_instance() {
    let input = "Type::Variant1";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, enum_instance) = instance.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(enum_instance.name.to_string(), "Type::Variant1");
    assert_eq!(rest.len(), 0);
}
