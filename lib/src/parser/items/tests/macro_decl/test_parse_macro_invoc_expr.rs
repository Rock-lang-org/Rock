use crate::ast::{Expression, Operand, SecondaryExpr, Statement, UnaryExpr};
use crate::lexer::TokenType;
use crate::parser::items::tests::common::*;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

fn arg_types(invocation: &MacroInvoc) -> Vec<TokenType> {
    invocation
        .args
        .iter()
        .map(|token| token.token_type.clone())
        .collect()
}

fn operand_of(expr: &Expression) -> &Operand {
    match expr {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) => &primary.operand,
        other => panic!("expected a primary expression, got {other:?}"),
    }
}

#[test]
fn macro_invoc_expr_takes_comma_separated_args_to_end_of_line() {
    let tokens = lex("%format \"a\", b\nnext");
    let tokens = &tokens[1..]; // skip the Indent(0)
    let config = Config::default();

    let (rest, invocation) = macro_invoc_expr(ParseCtx::from(tokens, &config)).unwrap();

    assert_eq!(invocation.name.name, "format");
    assert_eq!(
        arg_types(&invocation),
        vec![
            TokenType::String("a".to_string()),
            TokenType::Coma,
            TokenType::Ident("b".to_string()),
        ]
    );
    assert_eq!(rest.tokens[0].token_type, TokenType::Eol);
}

#[test]
fn macro_invoc_expr_stops_at_unmatched_closing_paren() {
    let tokens = lex("%format (a, b), c) d");
    let tokens = &tokens[1..]; // skip the Indent(0)
    let config = Config::default();

    let (rest, invocation) = macro_invoc_expr(ParseCtx::from(tokens, &config)).unwrap();

    assert_eq!(invocation.args.len(), 7);
    assert_eq!(rest.tokens[0].token_type, TokenType::CloseParen);
}

#[test]
fn parenthesized_macro_invoc_accepts_method_calls() {
    let tokens = lex("(%format a, b).println!");
    let config = Config::default();

    let (_, expr) = expression(ParseCtx::from(&tokens[1..], &config)).unwrap();

    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = &expr else {
        panic!("expected a primary expression, got {expr:?}");
    };
    let Operand::Expression(inner) = &primary.operand else {
        panic!("expected a parenthesized operand, got {:?}", primary.operand);
    };
    assert!(matches!(operand_of(inner), Operand::MacroInvoc(_)));
    assert!(matches!(
        primary.secondaries.as_deref(),
        Some([SecondaryExpr::Dot(_), SecondaryExpr::Arguments(_)])
    ));
}

#[test]
fn macro_invoc_expr_can_be_assigned() {
    let program = parse_string("main = ->\n    s = %format \"a\", b\n    s\n", &Config::default())
        .expect("assignment from a macro call should parse");

    let crate::ast::TopLevel::FunctionDecl(main) = &program.module.top_levels[0] else {
        panic!("expected main");
    };
    let Statement::Assignment(assignment) = &main.lambda.body.statements[0] else {
        panic!("expected an assignment");
    };
    let Operand::MacroInvoc(invocation) = operand_of(&assignment.rhs) else {
        panic!("expected a macro call");
    };
    assert_eq!(invocation.name.name, "format");
    assert_eq!(invocation.args.len(), 3);
    assert_eq!(main.lambda.body.statements.len(), 2);
}
