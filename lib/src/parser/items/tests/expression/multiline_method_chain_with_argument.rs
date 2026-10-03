use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn multiline_method_chain_with_argument() {
    // This reproduces the expression-problem test case
    // Note: Changed to use parentheses to make the argument explicit
    let input = r#"a
.lol
.mdr(toto.tata)
.haha"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    // Should parse as: a.lol.mdr(toto.tata).haha
    match expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(ident_path),
            secondaries: Some(secondaries),
            ..
        })) => {
            // Should be identifier 'a'
            assert_eq!(
                ident_path.path[0],
                IdentOrType::Ident(Ident {
                    name: "a".to_string(),
                    span: Span::test(),
                })
            );

            // Should have 4 secondaries: .lol, .mdr, arguments(toto.tata), .haha
            if secondaries.len() != 4 {
                eprintln!("Got {} secondaries:", secondaries.len());
                for (i, sec) in secondaries.iter().enumerate() {
                    eprintln!("  {}: {:?}", i, sec);
                }
            }
            assert_eq!(
                secondaries.len(),
                4,
                "Expected exactly 4 secondaries: .lol, .mdr, arguments, .haha"
            );

            // Verify the structure: .lol, .mdr, arguments(toto.tata), .haha
            match &secondaries[0] {
                SecondaryExpr::Dot(IdentOrNumber::Ident(ident)) => {
                    assert_eq!(ident.name, "lol");
                }
                _ => panic!("Expected .lol as first secondary"),
            }

            match &secondaries[1] {
                SecondaryExpr::Dot(IdentOrNumber::Ident(ident)) => {
                    assert_eq!(ident.name, "mdr");
                }
                _ => panic!("Expected .mdr as second secondary"),
            }

            match &secondaries[2] {
                SecondaryExpr::Arguments(args, _) => {
                    assert_eq!(args.len(), 1, "Expected exactly one argument");
                    // Verify the argument is (toto.tata) - a parenthesized expression
                    match &args[0].arg {
                        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Expression(inner_expr),
                            secondaries: None,
                            ..
                        })) => {
                            // The inner expression should be toto.tata
                            match inner_expr.as_ref() {
                                Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                                    operand: Operand::Ident(ident_path),
                                    secondaries: Some(arg_secondaries),
                                    ..
                                })) => {
                                    assert_eq!(
                                        ident_path.path[0],
                                        IdentOrType::Ident(Ident {
                                            name: "toto".to_string(),
                                            span: Span::test(),
                                        })
                                    );
                                    assert_eq!(
                                        arg_secondaries.len(),
                                        1,
                                        "Expected exactly one secondary in argument (just .tata)"
                                    );
                                    match &arg_secondaries[0] {
                                        SecondaryExpr::Dot(IdentOrNumber::Ident(ident)) => {
                                            assert_eq!(ident.name, "tata");
                                        }
                                        _ => panic!("Expected .tata in argument"),
                                    }
                                }
                                _ => panic!("Expected toto.tata inside parentheses"),
                            }
                        }
                        _ => panic!("Expected (toto.tata) as argument"),
                    }
                }
                _ => panic!("Expected arguments as third secondary"),
            }

            match &secondaries[3] {
                SecondaryExpr::Dot(IdentOrNumber::Ident(ident)) => {
                    assert_eq!(ident.name, "haha");
                }
                _ => panic!("Expected .haha as fourth secondary"),
            }
        }
        _ => panic!("Expected method chain, got: {:?}", expression),
    }
    assert_eq!(rest.len(), 0);
}
