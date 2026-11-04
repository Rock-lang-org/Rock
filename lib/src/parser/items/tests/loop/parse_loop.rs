use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn parse_loop() {
    let input = "loop\n    2";
    let tokens = lex_test(input);
    let config = Config::default();

    let (remaining, loop_) = r#loop.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(
        loop_,
        Loop::Loop(
            Block {
                statements: vec![Statement::Expression(Expression::UnaryExpr(
                    UnaryExpr::PrimaryExpr(PrimaryExpr {
                        operand: Operand::Literal(Literal {
                            kind: LiteralKind::Number(2),
                            span: tokens[3].span.clone()
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
