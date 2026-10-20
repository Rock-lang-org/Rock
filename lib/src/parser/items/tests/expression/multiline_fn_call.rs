use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::assert_formatted_eq;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;
use std::path::PathBuf;

#[test]
fn multiline_fn_call() {
    let input = r#"foo
        bar
        baz
        2 + 2"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_formatted_eq(
        &expression,
        &Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                    name: "foo".to_string(),
                    span: Span {
                        start: 0,
                        end: 3,
                        file_path: PathBuf::from("/test.rk"),
                    },
                })],
            }),
            secondaries: Some(vec![SecondaryExpr::Arguments(vec![
                Argument {
                    arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                        operand: Operand::Ident(IdentifierPath {
                            path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                                name: "bar".to_string(),
                                span: Span {
                                    start: 4,
                                    end: 7,
                                    file_path: PathBuf::from("/test.rk"),
                                },
                            })],
                        }),
                        secondaries: None,
                        type_annotation: None,
                    })),
                },
                Argument {
                    arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                        operand: Operand::Ident(IdentifierPath {
                            path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                                name: "baz".to_string(),
                                span: Span {
                                    start: 8,
                                    end: 11,
                                    file_path: PathBuf::from("/test.rk"),
                                },
                            })],
                        }),
                        secondaries: None,
                        type_annotation: None,
                    })),
                },
                Argument {
                    arg: Expression::BinopExpr(
                        UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Literal(Literal {
                                kind: crate::ast::LiteralKind::Number(2),
                                span: Span {
                                    start: 12,
                                    end: 13,
                                    file_path: PathBuf::from("/test.rk"),
                                },
                            }),
                            secondaries: None,
                            type_annotation: None,
                        }),
                        Operator { context: Default::default(),
                            value: "+".to_string(),
                            span: Span {
                                start: 14,
                                end: 15,
                                file_path: PathBuf::from("/test.rk"),
                            },
                        },
                        Box::new(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Literal(Literal {
                                kind: crate::ast::LiteralKind::Number(2),
                                span: Span {
                                    start: 16,
                                    end: 17,
                                    file_path: PathBuf::from("/test.rk"),
                                },
                            }),
                            secondaries: None,
                            type_annotation: None,
                        }))),
                    ),
                },
            ])]),
            type_annotation: None,
        })),
    );
    assert_eq!(rest.len(), 0);
}
