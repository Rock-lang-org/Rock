use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn mut_ident_pattern() {
    let input = "mut a";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, pattern) = pattern.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(
        pattern,
        Pattern {
            binding: None,
            kind: PatternKind::Ident(IdentPattern {
                name: Ident { context: Default::default(),
                    name: "a".to_string(),
                    span: Span::test()
                },
                mut_: true,
            })
        }
    );

    assert_eq!(rest.len(), 0);
}

#[test]
fn caret_ident_pattern() {
    let input = "^ a";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, pattern) = pattern.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(
        pattern,
        Pattern {
            binding: None,
            kind: PatternKind::Ident(IdentPattern {
                name: Ident { context: Default::default(),
                    name: "a".to_string(),
                    span: Span::test()
                },
                mut_: true,
            })
        }
    );

    assert_eq!(rest.len(), 0);
}
