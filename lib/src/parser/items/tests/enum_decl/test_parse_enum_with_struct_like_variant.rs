use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_enum_with_struct_like_variant() {
    let input = "enum StructLikeEnum\n    Variant\n        field1: Type1\n        field2: Type2\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, enum_decl) = enum_decl.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(enum_decl.name.to_string(), "StructLikeEnum");
    assert_eq!(enum_decl.variants.len(), 1);
    let variant = &enum_decl.variants[0];
    assert_eq!(variant.name.to_string(), "Variant");

    if let NamedFieldsOrTypesList::NamedFields(fields) = &variant.fields {
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name.to_string(), "field1");
        assert_eq!(fields[1].name.to_string(), "field2");
    } else {
        panic!("Expected NamedFields for struct-like variant");
    }

    assert_eq!(rest.len(), 0);
}
