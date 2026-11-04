use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_array_type() {
    let input = "[A]";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, parse_type) = parse_type
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(parse_type.to_string(), "[A]");
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parse_slice_type() {
    let input = "[I64]";
    let tokens = lex_test(input);
    let config = Config::default();

    let (_, ty) = parse_type(ParseCtx::from(&tokens, &config)).unwrap();

    match ty {
        ParseType::Slice(inner) => {
            assert_eq!(inner.type_name(), "I64");
        }
        other => panic!("expected slice type, got {other:?}"),
    }
}

#[test]
fn test_parse_fixed_array_type() {
    let input = "[I64; 4]";
    let tokens = lex_test(input);
    let config = Config::default();

    let (_, ty) = parse_type(ParseCtx::from(&tokens, &config)).unwrap();

    match ty {
        ParseType::Array { inner, len } => {
            assert_eq!(inner.type_name(), "I64");
            assert_eq!(len, 4);
        }
        other => panic!("expected fixed array type, got {other:?}"),
    }
}
