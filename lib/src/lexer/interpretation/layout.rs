//! Parser-facing layout, with retained original trivia and explicit terminal tokens.

use std::ops::Range;

use super::{PieceKind, RockError, RockView};
use crate::lexer::lossless::{LosslessStream, SyntaxContext};
use crate::lexer::{Span, Token, TokenType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutOrigin {
    Source(Range<usize>),
    Synthetic,
    EndOfInput,
}

#[derive(Debug, Clone)]
pub struct LayoutToken {
    pub token: Token,
    pub origin: LayoutOrigin,
    pub context: Option<SyntaxContext>,
}

#[derive(Clone, Copy)]
enum Terminal {
    Reading,
    FinishLine,
    EmptyIndent,
    EmptyEol,
    Eof,
    Done,
}

/// No whole-file lexing/lookahead: each request reads only through its token.
#[derive(Clone)]
pub struct LayoutView<'a> {
    view: RockView<'a>,
    terminal_layout: bool,
    terminal: Terminal,
    at_line_start: bool,
    line_has_code: bool,
    suppress_eol: bool,
    emitted: bool,
    last_was_eol: bool,
    last_was_indent: bool,
}

impl<'a> LayoutView<'a> {
    pub fn new(stream: &'a LosslessStream, terminal_layout: bool) -> Self {
        Self {
            view: RockView::new(stream),
            terminal_layout,
            terminal: Terminal::Reading,
            at_line_start: true,
            line_has_code: false,
            suppress_eol: false,
            emitted: false,
            last_was_eol: false,
            last_was_indent: false,
        }
    }

    pub fn offset(&self) -> usize {
        self.view.offset()
    }

    /// Suspend only at physical line boundaries; do not ask Rock to interpret
    /// any byte in the captured range, including quotes, comments or margins.
    pub fn capture_to(&mut self, end: usize, limits: crate::lexer::lossless::ScanLimits)
        -> Result<LosslessStream, RockError>
    {
        let stream = self.view.cursor.stream;
        let start = self.offset();
        let captured = stream.slice(start..end, limits).map_err(RockError::Preservation)?;
        if !matches!(self.terminal, Terminal::Reading) || !self.at_line_start {
            return Err(RockError::InvalidCaptureBoundary { span: captured.start_span() });
        }
        let mut cursor = self.view.cursor;
        let mut last = None;
        while cursor.offset < end {
            last = cursor.bump();
        }
        if end != start && end != stream.len_bytes() && last != Some('\n') {
            return Err(RockError::InvalidCaptureBoundary { span: captured.end_span() });
        }
        self.view.cursor = cursor;
        self.line_has_code = false;
        self.suppress_eol = false;
        self.last_was_indent = false;
        Ok(captured)
    }

    fn emit(&mut self, token: LayoutToken) -> LayoutToken {
        self.emitted = true;
        self.last_was_eol = token.token.token_type == TokenType::Eol;
        self.last_was_indent = matches!(token.token.token_type, TokenType::Indent(_));
        token
    }

    fn terminal_token(&mut self) -> Option<LayoutToken> {
        let (kind, next) = match self.terminal {
            Terminal::Reading | Terminal::Done => return None,
            Terminal::FinishLine => (TokenType::Eol, Terminal::EmptyIndent),
            Terminal::EmptyIndent => (TokenType::Indent(0), Terminal::EmptyEol),
            Terminal::EmptyEol => (TokenType::Eol, Terminal::Eof),
            Terminal::Eof => (TokenType::Eof, Terminal::Done),
        };
        self.terminal = next;
        let origin = if kind == TokenType::Eof { LayoutOrigin::EndOfInput } else { LayoutOrigin::Synthetic };
        let span = self.view.cursor.stream.end_span();
        Some(self.emit(LayoutToken { token: Token { token_type: kind, span }, origin, context: None }))
    }

    fn indentation(&mut self) -> Result<Option<LayoutToken>, RockError> {
        let start = self.view.cursor;
        self.view.take_while(start, |ch| ch.is_whitespace() && ch != '\n');
        let bytes = self.view.offset() - start.offset;
        let comment = self.view.peek(start, 0) == Some('/')
            && matches!(self.view.peek(start, 1), Some('/' | '*'));
        if self.emitted && comment {
            self.at_line_start = false;
            return Ok(None);
        }
        // The initial indent remains pending across leading comment-only lines.
        // Do not emit a second zero-width indent before their first code token.
        if self.last_was_indent && bytes == 0 {
            self.at_line_start = false;
            return Ok(None);
        }
        // Blank lines do not select an indentation step, even with many spaces.
        let width = if self.view.peek(start, 0) == Some('\n') { 0 } else { bytes };
        let span = self.view.span(start, bytes);
        let level = match u8::try_from(width) {
            Ok(level) => level,
            Err(_) => {
                self.view.cursor = start;
                return Err(RockError::IndentationTooWide { width, span });
            }
        };
        self.at_line_start = false;
        Ok(Some(self.emit(LayoutToken {
            token: Token { token_type: TokenType::Indent(level), span },
            origin: LayoutOrigin::Source(start.offset..self.view.offset()),
            context: None,
        })))
    }

    pub fn next_token(&mut self) -> Result<Option<LayoutToken>, RockError> {
        loop {
            if !matches!(self.terminal, Terminal::Reading) {
                return Ok(self.terminal_token());
            }
            if self.at_line_start && self.view.cursor.current().is_some() {
                if let Some(indent) = self.indentation()? {
                    return Ok(Some(indent));
                }
            }
            let Some(piece) = self.view.next_piece()? else {
                self.terminal = if !self.terminal_layout {
                    Terminal::Eof
                } else if self.emitted && !self.last_was_eol {
                    Terminal::FinishLine
                } else {
                    Terminal::EmptyIndent
                };
                continue;
            };
            let kind = match piece.kind {
                PieceKind::Token(kind) => {
                    self.line_has_code = true;
                    self.suppress_eol = false;
                    kind
                }
                PieceKind::LineComment | PieceKind::BlockComment => {
                    self.suppress_eol = !self.line_has_code;
                    continue;
                }
                PieceKind::Whitespace => continue,
                PieceKind::LineEnding => {
                    self.at_line_start = true;
                    self.line_has_code = false;
                    if self.suppress_eol {
                        self.suppress_eol = false;
                        continue;
                    }
                    // CR remains retained trivia; Rock's EOL points at the LF byte.
                    let span = Span::new(piece.span.file_path, piece.span.end - 1, piece.span.end);
                    return Ok(Some(self.emit(LayoutToken {
                        token: Token { token_type: TokenType::Eol, span },
                        origin: LayoutOrigin::Source(piece.range.end - 1..piece.range.end),
                        context: None,
                    })));
                }
            };
            return Ok(Some(self.emit(LayoutToken {
                token: Token { token_type: kind, span: piece.span },
                origin: LayoutOrigin::Source(piece.range),
                context: piece.context,
            })));
        }
    }
}

#[cfg(test)]
mod tests;
