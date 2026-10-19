use super::source_cursor::{SourceCursor, SourceError};
use super::ParseError;
use crate::lexer::{Span, Token, TokenType};

/// The sole owner of parser backing access; grammar code uses ParseCtx.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TokenCursor<'a> {
    Tokens { tokens: &'a [Token], start: usize, end: usize },
    Source(SourceCursor<'a>),
}

pub(super) struct TokenMetadata<'a> {
    pub eof_location: Option<(&'a std::path::PathBuf, usize)>,
    pub indent_step: usize,
    pub invalid_indent: Option<(u8, &'a Span)>,
}

impl<'a> TokenCursor<'a> {
    pub(super) fn new(tokens: &'a [Token]) -> Self {
        Self::Tokens { tokens, start: 0, end: tokens.len() }
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Tokens { start, end, .. } => end - start,
            Self::Source(cursor) => cursor.event_count(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        match self {
            Self::Tokens { start, end, .. } => start == end,
            Self::Source(cursor) => matches!(cursor.peek(0), Ok(None)),
        }
    }

    pub(super) fn peek(&self, index: usize) -> Result<Option<Token>, ParseError> {
        match self {
            Self::Tokens { tokens, start, end } => {
                Ok(start.checked_add(index).filter(|index| index < end)
                    .and_then(|index| tokens.get(index)).cloned())
            }
            Self::Source(cursor) => cursor.peek(index).map(|piece| piece.map(|piece| piece.token))
                .map_err(|error| source_error(error, cursor.source_eof())),
        }
    }

    pub(super) fn advance(&self, count: usize) -> Result<Option<Self>, ParseError> {
        match self {
            Self::Tokens { tokens, start, end } => Ok((count <= end - start)
                .then(|| Self::Tokens { tokens, start: start + count, end: *end })),
            Self::Source(cursor) => cursor.advance(count).map(|cursor| Some(Self::Source(cursor)))
                .map_err(|error| source_error(error, cursor.source_eof())),
        }
    }

    pub(super) fn prefix(&self, count: usize) -> Result<Option<Self>, ParseError> {
        match self {
            Self::Tokens { tokens, start, end } => Ok((count <= end - start)
                .then(|| Self::Tokens { tokens, start: *start, end: start + count })),
            Self::Source(cursor) => cursor.prefix(count).map(|cursor| Some(Self::Source(cursor)))
                .map_err(|error| source_error(error, cursor.source_eof())),
        }
    }

    pub(super) fn same_remaining(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Tokens { tokens, start, end }, Self::Tokens { tokens: rhs, start: right, end: bound }) =>
                std::ptr::eq(*tokens, *rhs) && start == right && end == bound,
            (Self::Source(left), Self::Source(right)) => left.same_remaining(right),
            _ => false,
        }
    }

    pub(super) fn token_metadata(&self) -> TokenMetadata<'a> {
        let Self::Tokens { tokens, start, end } = self else {
            return TokenMetadata { eof_location: None, indent_step: 4, invalid_indent: None };
        };
        let tokens = &tokens[*start..*end];
        let mut step = 0;
        let mut invalid = None;
        for token in tokens {
            if let TokenType::Indent(width) = token.token_type {
                if width % 2 != 0 { invalid = Some((width, &token.span)); break }
                if width > 0 && step == 0 { step = usize::from(width) }
            }
        }
        TokenMetadata {
            eof_location: tokens.last().map(|token| (&token.span.file_path, token.span.end)),
            indent_step: if step == 0 { 4 } else { step },
            invalid_indent: invalid,
        }
    }

    pub(super) fn prefix_eof_location(&self, count: usize) -> Option<(&'a std::path::PathBuf, usize)> {
        let Self::Tokens { tokens, start, end } = self else { return None };
        if count < end - start {
            let span = &tokens[start + count].span;
            Some((&span.file_path, span.start))
        } else {
            None
        }
    }
}

pub(super) fn source_error(error: SourceError, fallback: Span) -> ParseError {
    use crate::lexer::interpretation::RockError;

    match error {
        SourceError::UnexpectedEOF(span) => ParseError::UnexpectedEOF(span),
        SourceError::UnexpectedIndent(width, span) => ParseError::UnexpectedIndent(width, span),
        SourceError::LimitExceeded { resource, span } => ParseError::HardError(format!("parser source {resource} limit exceeded"), span),
        SourceError::Preservation(error) => ParseError::HardError(format!("source preservation failed: {error:?}"), fallback),
        SourceError::Rock(error) => match error {
            RockError::Lexer(error) => ParseError::Lexer(error),
            RockError::UnterminatedBlockComment(span) => ParseError::HardError("unterminated Rock block comment".into(), span),
            RockError::IndentationTooWide { width, span } => ParseError::HardError(format!("indentation width {width} exceeds 255 bytes"), span),
            RockError::InvalidCaptureBoundary { span } => ParseError::HardError("capture must use physical line boundaries".into(), span),
            RockError::Preservation(error) => ParseError::HardError(format!("source preservation failed: {error:?}"), fallback),
        },
    }
}
