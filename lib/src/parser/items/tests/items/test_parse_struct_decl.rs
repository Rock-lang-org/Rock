use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;
use std::path::PathBuf;

#[test]
fn test_parse_struct_decl() {
    let tokens = lex_test("struct Foo\n    a: Int\n    b: Int = 0\n");
    let config = Config::default();

    let (tokens, struct_decl) = struct_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(
        struct_decl,
        StructDecl {
            name: ParseTypeInner { context: Default::default(),
                name: "Foo".to_string(),
                generics: vec![],
                span: Span {
                    start: 7,
                    end: 10,
                    file_path: PathBuf::from("/test.rk"),
                },
            },
            generic_params: vec![],
            fields: vec![
                StructDeclField {
                    name: Ident { context: Default::default(),
                        name: "a".to_string(),
                        span: Span {
                            start: 15,
                            end: 16,
                            file_path: PathBuf::from("/test.rk"),
                        },
                    },
                    ty: ParseType::Type(ParseTypeInner { context: Default::default(),
                        name: "Int".to_string(),
                        generics: vec![],
                        span: Span {
                            start: 18,
                            end: 21,
                            file_path: PathBuf::from("/test.rk"),
                        }
                    }),
                    public: false,
                    default: None,
                },
                StructDeclField {
                    name: Ident { context: Default::default(),
                        name: "b".to_string(),
                        span: Span {
                            start: 26,
                            end: 27,
                            file_path: PathBuf::from("/test.rk"),
                        },
                    },
                    ty: ParseType::Type(ParseTypeInner { context: Default::default(),
                        name: "Int".to_string(),
                        generics: vec![],
                        span: Span {
                            start: 29,
                            end: 32,
                            file_path: PathBuf::from("/test.rk"),
                        }
                    }),
                    public: false,
                    default: Some(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                        operand: Operand::Literal(Literal {
                            kind: LiteralKind::Number(0),
                            span: Span {
                                start: 35,
                                end: 36,
                                file_path: PathBuf::from("/test.rk"),
                            }
                        }),
                        secondaries: None,
                        type_annotation: None,
                    }))),
                }
            ],
            exported: false,
        }
    );

    assert_eq!(tokens.len(), 0);
}
