//! Rock vocabulary; source recognition lives only in the lossless views.

use crate::lexer::Span;

pub const KEYWORDS: [&str; 27] = [
    "struct", "enum", "trait", "impl", "if", "then", "else", "for", "in", "while", "loop", "macro",
    "true", "false", "return", "continue", "break", "infix", "mod", "extern", "match", "unsafe",
    "type", "mut", "where", "as", "do",
];
pub const OPERATORS_CHARS: [char; 13] = [
    '+', '-', '*', '/', '%', '=', '!', '<', '>', '$', '|', '&', ';',
];

#[derive(Debug, Clone)]
pub enum LexerError {
    UnknownToken(char, Span),
}

#[cfg(test)]
pub use super::test_support::Lexer;

#[cfg(test)]
mod lexer_tests {
    use super::*;
    use crate::lexer::interpretation::RockError;
    use crate::lexer::{Token, TokenType};
    use std::path::PathBuf;

    #[test]
    fn unicode_literals_use_byte_spans_and_preserve_following_tokens() {
        let input = "\"é中🦀\" '🦀' '\\'' 42";
        let tokens = lex_input(input).unwrap();
        let literal = tokens
            .iter()
            .find(|t| matches!(t.token_type, TokenType::String(_)))
            .unwrap();
        assert_eq!(&input[literal.span.start..literal.span.end], "\"é中🦀\"");
        assert!(tokens
            .iter()
            .any(|t| matches!(&t.token_type, TokenType::Char(c) if c == "🦀")));
        assert!(tokens
            .iter()
            .any(|t| matches!(&t.token_type, TokenType::Char(c) if c == "\\'")));
        assert!(tokens
            .iter()
            .any(|t| matches!(&t.token_type, TokenType::Number(n) if n == "42")));
    }

    #[test]
    fn unterminated_unicode_literals_report_errors() {
        for input in ["\"🦀", "'é", "\"escaped\\\""] {
            assert!(lex_input(input).is_err());
        }
    }

    fn lex_input(input: &str) -> Result<Vec<Token>, RockError> {
        let mut lexer = Lexer::new(PathBuf::from("/test.rk"), input)?;
        lexer.collect()
    }

    #[test]
    fn test_line_comment() {
        let input = "// this is a comment\nfoo";
        let tokens = lex_input(input).unwrap();

        // Comments should be filtered out by the lexer
        // We should get: Indent(0), Ident("foo"), Eol, Indent(0), Eol, Eof
        let non_eof_tokens: Vec<_> = tokens
            .iter()
            .filter(|t| t.token_type != TokenType::Eof)
            .collect();

        // Should have identifier "foo" somewhere in the tokens
        let has_foo = non_eof_tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::Ident(ref name) if name == "foo"));
        assert!(has_foo, "Expected to find identifier 'foo' in tokens");
    }

    #[test]
    fn test_block_comment() {
        let input = "/* this is a block comment */bar";
        let tokens = lex_input(input).unwrap();

        // Comments should be filtered out by the lexer
        let has_bar = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::Ident(ref name) if name == "bar"));
        assert!(has_bar, "Expected to find identifier 'bar' in tokens");
    }

    #[test]
    fn test_spaced_dot_operator() {
        let input = "obj .method";
        let tokens = lex_input(input).unwrap();

        let has_spaced_dot = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::SpacedDot));
        assert!(has_spaced_dot, "Expected to find spaced dot token");
    }

    #[test]
    fn test_macro_tokens() {
        let input = "$var %macro $(repeat)*";
        let tokens = lex_input(input).unwrap();

        let has_macro_var = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::MacroVar(ref name) if name == "var"));
        let has_macro_invoc = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::MacroInvoc(ref name) if name == "macro"));
        let has_macro_repeat_open = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::MacroRepeatOpen));
        let has_macro_repeat_close = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::MacroRepeatClose));

        assert!(has_macro_var, "Expected macro variable token");
        assert!(has_macro_invoc, "Expected macro invocation token");
        assert!(has_macro_repeat_open, "Expected macro repeat open token");
        assert!(has_macro_repeat_close, "Expected macro repeat close token");
    }

    #[test]
    fn test_native_operator() {
        let input = "~IAdd";
        let tokens = lex_input(input).unwrap();

        let has_native_op = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::NativeOperator(ref name) if name == "IAdd"));
        assert!(has_native_op, "Expected native operator token");
    }

    #[test]
    fn test_leading_underscore_identifiers() {
        let input = "_ _name __errno_location";
        let tokens = lex_input(input).unwrap();

        assert!(matches!(tokens[1].token_type, TokenType::Underscore));
        assert!(matches!(tokens[2].token_type, TokenType::Ident(ref name) if name == "_name"));
        assert!(
            matches!(tokens[3].token_type, TokenType::Ident(ref name) if name == "__errno_location")
        );
    }

    #[test]
    fn test_arrows_and_special_operators() {
        let input = "-> !-> ~> \\T -> T => :: .. ?";
        let tokens = lex_input(input).unwrap();

        let has_arrow = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::Arrow));
        let has_unit_arrow = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::UnitArrow));
        let has_curried_arrow = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::CurriedArrow));
        let has_type_lambda = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::TypeLambda));
        let has_fat_arrow = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::FatArrow));
        let has_double_colon = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::DoubleColon));
        let has_double_dot = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::DoubleDot));
        let has_interogation = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::Interogation));

        assert!(has_arrow, "Expected arrow token");
        assert!(has_unit_arrow, "Expected unit arrow token");
        assert!(has_curried_arrow, "Expected curried arrow token");
        assert!(has_type_lambda, "Expected type lambda token");
        assert!(has_fat_arrow, "Expected fat arrow token");
        assert!(has_double_colon, "Expected double colon token");
        assert!(has_double_dot, "Expected double dot token");
        assert!(has_interogation, "Expected interogation token");
    }

    #[test]
    fn test_caret_and_tilde_tokens() {
        let input = "^ ~ ^@ ~@";
        let tokens = lex_input(input).unwrap();

        let has_caret = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::Caret));
        let has_tilde = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::Tilde));

        assert!(has_caret, "Expected caret token");
        assert!(has_tilde, "Expected tilde token");
    }

    #[test]
    fn test_keywords() {
        let input = "struct enum trait impl if then else unsafe";
        let tokens = lex_input(input).unwrap();

        let keywords = [
            "struct", "enum", "trait", "impl", "if", "then", "else", "unsafe",
        ];
        for keyword in &keywords {
            let has_keyword = tokens
                .iter()
                .any(|t| matches!(t.token_type, TokenType::Keyword(ref k) if k == keyword));
            assert!(has_keyword, "Expected keyword '{}' token", keyword);
        }
    }

    #[test]
    fn test_numbers_and_floats() {
        let input = "123 45.67 89";
        let tokens = lex_input(input).unwrap();

        let has_int_123 = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::Number(ref n) if n == "123"));
        let has_float = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::Float(ref f) if f == "45.67"));
        let has_int_89 = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::Number(ref n) if n == "89"));

        assert!(has_int_123, "Expected number '123' token");
        assert!(has_float, "Expected float '45.67' token");
        assert!(has_int_89, "Expected number '89' token");
    }

    #[test]
    fn test_strings_and_chars() {
        let input = r#""hello world" 'c'"#;
        let tokens = lex_input(input).unwrap();

        let has_string = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::String(ref s) if s == "hello world"));
        let has_char = tokens
            .iter()
            .any(|t| matches!(t.token_type, TokenType::Char(ref c) if c == "c"));

        assert!(has_string, "Expected string token");
        assert!(has_char, "Expected char token");
    }

    #[test]
    fn test_indentation() {
        let input = "foo\n    bar\n        baz";
        let tokens = lex_input(input).unwrap();

        // Should have different indentation levels
        let indent_levels: Vec<u8> = tokens
            .iter()
            .filter_map(|t| match &t.token_type {
                TokenType::Indent(level) => Some(*level),
                _ => None,
            })
            .collect();

        // Should have at least indentation levels 0, 4, and 8
        assert!(indent_levels.contains(&0), "Expected indent level 0");
        assert!(indent_levels.contains(&4), "Expected indent level 4");
        assert!(indent_levels.contains(&8), "Expected indent level 8");
    }
}
