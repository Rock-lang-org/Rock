use crate::ast::*;
use crate::parser::{expression, lex_test, ParseCtx, Parser};
use crate::Config;

fn parse(source: &str) -> Expression {
    let tokens = lex_test(source);
    let config = Config::default();
    let (rest, parsed) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();
    assert!(rest.is_empty(), "unparsed tokens in {source}: {rest:?}");
    parsed
}

fn primary(expression: &Expression) -> &PrimaryExpr {
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else {
        panic!("expected a primary expression: {expression:?}");
    };
    primary
}

fn secondaries(expression: &Expression) -> &[SecondaryExpr] {
    primary(expression)
        .secondaries
        .as_deref()
        .unwrap_or_default()
}

fn parse_in_body(source: &str) -> Expression {
    let program = crate::parser::parse_string(
        &format!("main = ->\n    {}\n", source.replace('\n', "\n    ")),
        &Config::default(),
    )
    .unwrap();
    let TopLevel::FunctionDecl(function) = &program.module.top_levels[0] else {
        panic!("expected function");
    };
    let Statement::Expression(expression) = &function.lambda.body.statements[0] else {
        panic!("expected expression");
    };
    expression.clone()
}

#[test]
fn multiline_chain_follows_completed_inline_call() {
    for source in [
        "make \"path\"\n    .unwrap_or -1\n    .println!",
        "make arg\n    .unwrap_or 0\n    .println!",
        "make first, second\n    .unwrap_or 0\n    .println!",
        "make nested arg\n    .unwrap_or 0\n    .println!",
        "make &mut arg\n    .unwrap_or 0\n    .println!",
        "make arg\n\n    // Continue the call.\n    .unwrap_or 0\n    .println!",
    ] {
        let parsed = parse_in_body(source);
        assert!(
            matches!(
                secondaries(&parsed),
                [
                    SecondaryExpr::Arguments(_),
                    SecondaryExpr::Dot(_),
                    SecondaryExpr::Arguments(_),
                    SecondaryExpr::Dot(_),
                    SecondaryExpr::Arguments(_)
                ]
            ),
            "{source}: {parsed:?}"
        );
    }
}

#[test]
fn multiline_chain_can_explicitly_belong_to_an_argument() {
    let parsed = parse("consume (make arg\n    .finish!)");
    let [SecondaryExpr::Arguments(arguments)] = secondaries(&parsed) else {
        panic!("expected a single outer call: {parsed:?}");
    };
    let Operand::Expression(inner) = &primary(&arguments[0].arg).operand else {
        panic!("expected an explicitly grouped argument");
    };
    assert!(matches!(
        secondaries(inner),
        [
            SecondaryExpr::Arguments(_),
            SecondaryExpr::Dot(_),
            SecondaryExpr::Arguments(_)
        ]
    ));
}

#[test]
fn multiline_chain_restores_statement_indentation() {
    let source = "main = !->\n    make 1\n        .unwrap_or 0\n        .println!\n    make 2\n        .unwrap_or 0\n        .println!\n";
    let program = crate::parser::parse_string(source, &Config::default()).unwrap();
    let TopLevel::FunctionDecl(function) = &program.module.top_levels[0] else {
        panic!("expected function");
    };
    assert_eq!(function.lambda.body.statements.len(), 2);
}

#[test]
fn call_result_propagation_precedes_any_following_infix_operator() {
    for operator in ["|>", "|>>", "+", "*", "==", "&&", "<+>"] {
        let source = format!("make &mut value? {operator} other");
        let parsed = parse(&source);
        let Expression::BinopExpr(UnaryExpr::PrimaryExpr(call), actual, _) = &parsed else {
            panic!("expected the operator outside the propagated call: {source}: {parsed:?}");
        };
        assert_eq!(actual.value, operator);
        assert!(matches!(
            call.secondaries.as_deref(),
            Some([SecondaryExpr::Arguments(_), SecondaryExpr::Interogation])
        ));
    }
}

#[test]
fn call_result_propagation_precedes_suffixes_and_reapplication() {
    for source in [
        "make value?.next 1?",
        "make value?[0]",
        "make value??",
        "make value? argument?",
    ] {
        let parsed = parse(source);
        assert!(
            matches!(
                secondaries(&parsed),
                [SecondaryExpr::Arguments(_), SecondaryExpr::Interogation, ..]
            ),
            "{source}: {parsed:?}"
        );
    }
    let parsed = parse("make value? as I32");
    let Expression::CastExpr(inner, _) = parsed else {
        panic!("expected a cast");
    };
    assert!(matches!(
        secondaries(&inner),
        [SecondaryExpr::Arguments(_), SecondaryExpr::Interogation]
    ));
}

#[test]
fn call_result_propagation_completes_expression_arguments_and_nested_calls() {
    for source in [
        "make a + b?",
        "make nested value?",
        "make first, nested second?",
    ] {
        let parsed = parse(source);
        assert!(
            matches!(
                secondaries(&parsed),
                [SecondaryExpr::Arguments(_), SecondaryExpr::Interogation]
            ),
            "{source}: {parsed:?}"
        );
    }
}

#[test]
fn argument_delimiters_keep_inner_propagation_local() {
    for source in [
        "consume (value?)",
        "consume dummy, [value?]",
        "consume dummy, [value?; 2]",
        "consume values[index?]",
        "consume (left?, right?)",
        "consume if flag then left? else right?",
        "consume Box value: item?",
        "consume (value -> make value?)",
    ] {
        let parsed = parse(source);
        assert!(
            matches!(secondaries(&parsed), [SecondaryExpr::Arguments(_)]),
            "{source}: {parsed:?}"
        );
    }
    let parsed = parse("consume (left?), right?");
    assert!(matches!(
        secondaries(&parsed),
        [SecondaryExpr::Arguments(_), SecondaryExpr::Interogation]
    ));
}

#[test]
fn ungrouped_propagation_ends_the_call_before_a_comma() {
    assert!(
        crate::parser::parse_string("main = -> consume left?, right\n", &Config::default(),)
            .is_err()
    );
}
