use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_enum_with_multiple_struct_like_variants() {
    let input = "enum MultiStructEnum\n    Variant1\n        field1: Type1\n    Variant2\n        field2: Type2\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, enum_decl) = enum_decl.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(enum_decl.name.to_string(), "MultiStructEnum");
    assert_eq!(enum_decl.variants.len(), 2);

    // Vérification du premier variant
    let variant1 = &enum_decl.variants[0];
    assert_eq!(variant1.name.to_string(), "Variant1");
    if let NamedFieldsOrTypesList::NamedFields(fields) = &variant1.fields {
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name.to_string(), "field1");
    } else {
        panic!("Expected NamedFields for Variant1");
    }

    // Vérification du deuxième variant
    let variant2 = &enum_decl.variants[1];
    assert_eq!(variant2.name.to_string(), "Variant2");
    if let NamedFieldsOrTypesList::NamedFields(fields) = &variant2.fields {
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name.to_string(), "field2");
    } else {
        panic!("Expected NamedFields for Variant2");
    }

    assert_eq!(rest.len(), 0);
}
