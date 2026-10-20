use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::assert_formatted_eq;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn dot_expression_with_literal() {
    let input = "4.test";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_formatted_eq(
        &expression,
        &Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Literal(Literal {
                kind: crate::ast::LiteralKind::Number(4),
                span: Span::test(),
            }),
            secondaries: Some(vec![SecondaryExpr::Dot(IdentOrNumber::Ident(Ident { context: Default::default(),
                name: "test".to_string(),
                span: Span::test(),
            }))]),
            type_annotation: None,
        })),
    );

    assert_eq!(rest.len(), 0);
}
