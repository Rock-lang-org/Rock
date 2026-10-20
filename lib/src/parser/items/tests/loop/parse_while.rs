use crate::ast::*;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn parse_while() {
    let input = "while x\n    2";
    let tokens = lex_test(input);
    let config = Config::default();

    let (remaining, loop_) = r#loop.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(
        loop_,
        Loop::While(
            Condition {
                pattern: None,
                expression: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                    operand: Operand::Ident(IdentifierPath {
                        path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                            name: "x".to_string(),
                            span: tokens[1].span.clone()
                        })]
                    }),
                    secondaries: None,
                    type_annotation: None,
                }))
            },
            Block {
                statements: vec![Statement::Expression(Expression::UnaryExpr(
                    UnaryExpr::PrimaryExpr(PrimaryExpr {
                        operand: Operand::Literal(Literal {
                            kind: LiteralKind::Number(2),
                            span: tokens[4].span.clone()
                        }),
                        secondaries: None,
                        type_annotation: None,
                    })
                ))],
            },
            tokens[0].span.clone(),
        )
    );
    assert_eq!(remaining.len(), 0);
}
