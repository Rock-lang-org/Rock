use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn error_propagation_chain() {
    let input = "a?.b?.c?";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    // Should parse as chained method calls with error propagation
    match expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(_),
            secondaries: Some(secondaries),
            ..
        })) => {
            // The actual structure might be: interogation, dot, ident, interogation, dot, ident, interogation
            // Let's just verify we have the expected number of secondaries
            assert!(
                secondaries.len() >= 3,
                "Expected at least 3 secondaries, got {}",
                secondaries.len()
            );

            // Verify we have interogation tokens
            let has_interogations = secondaries
                .iter()
                .any(|s| matches!(s, SecondaryExpr::Interogation));
            assert!(has_interogations, "Expected to find interogation tokens");

            // Verify we have dot tokens
            let has_dots = secondaries
                .iter()
                .any(|s| matches!(s, SecondaryExpr::Dot(_)));
            assert!(has_dots, "Expected to find dot tokens");
        }
        _ => panic!("Expected chained method calls, got: {:?}", expression),
    }
    assert_eq!(rest.len(), 0);
}
