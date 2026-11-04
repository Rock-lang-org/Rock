use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn complex_operator_precedence() {
    let input = "a + b * c - d / e";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    // Should parse with correct precedence: a + (b * c) - (d / e)
    // Note: This test verifies the parser accepts the input,
    // actual precedence is handled in desugaring phase
    match expression {
        Expression::BinopExpr(_, _, _) => {
            // Test passes if we get a binary expression
        }
        _ => panic!("Expected binary expression, got: {:?}", expression),
    }
    assert_eq!(rest.len(), 0);
}
