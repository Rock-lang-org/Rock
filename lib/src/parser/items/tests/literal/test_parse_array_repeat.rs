use crate::ast::{Expression, LiteralKind, Operand, ParseArrayLen, UnaryExpr};
use crate::parser::items::tests::common::parse_literal;

#[test]
fn test_parse_array_repeat() {
    let literal = parse_literal("[1 + 2;4]");

    let LiteralKind::ArrayRepeat { value, len } = literal.kind else {
        panic!("expected repeat array literal");
    };

    assert_eq!(len, ParseArrayLen::Literal(4));
    assert!(matches!(*value, Expression::BinopExpr(_, _, _)));
}

#[test]
fn test_parse_nested_array_repeat() {
    let literal = parse_literal("[[0; 2]; 3]");

    let LiteralKind::ArrayRepeat { value, len } = literal.kind else {
        panic!("expected outer repeat array literal");
    };
    assert_eq!(len, ParseArrayLen::Literal(3));
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = *value else {
        panic!("expected inner repeat array expression");
    };
    assert!(matches!(
        primary.operand,
        Operand::Literal(crate::ast::Literal {
            kind: LiteralKind::ArrayRepeat {
                len: ParseArrayLen::Literal(2),
                ..
            },
            ..
        })
    ));
}

#[test]
fn test_parse_array_repeat_with_const_param_length() {
    let literal = parse_literal("[0; N]");

    let LiteralKind::ArrayRepeat { value, len } = literal.kind else {
        panic!("expected repeat array literal");
    };

    let ParseArrayLen::Param(ident) = len else {
        panic!("expected a named length");
    };
    assert_eq!(ident.name, "N");
    assert!(matches!(*value, Expression::UnaryExpr(_)));
}

#[test]
fn test_parse_array_repeat_with_expression_and_const_param_length() {
    let literal = parse_literal("[1 + 2; N]");

    let LiteralKind::ArrayRepeat { value, len } = literal.kind else {
        panic!("expected repeat array literal");
    };

    assert!(matches!(len, ParseArrayLen::Param(_)));
    assert!(matches!(*value, Expression::BinopExpr(_, _, _)));
}
