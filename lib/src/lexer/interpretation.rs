//! Bounded Rock interpretation of retained lossless source segments.

use std::ops::Range;

use super::lexer::{KEYWORDS, OPERATORS_CHARS};
use super::lossless::{JoinBoundary, LosslessError, LosslessStream, SyntaxContext};
use super::{LexerError, Span, TokenType};

#[derive(Debug, Clone, PartialEq)]
pub enum PieceKind {
    Token(TokenType),
    Whitespace,
    LineEnding,
    LineComment,
    BlockComment,
}

#[derive(Debug, Clone)]
pub struct RockPiece {
    pub kind: PieceKind,
    /// Logical bytes in the supplied stream, retaining access to every segment.
    pub range: Range<usize>,
    pub span: Span,
    /// Only binding-bearing spellings require one agreed syntax context.
    pub context: Option<SyntaxContext>,
}

#[derive(Debug, Clone)]
pub enum RockError {
    Lexer(LexerError),
    Preservation(LosslessError),
    UnterminatedBlockComment(Span),
}

#[derive(Clone, Copy)]
struct Cursor<'a> {
    stream: &'a LosslessStream,
    run: usize,
    byte: usize,
    offset: usize,
}

impl Cursor<'_> {
    fn current(&self) -> Option<char> {
        self.stream.runs().get(self.run)?.text()[self.byte..].chars().next()
    }

    fn ends_token(&self, start: Self) -> bool {
        self.current().is_none() || (self.offset > start.offset && self.byte == 0
            && self.stream.runs()[self.run].boundary() == JoinBoundary::Separate)
    }

    fn bump(&mut self) -> Option<char> {
        let character = self.current()?;
        self.byte += character.len_utf8();
        self.offset += character.len_utf8();
        if self.byte == self.stream.runs()[self.run].text().len() {
            self.run += 1;
            self.byte = 0;
        }
        Some(character)
    }
}

/// Reads segments directly, without flattening text or treating NUL as EOF.
/// Physical layout and parser-added terminal tokens belong to A0.06.
pub struct RockView<'a> {
    cursor: Cursor<'a>,
}

impl<'a> RockView<'a> {
    pub fn new(stream: &'a LosslessStream) -> Self {
        Self { cursor: Cursor { stream, run: 0, byte: 0, offset: 0 } }
    }

    pub fn offset(&self) -> usize {
        self.cursor.offset
    }

    fn peek(&self, start: Cursor<'a>, distance: usize) -> Option<char> {
        let mut cursor = self.cursor;
        for _ in 0..distance {
            if cursor.ends_token(start) {
                return None;
            }
            cursor.bump()?;
        }
        if cursor.ends_token(start) { None } else { cursor.current() }
    }

    fn take(&mut self, count: usize) {
        for _ in 0..count {
            let consumed = self.cursor.bump();
            debug_assert!(consumed.is_some());
        }
    }

    fn take_while(&mut self, start: Cursor<'a>, predicate: impl Fn(char) -> bool) {
        while self.peek(start, 0).map(&predicate).unwrap_or(false) {
            self.take(1);
        }
    }

    fn spelling(&self, start: Cursor<'a>) -> Result<String, RockError> {
        let mut text = String::new();
        text.try_reserve_exact(self.cursor.offset - start.offset)
            .map_err(|_| RockError::Preservation(LosslessError::AllocationFailed { offset: start.offset }))?;
        let end_run = self.cursor.run + usize::from(self.cursor.byte != 0);
        for index in start.run..end_run {
            let run = &start.stream.runs()[index];
            let begin = if index == start.run { start.byte } else { 0 };
            let end = if index == self.cursor.run { self.cursor.byte } else { run.text().len() };
            text.push_str(&run.text()[begin..end]);
        }
        Ok(text)
    }

    fn span(&self, start: Cursor<'a>, len: usize) -> Span {
        let run = &start.stream.runs()[start.run];
        let span = run.span();
        Span::new(span.file_path, span.start + start.byte, span.start + start.byte + len)
    }

    fn finish(&self, start: Cursor<'a>, kind: PieceKind) -> Result<RockPiece, RockError> {
        let binding = matches!(&kind, PieceKind::Token(
            TokenType::Ident(_) | TokenType::Type(_) | TokenType::MacroVar(_)
            | TokenType::MacroInvoc(_) | TokenType::NativeOperator(_)
            | TokenType::Operator(_) | TokenType::StuckOperator(_)
        ));
        let context = if binding {
            let first = start.stream.runs()[start.run].context();
            let end_run = self.cursor.run + usize::from(self.cursor.byte != 0);
            let mut offset = start.offset;
            for index in start.run..end_run {
                let run = &start.stream.runs()[index];
                if run.context() != first {
                    return Err(RockError::Preservation(LosslessError::ConflictingContexts {
                        offset, left: first, right: run.context(),
                    }));
                }
                offset += run.text().len() - if index == start.run { start.byte } else { 0 };
            }
            Some(first)
        } else {
            None
        };
        Ok(RockPiece {
            span: self.span(start, self.cursor.offset - start.offset),
            range: start.offset..self.cursor.offset,
            kind,
            context,
        })
    }

    /// An error does not consume input, so a failed view cannot destroy a capture.
    pub fn next_piece(&mut self) -> Result<Option<RockPiece>, RockError> {
        let saved = self.cursor;
        let result = self.scan_piece();
        if result.is_err() {
            self.cursor = saved;
        }
        result
    }

    fn scan_piece(&mut self) -> Result<Option<RockPiece>, RockError> {
        let start = self.cursor;
        let Some(character) = self.peek(start, 0) else { return Ok(None) };
        if character == ' ' && self.peek(start, 1) == Some('.')
            && !matches!(self.peek(start, 2), Some('.' | ' '))
        {
            self.take(2);
            return self.finish(start, PieceKind::Token(TokenType::SpacedDot)).map(Some);
        }
        if character == '\n' || (character == '\r' && self.peek(start, 1) == Some('\n')) {
            self.take(if character == '\r' { 2 } else { 1 });
            return self.finish(start, PieceKind::LineEnding).map(Some);
        }
        if character.is_whitespace() {
            self.take_while(start, |ch| ch.is_whitespace() && ch != '\n');
            return self.finish(start, PieceKind::Whitespace).map(Some);
        }
        let fixed = match character {
            '!' if self.peek(start, 1) == Some('-') && self.peek(start, 2) == Some('>') => Some((TokenType::UnitArrow, 3)),
            '-' if self.peek(start, 1) == Some('>') => Some((TokenType::Arrow, 2)),
            '~' if self.peek(start, 1) == Some('>') => Some((TokenType::CurriedArrow, 2)),
            '=' if self.peek(start, 1) == Some('>') => Some((TokenType::FatArrow, 2)),
            '\\' => Some((TokenType::TypeLambda, 1)),
            '$' if self.peek(start, 1) == Some('(') => Some((TokenType::MacroRepeatOpen, 2)),
            ')' if self.peek(start, 1) == Some('*') => Some((TokenType::MacroRepeatClose, 2)),
            '&' if self.peek(start, 1) == Some('&') => Some((TokenType::Operator("&&".into()), 2)),
            '&' => Some((TokenType::Ampersand, 1)),
            ':' if self.peek(start, 1) == Some(':') => Some((TokenType::DoubleColon, 2)),
            ':' => Some((TokenType::Colon, 1)),
            '.' if self.peek(start, 1) == Some(' ') => Some((TokenType::Operator(".".into()), 1)),
            '.' if self.peek(start, 1) == Some('.') && self.peek(start, 2) == Some('=') => Some((TokenType::DoubleDotEqual, 3)),
            '.' if self.peek(start, 1) == Some('.') => Some((TokenType::DoubleDot, 2)),
            '.' => Some((TokenType::Dot, 1)),
            '(' => Some((TokenType::OpenParen, 1)),
            ')' => Some((TokenType::CloseParen, 1)),
            '[' => Some((TokenType::OpenBracket, 1)),
            ']' => Some((TokenType::CloseBracket, 1)),
            '{' => Some((TokenType::OpenBrace, 1)),
            '}' => Some((TokenType::CloseBrace, 1)),
            ',' => Some((TokenType::Coma, 1)),
            '^' => Some((TokenType::Caret, 1)),
            '?' => Some((TokenType::Interogation, 1)),
            '@' => Some((TokenType::Arobase, 1)),
            '_' if !self.peek(start, 1).map(|ch| ch.is_alphanumeric() || ch == '_').unwrap_or(false) => Some((TokenType::Underscore, 1)),
            '~' if !self.peek(start, 1).map(|ch| ch.is_alphabetic() && ch.is_uppercase()).unwrap_or(false) => Some((TokenType::Tilde, 1)),
            _ => None,
        };
        if let Some((kind, count)) = fixed {
            self.take(count);
            return self.finish(start, PieceKind::Token(kind)).map(Some);
        }
        let prefixed = match character {
            '$' => self.peek(start, 1).map(char::is_alphabetic).unwrap_or(false),
            '%' => self.peek(start, 1).map(|ch| ch.is_alphabetic() && ch.is_lowercase()).unwrap_or(false),
            '~' => true,
            _ => false,
        };
        let kind = if prefixed {
            self.take(1);
            self.take_while(start, |ch| ch == '_' || if character == '~' {
                ch.is_alphabetic() || ch.is_ascii_digit()
            } else {
                ch.is_alphanumeric()
            });
            let mut name = self.spelling(start)?;
            name.remove(0); // The marker is one ASCII byte; retain the allocated spelling.
            match character {
                '$' => TokenType::MacroVar(name),
                '%' => TokenType::MacroInvoc(name),
                _ => TokenType::NativeOperator(name),
            }
        } else if character.is_alphabetic() || character == '_' {
            self.take_while(start, |ch| ch.is_alphanumeric() || ch == '_');
            let spelling = self.spelling(start)?;
            if character.is_uppercase() {
                TokenType::Type(spelling)
            } else if KEYWORDS.contains(&spelling.as_str()) {
                TokenType::Keyword(spelling)
            } else {
                TokenType::Ident(spelling)
            }
        } else if character.is_ascii_digit() {
            self.take_while(start, |ch| ch.is_ascii_digit());
            if self.peek(start, 0) == Some('.') && self.peek(start, 1).map(|ch| ch.is_ascii_digit()).unwrap_or(false) {
                self.take(1);
                self.take_while(start, |ch| ch.is_ascii_digit());
                TokenType::Float(self.spelling(start)?)
            } else {
                TokenType::Number(self.spelling(start)?)
            }
        } else if matches!(character, '\'' | '"') {
            return self.quoted(start, character).map(Some);
        } else if character == '/' && matches!(self.peek(start, 1), Some('/' | '*')) {
            return self.comment(start).map(Some);
        } else if OPERATORS_CHARS.contains(&character) {
            self.take_while(start, |ch| OPERATORS_CHARS.contains(&ch));
            let spelling = self.spelling(start)?;
            if spelling == "=" {
                TokenType::Equal
            } else if matches!(self.peek(start, 0), Some(' ' | '\n')) {
                TokenType::Operator(spelling)
            } else {
                TokenType::StuckOperator(spelling)
            }
        } else {
            return Err(RockError::Lexer(LexerError::UnknownToken(
                character, self.span(start, character.len_utf8()),
            )));
        };
        self.finish(start, PieceKind::Token(kind)).map(Some)
    }
}

mod delimited;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod delimited_tests;
