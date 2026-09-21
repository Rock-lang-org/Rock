use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::assert_formatted_eq;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;
use std::path::PathBuf;

fn default_span() -> Span {
    Span {
        start: 0,
        end: 0,
        file_path: PathBuf::from("/test.rk"),
    }
}

fn parse_type_i32() -> ParseType {
    ParseType::Type(ParseTypeInner {
        name: "I32".to_string(),
        generics: vec![],
        span: default_span(),
    })
}

fn parse_type_f64() -> ParseType {
    ParseType::Type(ParseTypeInner {
        name: "F64".to_string(),
        generics: vec![],
        span: default_span(),
    })
}

fn parse_type_char() -> ParseType {
    ParseType::Type(ParseTypeInner {
        name: "Char".to_string(),
        generics: vec![],
        span: default_span(),
    })
}

/// `42 as I32` parses to a CastExpr
#[test]
fn test_cast_simple() {
    let input = "42 as I32";
    let tokens = lex_test(input);
    let config = Config::default();
    let (rest, expr) = expression
        .process(ParseCtx::from(&tokens, &config))
        .expect("parse failed");
    assert_eq!(rest.len(), 0);
    assert!(
        matches!(&expr, Expression::CastExpr(_, _)),
        "expected CastExpr, got {:?}",
        expr
    );
    if let Expression::CastExpr(inner, ty) = expr {
        assert!(matches!(*inner, Expression::UnaryExpr(_)));
        assert_formatted_eq(&ty, &parse_type_i32());
    }
}

/// `x as F64` — identifier cast
#[test]
fn test_cast_ident() {
    let input = "x as F64";
    let tokens = lex_test(input);
    let config = Config::default();
    let (rest, expr) = expression
        .process(ParseCtx::from(&tokens, &config))
        .expect("parse failed");
    assert_eq!(rest.len(), 0);
    assert!(matches!(expr, Expression::CastExpr(_, _)));
    if let Expression::CastExpr(_, ty) = expr {
        assert_formatted_eq(&ty, &parse_type_f64());
    }
}

/// `a + b as I32` — `as` binds tighter than `+`, so this is `a + (b as I32)`.
/// The outer expression is a BinopExpr whose RHS is a CastExpr.
#[test]
fn test_cast_binop_rhs() {
    let input = "a + b as I32";
    let tokens = lex_test(input);
    let config = Config::default();
    let (rest, expr) = expression
        .process(ParseCtx::from(&tokens, &config))
        .expect("parse failed");
    assert_eq!(rest.len(), 0);
    // Outer: BinopExpr
    if let Expression::BinopExpr(_, op, rhs) = &expr {
        assert_eq!(op.value, "+");
        // RHS should be a CastExpr
        assert!(
            matches!(rhs.as_ref(), Expression::CastExpr(_, _)),
            "expected RHS to be CastExpr, got {:?}",
            rhs
        );
    } else {
        panic!("expected BinopExpr, got {:?}", expr);
    }
}

/// `byte as Char` — U8 to Char cast
#[test]
fn test_cast_to_char() {
    let input = "byte as Char";
    let tokens = lex_test(input);
    let config = Config::default();
    let (rest, expr) = expression
        .process(ParseCtx::from(&tokens, &config))
        .expect("parse failed");
    assert_eq!(rest.len(), 0);
    assert!(matches!(expr, Expression::CastExpr(_, _)));
    if let Expression::CastExpr(_, ty) = expr {
        assert_formatted_eq(&ty, &parse_type_char());
    }
}

/// `(a + b) as I32` — parenthesised expression cast
#[test]
fn test_cast_parenthesised_expr() {
    let input = "(a + b) as I32";
    let tokens = lex_test(input);
    let config = Config::default();
    let (rest, expr) = expression
        .process(ParseCtx::from(&tokens, &config))
        .expect("parse failed");
    assert_eq!(rest.len(), 0);
    // The whole thing should be a CastExpr
    assert!(
        matches!(&expr, Expression::CastExpr(_, _)),
        "expected CastExpr, got {:?}",
        expr
    );
    if let Expression::CastExpr(_, ty) = expr {
        assert_formatted_eq(&ty, &parse_type_i32());
    }
}

#[test]
fn test_cast_on_both_binary_operands() {
    let tokens = lex_test("total as F64 / count as F64");
    let config = Config::default();
    let (rest, expr) = expression
        .process(ParseCtx::from(&tokens, &config))
        .expect("parse failed");
    assert!(rest.is_empty());
    let Expression::BinopExpr(UnaryExpr::PrimaryExpr(lhs), op, rhs) = expr else {
        panic!("expected binary expression with a cast left operand");
    };
    assert_eq!(op.value, "/");
    let Operand::Expression(lhs) = lhs.operand else {
        panic!("expected a wrapped left cast");
    };
    assert!(matches!(*lhs, Expression::CastExpr(_, _)));
    assert!(matches!(*rhs, Expression::CastExpr(_, _)));
}
