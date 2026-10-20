use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::assert_formatted_eq;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn multiline_operator() {
    // Test multiline operators with indentation on the RHS
    // This tests the "expression on next line with indentation" continuation
    let input = r#"foo +
    2"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_formatted_eq(
        &expression,
        &Expression::BinopExpr(
            UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Ident(IdentifierPath {
                    path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                        name: "foo".to_string(),
                        span: Span::test(),
                    })],
                }),
                secondaries: None,
                type_annotation: None,
            }),
            Operator { context: Default::default(),
                value: "+".to_string(),
                span: Span::test(),
            },
            Box::new(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Literal(Literal {
                    kind: crate::ast::LiteralKind::Number(2),
                    span: Span::test(),
                }),
                secondaries: None,
                type_annotation: None,
            }))),
        ),
    );
    assert_eq!(rest.len(), 0);
}

#[test]
fn multiline_operator_indented() {
    // Test operators at the beginning of an indented line
    let input = r#"foo
    + 2"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_formatted_eq(
        &expression,
        &Expression::BinopExpr(
            UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Ident(IdentifierPath {
                    path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                        name: "foo".to_string(),
                        span: Span::test(),
                    })],
                }),
                secondaries: None,
                type_annotation: None,
            }),
            Operator { context: Default::default(),
                value: "+".to_string(),
                span: Span::test(),
            },
            Box::new(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Literal(Literal {
                    kind: crate::ast::LiteralKind::Number(2),
                    span: Span::test(),
                }),
                secondaries: None,
                type_annotation: None,
            }))),
        ),
    );
    assert_eq!(rest.len(), 0);
}

#[test]
fn multiline_operator_chained() {
    // Test multiple operator continuations at the same indent level
    // All continuation lines should be at indent_level + indent_step
    let input = r#"2 + 3 + 4
    + 5 + 6
    + 7"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expr) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    // The result should be a chain of BinopExprs
    // We just verify it parses successfully and consumes all tokens
    assert_eq!(rest.len(), 0);

    // Verify the outermost is a BinopExpr with "+" operator
    match &expr {
        Expression::BinopExpr(_, op, _) => {
            assert_eq!(op.value, "+");
        }
        _ => panic!("Expected BinopExpr, got {:?}", expr),
    }
}

#[test]
fn multiline_operator_not_allowed_without_indent() {
    // Operators on the next line WITHOUT indentation should NOT be consumed
    // This prevents confusion with export syntax like "< foo"
    let input = r#"foo
+ 2"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expr) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    // Should only parse "foo", not consuming "+ 2"
    match &expr {
        Expression::UnaryExpr(_) => {}
        _ => panic!("Expected UnaryExpr (just 'foo'), got {:?}", expr),
    }
    // rest should contain Eol, Indent(0), +, 2
    assert!(rest.len() > 0);
}
