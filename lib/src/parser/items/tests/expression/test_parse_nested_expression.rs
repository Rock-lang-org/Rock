use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::assert_formatted_eq;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;
use std::path::PathBuf;

#[test]
fn test_parse_nested_expression() {
    let input = "a.a + b + 2";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_formatted_eq(
        &expression,
        &Expression::BinopExpr(
            UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Ident(IdentifierPath {
                    path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                        name: "a".to_string(),
                        span: Span {
                            start: 0,
                            end: 1,
                            file_path: PathBuf::from("/test.rk"),
                        },
                    })],
                }),
                secondaries: Some(vec![SecondaryExpr::Dot(IdentOrNumber::Ident(Ident { context: Default::default(),
                    name: "a".to_string(),
                    span: Span {
                        start: 2,
                        end: 3,
                        file_path: PathBuf::from("/test.rk"),
                    },
                }))]),
                type_annotation: None,
            }),
            Operator { context: Default::default(),
                value: "+".to_string(),
                span: Span::test(),
            },
            Box::new(Expression::BinopExpr(
                UnaryExpr::PrimaryExpr(PrimaryExpr {
                    operand: Operand::Ident(IdentifierPath {
                        path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                            name: "b".to_string(),
                            span: Span {
                                start: 6,
                                end: 7,
                                file_path: PathBuf::from("/test.rk"),
                            },
                        })],
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
                        kind: crate::ast::LiteralKind::Number(2),
                        span: Span {
                            start: 10,
                            end: 11,
                            file_path: PathBuf::from("/test.rk"),
                        },
                    }),
                    secondaries: None,
                    type_annotation: None,
                }))),
            )),
        ),
    );
    assert_eq!(rest.len(), 0);
}
