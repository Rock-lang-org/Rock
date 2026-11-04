use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_enum_with_single_variant() {
    let input = "enum SingleVariant\n    OnlyVariant\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, enum_decl) = enum_decl.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(enum_decl.name.to_string(), "SingleVariant");
    assert_eq!(enum_decl.variants.len(), 1);
    assert_eq!(enum_decl.variants[0].name.to_string(), "OnlyVariant");
    assert_eq!(rest.len(), 0);
}
