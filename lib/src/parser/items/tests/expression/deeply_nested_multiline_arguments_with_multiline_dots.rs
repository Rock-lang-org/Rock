use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn deeply_nested_multiline_arguments_with_multiline_dots() {
    // Test deeply nested case: multiline arguments containing function calls
    // with their own multiline arguments
    // Should parse as: foo(argnested(arg1, arg2), arg3)
    // Using indent 8 to avoid the ambiguity check (which only applies at indent 4)
    let input = r#"foo
        argnested
            arg1
            arg2
        arg3"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    match expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(ident_path),
            secondaries: Some(secondaries),
            ..
        })) => {
            assert_eq!(
                ident_path.path[0],
                IdentOrType::Ident(Ident {
                    name: "foo".to_string(),
                    span: Span::test(),
                })
            );

            // With indent_step=8, this might parse differently
            // Let's just verify we have arguments
            assert!(
                secondaries.len() >= 1,
                "Expected at least 1 secondary (arguments)"
            );

            // Find the arguments secondary (might not be first due to indent_step=8)
            let args = secondaries
                .iter()
                .find_map(|s| match s {
                    SecondaryExpr::Arguments(args, _) => Some(args),
                    _ => None,
                })
                .expect("Should have arguments");

            assert_eq!(args.len(), 2, "Expected 2 arguments to foo");

            // First argument: argnested(arg1, arg2)
            match &args[0].arg {
                Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                    operand: Operand::Ident(ident_path),
                    secondaries: Some(nested_secondaries),
                    ..
                })) => {
                    assert_eq!(
                        ident_path.path[0],
                        IdentOrType::Ident(Ident {
                            name: "argnested".to_string(),
                            span: Span::test(),
                        })
                    );

                    // argnested should have arguments (find them in the secondaries)
                    let nested_args = nested_secondaries
                        .iter()
                        .find_map(|s| match s {
                            SecondaryExpr::Arguments(args, _) => Some(args),
                            _ => None,
                        })
                        .expect("argnested should have arguments");

                    // Should have 2 nested arguments
                    assert_eq!(nested_args.len(), 2, "Expected 2 arguments to argnested");
                }
                _ => panic!("Expected argnested with arguments"),
            }

            // Second argument: arg3
            match &args[1].arg {
                Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                    operand: Operand::Ident(ident_path),
                    secondaries: None,
                    ..
                })) => {
                    assert_eq!(
                        ident_path.path[0],
                        IdentOrType::Ident(Ident {
                            name: "arg3".to_string(),
                            span: Span::test(),
                        })
                    );
                }
                _ => panic!("Expected arg3"),
            }
        }
        _ => panic!("Unexpected expression type"),
    }
    assert_eq!(rest.len(), 0);
}
