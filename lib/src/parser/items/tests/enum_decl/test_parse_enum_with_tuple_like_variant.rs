use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_enum_with_tuple_like_variant() {
    let input = "enum TupleLikeEnum\n    Variant Type1, Type2\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, enum_decl) = enum_decl.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(enum_decl.name.to_string(), "TupleLikeEnum");
    assert_eq!(enum_decl.variants.len(), 1);
    let variant = &enum_decl.variants[0];
    assert_eq!(variant.name.to_string(), "Variant");

    if let NamedFieldsOrTypesList::TypesList(types) = &variant.fields {
        assert_eq!(types.len(), 2);
        assert_eq!(types[0].to_string(), "Type1");
        assert_eq!(types[1].to_string(), "Type2");
    } else {
        panic!("Expected TypesList for tuple-like variant");
    }

    assert_eq!(rest.len(), 0);
}
