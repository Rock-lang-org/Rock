use crate::lexer::{Token, TokenType};
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_macro_entry() {
    let input = "macro make\n    $a:ident =>\n        statement";
    let config = Config::default();

    let program = parse_string(input, &config).unwrap();
    let TopLevel::MacroDecl(decl) = &program.module.top_levels[0] else { panic!("expected macro") };
    let macro_entry = &decl.entries[0];

    assert_eq!(macro_entry.defs.len(), 1);
    assert_eq!(program.module.top_levels.len(), 1);
}

#[test]
fn macro_entry_reports_odd_body_indent() {
    let tokens = vec![
        Token::from(TokenType::Indent(4)),
        Token::from(TokenType::MacroVar("name".to_string())),
        Token::from(TokenType::Colon),
        Token::from(TokenType::Ident("ident".to_string())),
        Token::from(TokenType::FatArrow),
        Token::from(TokenType::Eol),
        Token::from(TokenType::Indent(5)),
        Token::from(TokenType::Ident("statement".to_string())),
        Token::from(TokenType::Eof),
    ];
    let config = Config::default();
    let mut parse_ctx = ParseCtx::from(&tokens, &config);
    parse_ctx.indent_level = 4;
    parse_ctx.invalid_indent = None;

    let result = macro_entry(parse_ctx);

    assert!(matches!(result, Err(ParseError::UnexpectedIndent(5, _))));
}
