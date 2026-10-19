use super::{Cursor, PieceKind, RockError, RockPiece, RockView};
use crate::lexer::{LexerError, TokenType};

impl<'a> RockView<'a> {
    pub(super) fn quoted(&mut self, start: Cursor<'a>, quote: char) -> Result<RockPiece, RockError> {
        self.take(1);
        let body_start = self.cursor;
        let mut escaped = false;
        while let Some(character) = self.peek(start, 0) {
            if !escaped && character == quote {
                let value = self.spelling(body_start)?;
                self.take(1);
                let token = if quote == '\'' { TokenType::Char(value) } else { TokenType::String(value) };
                return self.finish(start, PieceKind::Token(token));
            }
            self.take(1);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            }
        }
        Err(RockError::Lexer(LexerError::UnknownToken(
            quote, self.span(start, self.cursor.offset - start.offset),
        )))
    }

    pub(super) fn comment(&mut self, start: Cursor<'a>) -> Result<RockPiece, RockError> {
        let line_comment = self.peek(start, 1) == Some('/');
        self.take(2);
        if line_comment {
            // EOF is a valid line-comment terminator, not a sentinel to scan past.
            self.take_while(start, |character| character != '\n');
            return self.finish(start, PieceKind::LineComment);
        }
        while let Some(character) = self.peek(start, 0) {
            if character == '*' && self.peek(start, 1) == Some('/') {
                self.take(2);
                return self.finish(start, PieceKind::BlockComment);
            }
            self.take(1);
        }
        Err(RockError::UnterminatedBlockComment(
            self.span(start, self.cursor.offset - start.offset),
        ))
    }
}
