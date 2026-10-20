use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::assert_formatted_eq;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn nested_parenthesis_expression() {
    let input = "(1 + (2 + 3))";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_formatted_eq(
        &expression,
        &Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Expression(Box::new(Expression::BinopExpr(
                UnaryExpr::PrimaryExpr(PrimaryExpr {
                    operand: Operand::Literal(Literal {
                        kind: crate::ast::LiteralKind::Number(1),
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
                    operand: Operand::Expression(Box::new(Expression::BinopExpr(
                        UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Literal(Literal {
                                kind: crate::ast::LiteralKind::Number(2),
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
                                kind: crate::ast::LiteralKind::Number(3),
                                span: Span::test(),
                            }),
                            secondaries: None,
                            type_annotation: None,
                        }))),
                    ))),
                    secondaries: None,
                    type_annotation: None,
                }))),
            ))),
            secondaries: None,
            type_annotation: None,
        })),
    );

    assert_eq!(rest.len(), 0);
}
