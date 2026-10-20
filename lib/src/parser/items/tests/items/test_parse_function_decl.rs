use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;
use std::path::PathBuf;

#[test]
fn test_parse_function_decl() {
    let tokens = lex_test("a = foo -> foo\n");
    let config = Config::default();

    let (tokens, function_decl) = function_decl(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(
        function_decl,
        FunctionDecl {
            name: Ident { context: Default::default(),
                name: "a".to_string(),
                span: Span {
                    start: 0,
                    end: 1,
                    file_path: PathBuf::from("/test.rk"),
                },
            },
            lambda: LambdaDecl {
                parameters: vec![Pattern {
                    binding: None,
                    kind: PatternKind::Ident(IdentPattern {
                        name: Ident { context: Default::default(),
                            name: "foo".to_string(),
                            span: Span {
                                start: 4,
                                end: 7,
                                file_path: PathBuf::from("/test.rk"),
                            },
                        },
                        mut_: false,
                    }),
                },],
                body: Block {
                    statements: vec![Statement::Expression(Expression::UnaryExpr(
                        UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Ident(IdentifierPath {
                                path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                                    name: "foo".to_string(),
                                    span: Span {
                                        start: 11,
                                        end: 14,
                                        file_path: PathBuf::from("/test.rk"),
                                    },
                                })]
                            }),
                            secondaries: None,
                            type_annotation: None,
                        })
                    ))]
                },
                arrow_kind: LambdaArrowKind::Normal,
                span: Span {
                    start: 8,
                    end: 10,
                    file_path: PathBuf::from("/test.rk"),
                },
            },
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }
    );

    assert_eq!(tokens.len(), 0);
}
