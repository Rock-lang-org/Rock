use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_enum_with_no_variants() {
    let input = "enum EmptyEnum\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, enum_decl) = enum_decl.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(enum_decl.name.to_string(), "EmptyEnum");
    assert_eq!(enum_decl.variants.len(), 0);
    assert_eq!(rest.len(), 0);
}
