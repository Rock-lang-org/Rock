use crate::ast::*;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_enum_with_complex_types_in_variants() {
    let input = "enum ComplexEnum T\n    Variant1 Option Result T, E\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, enum_decl) = enum_decl.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(enum_decl.name.to_string(), "ComplexEnum");
    assert_eq!(enum_decl.generic_params[0].name.name, "T");
    assert_eq!(enum_decl.variants.len(), 1);
    let variant = &enum_decl.variants[0];

    if let NamedFieldsOrTypesList::TypesList(types) = &variant.fields {
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].to_string(), "Option Result, T, E");
    } else {
        panic!("Expected TypesList for complex variant");
    }

    assert_eq!(rest.len(), 0);
}
