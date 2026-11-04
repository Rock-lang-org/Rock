use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn unsafe_block() {
    let input = "unsafe\n    ptr = 0\n    *ptr";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    // Should parse as an unsafe block operand
    match expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Unsafe(_, span),
            ..
        })) => {
            assert_eq!(span, tokens[0].span);
            // Test passes if we get an unsafe operand
        }
        _ => panic!("Expected unsafe block, got: {:?}", expression),
    }
    assert_eq!(rest.len(), 0);
}
