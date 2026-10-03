use crate::ast::*;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn application_argument_consumes_infix_expression() {
    let input = "foo 2 + 2";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        secondaries: Some(secondaries),
        ..
    })) = expression
    else {
        panic!("expected application");
    };
    let SecondaryExpr::Arguments(arguments, _) = &secondaries[0] else {
        panic!("expected arguments");
    };

    assert!(matches!(arguments[0].arg, Expression::BinopExpr(_, _, _)));

    assert_eq!(rest.len(), 0);
}

#[test]
fn parenthesized_application_can_be_used_as_infix_operand() {
    let input = "(Option::Some 2) <&> inc";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert!(matches!(expression, Expression::BinopExpr(_, _, _)));
    assert_eq!(rest.len(), 0);
}
