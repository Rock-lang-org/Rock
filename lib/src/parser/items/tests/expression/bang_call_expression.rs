use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;
use std::path::PathBuf;

#[test]
fn bang_call_expression() {
    let input = "hello!";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(
        expression,
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(Ident {
                    name: "hello".to_string(),
                    span: Span {
                        start: 0,
                        end: 5,
                        file_path: PathBuf::default(),
                    },
                })],
            }),
            secondaries: Some(vec![SecondaryExpr::Arguments(
                vec![],
                Span::new("/test.rk".into(), 5, input.len())
            )]),
            type_annotation: None,
        })),
    );

    assert_eq!(rest.len(), 0);
}
