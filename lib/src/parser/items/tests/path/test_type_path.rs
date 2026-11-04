use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_type_path() {
    let input = "ident::Type::Type";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, ident) = type_path.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(ident.path.len(), 3);
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_type_path_keeps_enum_variant_segments() {
    let input = "Option::Some";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, path) = type_path.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(path.path.len(), 2);
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_type_path_accepts_self_target_projection() {
    let input = "Self::Target";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, path) = type_path.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(path.path.len(), 2);
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_type_path_accepts_self_output_projection() {
    let input = "Self::Output";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, path) = type_path.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(path.path.len(), 2);
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_type_path_rejects_reference_segment() {
    let input = "&Foo::Bar";
    let tokens = lex_test(input);
    let config = Config::default();

    assert!(type_path.process(ParseCtx::from(&tokens, &config)).is_err());
}

#[test]
fn test_type_path_rejects_compound_unit_path() {
    let input = "()::Foo";
    let tokens = lex_test(input);
    let config = Config::default();

    assert!(type_path.process(ParseCtx::from(&tokens, &config)).is_err());
}
