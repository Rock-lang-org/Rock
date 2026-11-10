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
            assert_eq!(len, ParseArrayLen::Literal(4));
        }
        other => panic!("expected fixed array type, got {other:?}"),
    }
}

#[test]
fn test_parse_const_generic_array_type() {
    let input = "[T; N]";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, ty) = parse_type(ParseCtx::from(&tokens, &config)).unwrap();

    match ty {
        ParseType::Array { inner, len } => {
            assert_eq!(inner.type_name(), "T");
            match len {
                ParseArrayLen::Param(ident) => assert_eq!(ident.name, "N"),
                other => panic!("expected named length, got {other:?}"),
            }
        }
        other => panic!("expected fixed array type, got {other:?}"),
    }
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parse_array_length_rejects_lowercase_name() {
    let input = "[T; n]";
    let tokens = lex_test(input);
    let config = Config::default();

    assert!(parse_type(ParseCtx::from(&tokens, &config)).is_err());
}

#[test]
fn test_parse_literal_type_argument() {
    let tokens = lex_test("Buf I64, 3");
    let config = Config::default();

    let (rest, ty) = parse_type(ParseCtx::from(&tokens, &config)).unwrap();

    match ty {
        ParseType::Application(application) => {
            assert_eq!(application.args.len(), 2);
            assert!(matches!(
                application.args[1],
                ParseType::ConstValue { value: 3, .. }
            ));
        }
        other => panic!("expected type application, got {other:?}"),
    }
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parse_literal_type_argument_as_only_argument() {
    let tokens = lex_test("Wrapper 8");
    let config = Config::default();

    let (rest, ty) = parse_type(ParseCtx::from(&tokens, &config)).unwrap();

    match ty {
        ParseType::Application(application) => {
            assert_eq!(application.args.len(), 1);
            assert!(matches!(
                application.args[0],
                ParseType::ConstValue { value: 8, .. }
            ));
        }
        other => panic!("expected type application, got {other:?}"),
    }
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parse_leading_number_is_const_value_atom() {
    let tokens = lex_test("3");
    let config = Config::default();

    let (_, ty) = parse_type(ParseCtx::from(&tokens, &config)).unwrap();

    // A leading number is accepted as an atom, but only argument positions give it meaning.
    assert!(matches!(ty, ParseType::ConstValue { value: 3, .. }));
}
