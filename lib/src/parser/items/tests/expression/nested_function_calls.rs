use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn nested_function_calls() {
    let input = "f g h x";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    // Should parse as nested function calls
    match expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(_),
            secondaries: Some(_),
            ..
        })) => {
            // Test passes if we get function calls
        }
        _ => panic!("Expected function calls, got: {:?}", expression),
    }
    assert_eq!(rest.len(), 0);
}
