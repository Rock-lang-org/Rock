use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::assert_formatted_eq;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_assignment() {
    let input = "a = 1";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, statement) = statement.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_formatted_eq(
        &statement,
        &Statement::Assignment(Assignment {
            lhs: AssignmentLHS::Pattern {
                pattern: Pattern {
                    binding: None,
                    kind: PatternKind::Ident(IdentPattern {
                        name: Ident { context: Default::default(),
                            name: "a".to_string(),
                            span: Span::test(),
                        },
                        mut_: false,
                    }),
                },
                type_annotation: None,
            },
            rhs: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Literal(Literal {
                    kind: crate::ast::LiteralKind::Number(1),
                    span: Span::test(),
                }),
                secondaries: None,
                type_annotation: None,
            })),
        }),
    );

    assert_eq!(rest.len(), 0);
}
