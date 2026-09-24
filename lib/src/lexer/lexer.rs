use std::path::PathBuf;

use crate::lexer::{span::Span, Token, TokenType};

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

pub struct Lexer {
    file_path: PathBuf,
    input: String,
    position: usize,
    last_token: Option<Token>,
    /// true by default, is turned off for the parser unit tests
    add_empty_newline_at_end: bool,
    /// tracks whether a comment was just skipped (for EOL skipping)
    after_comment: bool,
}

impl Lexer {
    pub fn new(file_path: PathBuf, input: &str) -> Result<Self, LexerError> {
        Ok(Lexer {
            file_path,
            input: input.to_string(),
            position: 0,
            last_token: None,
            add_empty_newline_at_end: true,
            after_comment: false,
        })
    }

    #[cfg(test)]
    pub fn with_newline_at_end(mut self, add_empty_newline_at_end: bool) -> Self {
        self.add_empty_newline_at_end = add_empty_newline_at_end;

        self
    }

    pub fn next(&mut self) -> Result<Token, LexerError> {
        let token = self.match_current_char()?;

        self.position = token.span.end;

        // Check if we need to skip this token
        let should_skip = if let TokenType::Comment(_) = token.token_type {
            true
        } else if let TokenType::Eol = token.token_type {
            // Skip EOL if a comment was just skipped
            self.after_comment
        } else if let TokenType::Indent(_) = token.token_type {
            // Look ahead to see if this indent is followed by a comment
            // If so, skip the indent (but not if this is the very first token)
            if self.last_token.is_none() && !self.after_comment {
                false // Don't skip the first token
            } else {
                let next_pos = token.span.end;
                let saved_pos = self.position;
                self.position = next_pos;

                if let Ok(next_token) = self.match_current_char() {
                    self.position = saved_pos; // Restore position
                    matches!(next_token.token_type, TokenType::Comment(_))
                } else {
                    self.position = saved_pos; // Restore position
                    false
                }
            }
        } else {
            false
        };

        if should_skip {
            if matches!(token.token_type, TokenType::Comment(_)) {
                self.after_comment = !self.comment_has_code_before(token.span.start);
            } else if matches!(token.token_type, TokenType::Eol) {
                self.after_comment = false;
            }
            return self.next();
        }

        self.after_comment = false;

        self.last_token = Some(token.clone());
        Ok(token)
    }

    pub fn collect(&mut self) -> Result<Vec<Token>, LexerError> {
        let mut tokens = Vec::new();

        loop {
            let token = self.next()?;

            if token.token_type == TokenType::Eof {
                break;
            }

            tokens.push(token);
        }

        if self.add_empty_newline_at_end {
            if let Some(token) = tokens.last() {
                if token.token_type != TokenType::Eol {
                    // Finish the current line
                    tokens.push(self.token(TokenType::Eol, 1));
                }
            }
            // add an empty line at the end of the file
            tokens.push(self.token(TokenType::Indent(0), 0));
            tokens.push(self.token(TokenType::Eol, 1));
        }

        tokens.push(Token {
            token_type: TokenType::Eof,
            span: Span {
                file_path: self.file_path.clone(),
                start: self.input.len(),
                end: self.input.len(),
            },
        });

        Ok(tokens)
    }

    fn comment_has_code_before(&self, comment_start: usize) -> bool {
        let line_start = self.input[..comment_start]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let line_prefix = &self.input[line_start..comment_start];
        Self::line_prefix_has_code(line_prefix)
    }

    fn line_prefix_has_code(prefix: &str) -> bool {
        let mut index = 0;
        while index < prefix.len() {
            let rest = &prefix[index..];
            if rest.starts_with("//") {
                return false;
            }
            if rest.starts_with("/*") {
                let Some(close) = rest[2..].find("*/") else {
                    return false;
                };
                index += 2 + close + 2;
                continue;
            }

            let ch = rest
                .chars()
                .next()
                .expect("index should be on char boundary");
            if !ch.is_whitespace() {
                return true;
            }
            index += ch.len_utf8();
        }

        false
    }

    fn span(&self, len: usize) -> Span {
        Span {
            file_path: self.file_path.clone(),
            start: self.position,
            end: self.position + len,
        }
    }

    fn token(&self, token_type: TokenType, len: usize) -> Token {
        Token {
            token_type,
            span: self.span(len),
        }
    }

    fn match_current_char(&mut self) -> Result<Token, LexerError> {
        // Handle the space dot
        if self.current_char() == ' '
            && self.peek(1) == '.'
            && self.peek(2) != '.'
            // special case to differenciate the dot operator
            && self.peek(2) != ' '
        {
            return Ok(self.token(TokenType::SpacedDot, 2));
        }

        // Skip whitespace except when in the start of the file for indentation
        if self.prev_char() != '\n' && self.position != 0 || self.position == self.input.len() {
            self.skip_whitespace();
        } else if let Some(token) = &self.last_token {
            match token.token_type {
                TokenType::Indent(_) => (),
                _ => {
                    return Ok(self.indent());
                }
            }
        } else {
            return Ok(self.indent());
        }

        let token = match self.current_char() {
            '\n' => self.token(TokenType::Eol, 1),
            '!' if self.peek(1) == '-' && self.peek(2) == '>' => {
                self.token(TokenType::UnitArrow, 3)
            }
            '-' if self.peek(1) == '>' => self.token(TokenType::Arrow, 2),
            '~' if self.peek(1) == '>' => self.token(TokenType::CurriedArrow, 2),
            '\\' => self.token(TokenType::TypeLambda, 1),
            '=' if self.peek(1) == '>' => self.token(TokenType::FatArrow, 2),
            '$' if self.peek(1).is_alphabetic() => self.macro_var(),
            '$' if self.peek(1) == '(' => self.token(TokenType::MacroRepeatOpen, 2),
            ')' if self.peek(1) == '*' => self.token(TokenType::MacroRepeatClose, 2),
            '%' if self.peek(1).is_alphabetic() && self.peek(1).is_lowercase() => {
                self.macro_invoc()
            }
            '^' => self.token(TokenType::Caret, 1),
            '~' if self.peek(1).is_alphabetic() && self.peek(1).is_uppercase() => {
                self.native_operator()
            }
            '~' => self.token(TokenType::Tilde, 1),
            '/' if self.peek(1) == '/' => self.comment_eol(),
            '/' if self.peek(1) == '*' => self.comment(),
            '&' if self.peek(1) == '&' => self.token(TokenType::Operator("&&".to_string()), 2),
            '&' => self.token(TokenType::Ampersand, 1),
            c if OPERATORS_CHARS.contains(&c) => self.operator(),
            '(' => self.token(TokenType::OpenParen, 1),
            ')' => self.token(TokenType::CloseParen, 1),
            '[' => self.token(TokenType::OpenBracket, 1),
            ']' => self.token(TokenType::CloseBracket, 1),
            ',' => self.token(TokenType::Coma, 1),
            ':' if self.peek(1) == ':' => self.token(TokenType::DoubleColon, 2),
            ':' => self.token(TokenType::Colon, 1),
            '.' if self.peek(1) == ' ' => self.operator(),
            '.' if self.peek(1) == '.' && self.peek(2) == '=' => {
                self.token(TokenType::DoubleDotEqual, 3)
            }
            '.' if self.peek(1) == '.' => self.token(TokenType::DoubleDot, 2),
            '.' => self.token(TokenType::Dot, 1),
            '?' => self.token(TokenType::Interogation, 1),
            '\'' => self.char(),
            '"' => self.string(),
            '@' => self.token(TokenType::Arobase, 1),
            '_' if self.peek(1).is_alphanumeric() || self.peek(1) == '_' => {
                self.ident_or_keyword_or_type()
            }
            '_' => self.token(TokenType::Underscore, 1),
            c if c.is_alphabetic() => self.ident_or_keyword_or_type(),
            c if c.is_ascii_digit() => self.number(),
            '\0' => self.token(TokenType::Eof, 1),
            c => return Err(LexerError::UnknownToken(c, self.span(1))),
        };

        Ok(token)
    }

    fn operator(&self) -> Token {
        let start = self.position;
        let mut end = self.position;

        if self.peek(0) == '.' {
            end += 1;
        } else {
            while OPERATORS_CHARS.contains(&self.peek(end - start)) {
                end += 1;
            }
        }

        if self.input[start..end] == *"=" {
            self.token(TokenType::Equal, 1)
        } else if self.input.len() > 2 && self.input[0..2] == *". " {
            self.token(TokenType::Operator(self.input[0..1].to_string()), 2)
        } else if self.input.len() > end + 1
            && (self.input[end..end + 1] == *" " || self.input[end..end + 1] == *"\n")
        {
            self.token(
                TokenType::Operator(self.input[start..end].to_string()),
                end - start,
            )
        } else {
            self.token(
                TokenType::StuckOperator(self.input[start..end].to_string()),
                end - start,
            )
        }
    }

    fn ident_or_keyword_or_type(&self) -> Token {
        let start = self.position;
        let mut end = self.position;

        while self.peek(end - start).is_alphanumeric() || self.peek(end - start) == '_' {
            end += 1;
        }

        let ident = self.input[start..end].to_string();

        if self.current_char().is_uppercase() {
            self.token(TokenType::Type(ident), end - start)
        } else if KEYWORDS.contains(&ident.as_str()) {
            self.token(TokenType::Keyword(ident), end - start)
        } else {
            self.token(TokenType::Ident(ident), end - start)
        }
    }

    fn macro_var(&self) -> Token {
        let start = self.position;
        let mut end = self.position;

        while self.peek(end - start + 1).is_alphanumeric() || self.peek(end - start + 1) == '_' {
            end += 1;
        }

        self.token(
            TokenType::MacroVar(self.input[start + 1..end + 1].to_string()),
            end - start + 1,
        )
    }

    fn comment_eol(&self) -> Token {
        let start = self.position;
        let mut end = self.position;

        // consume the '//'
        end += 2;

        while self.peek(end - start) != '\n' {
            end += 1;
        }

        self.token(TokenType::Comment(String::new()), end - start)
    }

    fn comment(&self) -> Token {
        let start = self.position;
        let mut end = self.position;

        // consume the '/*'
        end += 2;

        while self.peek(end - start) != '*' && self.peek(end - start + 1) != '/' {
            end += 1;
        }

        // consume the '*/'
        end += 2;

        self.token(TokenType::Comment(String::new()), end - start)
    }

    fn macro_invoc(&self) -> Token {
        let start = self.position;
        let mut end = self.position;

        while self.peek(end - start + 1).is_alphanumeric() || self.peek(end - start + 1) == '_' {
            end += 1;
        }

        self.token(
            TokenType::MacroInvoc(self.input[start + 1..end + 1].to_string()),
            end - start + 1,
        )
    }

    fn native_operator(&self) -> Token {
        let start = self.position;
        let mut end = self.position;

        while self.peek(end - start + 1).is_alphabetic()
            || self.peek(end - start + 1).is_ascii_digit()
            || self.peek(end - start + 1) == '_'
        {
            end += 1;
        }

        self.token(
            TokenType::NativeOperator(self.input[start + 1..end + 1].to_string()),
            end - start + 1,
        )
    }

    fn number(&self) -> Token {
        let start = self.position;
        let mut end = self.position;

        while self.peek(end - start).is_ascii_digit() {
            end += 1;
        }

        if self.peek(end - start) == '.' {
            let old_end = end;
            end += 1;

            while self.peek(end - start).is_ascii_digit() {
                end += 1;
            }
            if end > old_end + 1 {
                return self.token(
                    TokenType::Float(self.input[start..end].to_string()),
                    end - start,
                );
            } else {
                end = old_end;
            }
        }

        self.token(
            TokenType::Number(self.input[start..end].to_string()),
            end - start,
        )
    }

    fn char(&self) -> Token {
        let start = self.position;
        let mut end = self.position;

        // We deliberately allow multi-character literals here to handle escaped chars
        // The other errors should be catched in the parser
        while self.peek(end - start + 1) != '\'' {
            end += 1;
        }

        self.token(
            TokenType::Char(self.input[start + 1..end + 1].to_owned()),
            end - start + 2,
        )
    }

    fn string(&self) -> Token {
        let start = self.position;
        let mut end = self.position;

        loop {
            let ch = self.peek(end - start + 1);
            if ch == '"' {
                break;
            }
            // Skip escaped characters (e.g., \", \\, \n)
            if ch == '\\' {
                end += 2; // skip backslash + next char
                continue;
            }
            end += 1;
        }

        self.token(
            TokenType::String(self.input[start + 1..end + 1].to_string()),
            end - start + 2,
        )
    }

    fn indent(&mut self) -> Token {
        let start = self.position;
        let mut end = self.position;

        while self.peek(end - start).is_whitespace() && self.peek(end - start) != '\n' {
            end += 1;
        }

        let indent_level = if self.peek(end - start) == '\n' {
            0
        } else {
            end - start
        };

        self.token(TokenType::Indent(indent_level as u8), end - start)
    }

    fn skip_whitespace(&mut self) {
        while self.current_char().is_whitespace() && self.current_char() != '\n' {
            self.position += 1;
        }
    }

    fn current_char(&self) -> char {
        self.input[self.position..].chars().next().unwrap_or('\0')
    }

    fn prev_char(&self) -> char {
        self.input[..self.position]
            .chars()
            .next_back()
            .unwrap_or('\0')
    }

    fn peek(&self, n: usize) -> char {
        self.input[self.position..].chars().nth(n).unwrap_or('\0')
    }
}

#[cfg(test)]
mod lexer_tests {
    use super::*;
    use std::path::PathBuf;

    fn lex_input(input: &str) -> Result<Vec<Token>, LexerError> {
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
