use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::*;

#[test]
fn test_parse_array_nested_expr() {
    let input = "[1, [hello, 3], 8, 5 + 4]";
    let literal = parse_literal(input);

    let expected = LiteralKind::Array(Array {
        elements: vec![
            Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Literal(Literal {
                    kind: LiteralKind::Number(1),
                    span: Span::test(),
                }),
                secondaries: None,
                type_annotation: None,
            })),
            Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Literal(Literal {
                    span: Span::test(),
                    kind: LiteralKind::Array(Array {
                        elements: vec![
                            Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                                operand: Operand::Ident(IdentifierPath {
                                    path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                                        name: "hello".to_string(),
                                        span: Span::test(),
                                    })],
                                }),
                                secondaries: None,
                                type_annotation: None,
                            })),
                            Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                                operand: Operand::Literal(Literal {
                                    kind: LiteralKind::Number(3),
                                    span: Span::test(),
                                }),
                                secondaries: None,
                                type_annotation: None,
                            })),
                        ],
                    }),
                }),
                secondaries: None,
                type_annotation: None,
            })),
            Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Literal(Literal {
                    kind: LiteralKind::Number(8),
                    span: Span::test(),
                }),
                secondaries: None,
                type_annotation: None,
            })),
            Expression::BinopExpr(
                UnaryExpr::PrimaryExpr(PrimaryExpr {
                    operand: Operand::Literal(Literal {
                        kind: LiteralKind::Number(5),
                        span: Span::test(),
                    }),
                    secondaries: None,
                    type_annotation: None,
                }),
                Operator { context: Default::default(),
                    value: "+".to_string(),
                    span: Span::test(),
                },
                Box::new(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                    operand: Operand::Literal(Literal {
                        kind: LiteralKind::Number(4),
                        span: Span::test(),
                    }),
                    secondaries: None,
                    type_annotation: None,
                }))),
            ),
        ],
    });

    assert_formatted_literal_kind_eq(&literal.kind, &expected);
}
