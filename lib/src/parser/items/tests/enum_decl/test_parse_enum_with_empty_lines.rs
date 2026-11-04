use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_enum_with_empty_lines() {
    let input = "enum SpacedEnum\n\n    Variant1\n\n    Variant2\n\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, enum_decl) = enum_decl.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(enum_decl.name.to_string(), "SpacedEnum");
    assert_eq!(enum_decl.variants.len(), 2);
    assert_eq!(rest.len(), 0);
}
