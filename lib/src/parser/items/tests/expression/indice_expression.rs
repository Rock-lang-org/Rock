use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::assert_formatted_eq;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;
use std::path::PathBuf;

#[test]
fn indice_expression() {
    let input = "hello[1]";
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
                    name: "hello".to_string(),
                    span: Span {
                        start: 0,
                        end: 5,
                        file_path: PathBuf::from("/test.rk"),
                    },
                })],
            }),
            secondaries: Some(vec![SecondaryExpr::Indice(Box::new(
                Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                    operand: Operand::Literal(Literal {
                        kind: crate::ast::LiteralKind::Number(1),
                        span: Span {
                            start: 6,
                            end: 7,
                            file_path: PathBuf::from("/test.rk"),
                        },
                    }),
                    secondaries: None,
                    type_annotation: None,
                })),
            ))]),
            type_annotation: None,
        })),
    );

    assert_eq!(rest.len(), 0);
}
