use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn dot_expression() {
    let input = "hello.world";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(
        expression,
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                    name: "hello".to_string(),
                    span: Span::test(),
                })],
            }),
            secondaries: Some(vec![SecondaryExpr::Dot(IdentOrNumber::Ident(Ident { context: Default::default(),
                name: "world".to_string(),
                span: Span::test(),
            }))]),
            type_annotation: None,
        })),
    );

    assert_eq!(rest.len(), 0);
}
