use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;
use std::path::PathBuf;

#[test]
fn test_parse_ident() {
    let tokens = lex_test("foo");
    let config = Config::default();

    let (parse_ctx, ident) = ident(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(
        ident,
        Ident { context: Default::default(),
            name: "foo".to_string(),
            span: Span {
                start: 0,
                end: 3,
                file_path: PathBuf::from("/test.rk"),
            },
        }
    );

    assert_eq!(parse_ctx.len(), 0);
}
