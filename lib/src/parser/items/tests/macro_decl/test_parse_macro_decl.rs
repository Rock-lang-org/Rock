use crate::lexer::TokenType;
use crate::parser::items::tests::common::*;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_macro_decl() {
    let input = "macro mymacro\n    $a:ident, $b:ty =>\n        statement";
    let tokens = lex(input);
    let tokens = &tokens[1..]; // skip the Indent(0)
    let config = Config::default();

    let (rest, macro_decl) = macro_decl(ParseCtx::from(tokens, &config)).unwrap();

    assert_eq!(macro_decl.name.name, "mymacro");
    assert_eq!(macro_decl.entries.len(), 1);
    assert_eq!(rest.len(), 0);
}

fn parse_macro_decl_error(input: &str) -> String {
    let tokens = lex(input);
    let config = Config::default();

    match crate::parser::parse_module_tokens(&tokens, &config) {
        Err(ParseError::HardError(message, _)) => message,
        other => panic!("expected a hard macro parse error, got {other:?}"),
    }
}

#[test]
fn test_parse_macro_repetition_keeps_inner_close_paren() {
    let input = "macro m\n    $( ( $a:ident ) )* =>\n        $( .f ($a) )*";
    let tokens = lex(input);
    let tokens = &tokens[1..]; // skip the Indent(0)
    let config = Config::default();

    let (_, macro_decl) = macro_decl(ParseCtx::from(tokens, &config)).unwrap();
    let entry = &macro_decl.entries[0];

    let MacroFragment::Repetition(head) = &entry.defs[0] else {
        panic!("expected a matcher repetition, got {:?}", entry.defs[0]);
    };
    assert!(matches!(
        head.last(),
        Some(MacroFragment::Token(token)) if token.token_type == TokenType::CloseParen
    ));

    let body_repetition = entry
        .body
        .iter()
        .find_map(|fragment| match fragment {
            MacroFragment::Repetition(inner) => Some(inner),
            _ => None,
        })
        .expect("template repetition");
    assert!(matches!(
        body_repetition.last(),
        Some(MacroFragment::Token(token)) if token.token_type == TokenType::CloseParen
    ));
}

#[test]
fn test_parse_macro_matcher_repetition_requires_star_close() {
    let message = parse_macro_decl_error("macro m\n    $( $a:ident ) =>\n        x = -> 1");

    assert!(message.contains("Unclosed macro repetition"), "{message}");
}

#[test]
fn test_parse_macro_template_repetition_requires_star_close() {
    let message =
        parse_macro_decl_error("macro m\n    $( $a:ident )* =>\n        $( $a = -> 1 )\nx = -> 2");

    assert!(message.contains("Unclosed macro repetition"), "{message}");
}

#[test]
fn test_parse_macro_rejects_stray_repetition_close() {
    let message = parse_macro_decl_error("macro m\n    $a:ident )* =>\n        x = -> 1");

    assert!(
        message.contains("outside of a macro repetition"),
        "{message}"
    );
}
