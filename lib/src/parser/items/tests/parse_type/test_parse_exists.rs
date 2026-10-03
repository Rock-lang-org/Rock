use crate::ast::{Expression, Operand, ParseType, UnaryExpr};
use crate::parser::items::{expression, parse_type};
use crate::parser::*;
use crate::Config;

fn parse(source: &str) -> ParseType {
    let tokens = lex_test(source);
    let config = Config::default();
    let (rest, ty) = parse_type
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();
    assert!(rest.is_empty());
    ty
}

#[test]
fn existential_parser_retains_one_binder_bounds_and_payload() {
    let ty = parse("exists Hidden where Hidden: Accept in (&Hidden, &Hidden)");
    let ParseType::Exists(package) = &ty else {
        panic!("package")
    };
    assert_eq!(package.parameter.name.name, "Hidden");
    assert_eq!(package.where_clauses.len(), 1);
    assert!(matches!(package.payload.as_ref(), ParseType::Tuple(elements) if elements.len() == 2));
    assert_eq!(parse(&ty.to_string()).to_string(), ty.to_string());
}

#[test]
fn existential_parser_preserves_constructor_kind_for_explicit_admission_rejection() {
    let ParseType::Exists(package) = parse("exists (F _) where F: Family in (&(F I64), &(F Bool))")
    else {
        panic!("package")
    };
    assert!(package.parameter.kind.is_some());
}

#[test]
fn pack_parser_retains_typed_tuple_payload() {
    let tokens = lex_test("pack (exists T where T: Accept in (&T, &T)) (&a, &b)");
    let config = Config::default();
    let (rest, parsed) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();
    assert!(rest.is_empty());
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = parsed else {
        panic!("primary")
    };
    let Operand::Pack(pack) = primary.operand else {
        panic!("typed package pack")
    };
    assert!(matches!(pack.target, ParseType::Exists(_)));
}
