use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn self_ident() {
    let input = "@foo";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(
        expression,
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::SelfIdent(Ident { context: Default::default(),
                name: "foo".to_string(),
                span: Span::test(),
            }),
            secondaries: None,
            type_annotation: None,
        })),
    );
    assert_eq!(rest.len(), 0);
}
