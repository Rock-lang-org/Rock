use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn dot_expression_with_number() {
    let input = "some_tuple.1";
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
                    name: "some_tuple".to_string(),
                    span: Span::test(),
                })],
            }),
            secondaries: Some(vec![SecondaryExpr::Dot(IdentOrNumber::Number(1))]),
            type_annotation: None,
        })),
    );
    assert_eq!(rest.len(), 0);
}
