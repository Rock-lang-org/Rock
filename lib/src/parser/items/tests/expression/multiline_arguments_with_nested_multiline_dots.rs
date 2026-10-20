use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn multiline_arguments_with_nested_multiline_dots() {
    // Test complex case: multiline arguments where arguments themselves have multiline dots
    // Should parse as: foo.bar(arg1.method1.method2, arg2).baz
    let input = r#"foo
    .bar
        arg1
            .method1
            .method2
        arg2
    .baz"#;
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
                IdentOrType::Ident(Ident { context: Default::default(),
                    name: "foo".to_string(),
                    span: Span::test(),
                })
            );

            // Should have 3 secondaries: .bar, arguments, .baz
            assert_eq!(secondaries.len(), 3);

            // Check .bar
            match &secondaries[0] {
                SecondaryExpr::Dot(IdentOrNumber::Ident(ident)) => {
                    assert_eq!(ident.name, "bar");
                }
                _ => panic!("Expected .bar"),
            }

            // Check arguments
            match &secondaries[1] {
                SecondaryExpr::Arguments(args) => {
                    assert_eq!(args.len(), 2, "Expected 2 arguments");

                    // First argument: arg1.method1.method2
                    match &args[0].arg {
                        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Ident(ident_path),
                            secondaries: Some(arg_secondaries),
                            ..
                        })) => {
                            assert_eq!(
                                ident_path.path[0],
                                IdentOrType::Ident(Ident { context: Default::default(),
                                    name: "arg1".to_string(),
                                    span: Span::test(),
                                })
                            );
                            // Should have .method1 and .method2
                            assert_eq!(arg_secondaries.len(), 2);
                            match &arg_secondaries[0] {
                                SecondaryExpr::Dot(IdentOrNumber::Ident(ident)) => {
                                    assert_eq!(ident.name, "method1");
                                }
                                _ => panic!("Expected .method1"),
                            }
                            match &arg_secondaries[1] {
                                SecondaryExpr::Dot(IdentOrNumber::Ident(ident)) => {
                                    assert_eq!(ident.name, "method2");
                                }
                                _ => panic!("Expected .method2"),
                            }
                        }
                        _ => panic!("Expected arg1.method1.method2"),
                    }

                    // Second argument: arg2
                    match &args[1].arg {
                        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Ident(ident_path),
                            secondaries: None,
                            ..
                        })) => {
                            assert_eq!(
                                ident_path.path[0],
                                IdentOrType::Ident(Ident { context: Default::default(),
                                    name: "arg2".to_string(),
                                    span: Span::test(),
                                })
                            );
                        }
                        _ => panic!("Expected arg2"),
                    }
                }
                _ => panic!("Expected arguments"),
            }

            // Check .baz
            match &secondaries[2] {
                SecondaryExpr::Dot(IdentOrNumber::Ident(ident)) => {
                    assert_eq!(ident.name, "baz");
                }
                _ => panic!("Expected .baz"),
            }
        }
        _ => panic!("Unexpected expression type"),
    }
    assert_eq!(rest.len(), 0);
}
