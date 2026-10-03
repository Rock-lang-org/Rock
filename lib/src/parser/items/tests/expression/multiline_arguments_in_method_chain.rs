use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn multiline_arguments_in_method_chain() {
    // Test multiline arguments: foo.bar(arg1, arg2).baz
    let input = r#"foo
    .bar
        arg1
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
                IdentOrType::Ident(Ident {
                    name: "foo".to_string(),
                    span: Span::test(),
                })
            );

            // Should have 3 secondaries: .bar, arguments(arg1, arg2), .baz
            assert_eq!(secondaries.len(), 3);

            match &secondaries[0] {
                SecondaryExpr::Dot(IdentOrNumber::Ident(ident)) => {
                    assert_eq!(ident.name, "bar");
                }
                _ => panic!("Expected .bar"),
            }

            match &secondaries[1] {
                SecondaryExpr::Arguments(args, _) => {
                    assert_eq!(args.len(), 2);
                }
                _ => panic!("Expected arguments"),
            }

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
