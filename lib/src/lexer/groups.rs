//! Optional Rock group views; neutral lossless input never requires balancing.

use std::ops::Range;

use super::interpretation::{PieceKind, RockError, RockView};
use super::lossless::{JoinBoundary, LosslessError, LosslessStream, ScanLimits};
use super::{Span, TokenType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delimiter {
    Paren,
    Bracket,
    Brace,
}

impl Delimiter {
    pub fn open_token(self) -> TokenType {
        match self {
            Self::Paren => TokenType::OpenParen,
            Self::Bracket => TokenType::OpenBracket,
            Self::Brace => TokenType::OpenBrace,
        }
    }

    pub fn close_token(self) -> TokenType {
        match self {
            Self::Paren => TokenType::CloseParen,
            Self::Bracket => TokenType::CloseBracket,
            Self::Brace => TokenType::CloseBrace,
        }
    }

    fn character(self, opening: bool) -> u8 {
        match (self, opening) {
            (Self::Paren, true) => b'(', (Self::Paren, false) => b')',
            (Self::Bracket, true) => b'[', (Self::Bracket, false) => b']',
            (Self::Brace, true) => b'{', (Self::Brace, false) => b'}',
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GroupLimits {
    pub max_depth: usize,
    pub max_groups: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupLimit {
    Depth,
    Count,
}

#[derive(Debug, Clone)]
pub enum GroupError {
    Rock(RockError),
    Preservation(LosslessError),
    UnexpectedClose { delimiter: Delimiter, span: Span },
    MismatchedClose { expected: Delimiter, actual: Delimiter, open: Span, close: Span },
    Unclosed { delimiter: Delimiter, open: Span },
    LimitExceeded { limit: GroupLimit, span: Span },
    InvalidWrapper { delimiter: Delimiter, opening: bool, span: Span },
}

/// Flat preorder records avoid recursive parsing, traversal and destruction.
#[derive(Debug, Clone)]
pub struct GroupRange {
    pub delimiter: Delimiter,
    pub open: Range<usize>,
    pub close: Range<usize>,
    pub parent: Option<usize>,
}

struct OpenGroup {
    index: usize,
    span: Span,
}

#[derive(Debug)]
pub struct BalancedGroups<'a> {
    stream: &'a LosslessStream,
    records: Vec<GroupRange>,
}

impl<'a> BalancedGroups<'a> {
    /// Explicitly requests a Rock interpretation, unlike source preservation.
    pub fn parse(stream: &'a LosslessStream, limits: GroupLimits) -> Result<Self, GroupError> {
        let mut view = RockView::new(stream);
        let mut records = Vec::<GroupRange>::new();
        let mut stack = Vec::<OpenGroup>::new();
        while let Some(piece) = view.next_piece().map_err(GroupError::Rock)? {
            let PieceKind::Token(token) = piece.kind else { continue };
            let (delimiter, opening, offset) = match token {
                TokenType::OpenParen => (Delimiter::Paren, true, 0),
                TokenType::OpenBracket => (Delimiter::Bracket, true, 0),
                TokenType::OpenBrace => (Delimiter::Brace, true, 0),
                TokenType::CloseParen | TokenType::MacroRepeatClose => (Delimiter::Paren, false, 0),
                TokenType::CloseBracket => (Delimiter::Bracket, false, 0),
                TokenType::CloseBrace => (Delimiter::Brace, false, 0),
                TokenType::MacroRepeatOpen => (Delimiter::Paren, true, 1),
                _ => continue,
            };
            let range = piece.range.start + offset..piece.range.start + offset + 1;
            let span = Span::new(piece.span.file_path, piece.span.start + offset, piece.span.start + offset + 1);
            if opening {
                if stack.len() == limits.max_depth {
                    return Err(GroupError::LimitExceeded { limit: GroupLimit::Depth, span });
                }
                if records.len() == limits.max_groups {
                    return Err(GroupError::LimitExceeded { limit: GroupLimit::Count, span });
                }
                reserve(&mut records, range.start)?;
                reserve(&mut stack, range.start)?;
                records.push(GroupRange {
                    delimiter,
                    close: range.end..range.end,
                    open: range,
                    parent: stack.last().map(|frame| frame.index),
                });
                stack.push(OpenGroup { index: records.len() - 1, span });
            } else {
                let Some(frame) = stack.pop() else {
                    return Err(GroupError::UnexpectedClose { delimiter, span });
                };
                let record = &mut records[frame.index];
                if record.delimiter != delimiter {
                    return Err(GroupError::MismatchedClose {
                        expected: record.delimiter, actual: delimiter, open: frame.span, close: span,
                    });
                }
                record.close = range;
            }
        }
        if let Some(frame) = stack.last() {
            return Err(GroupError::Unclosed {
                delimiter: records[frame.index].delimiter,
                open: frame.span.clone(),
            });
        }
        Ok(Self { stream, records })
    }

    pub fn records(&self) -> &[GroupRange] {
        &self.records
    }

    pub fn group(&self, index: usize) -> Option<GroupView<'_>> {
        Some(GroupView { stream: self.stream, record: self.records.get(index)? })
    }
}

#[derive(Debug)]
pub struct GroupView<'a> {
    stream: &'a LosslessStream,
    record: &'a GroupRange,
}

impl GroupView<'_> {
    pub fn delimiter(&self) -> Delimiter {
        self.record.delimiter
    }

    pub fn whole(&self, limits: ScanLimits) -> Result<LosslessStream, LosslessError> {
        self.stream.slice(self.record.open.start..self.record.close.end, limits)
    }

    pub fn open(&self, limits: ScanLimits) -> Result<LosslessStream, LosslessError> {
        self.stream.slice(self.record.open.clone(), limits)
    }

    pub fn inner(&self, limits: ScanLimits) -> Result<LosslessStream, LosslessError> {
        self.stream.slice(self.record.open.end..self.record.close.start, limits)
    }

    pub fn close(&self, limits: ScanLimits) -> Result<LosslessStream, LosslessError> {
        self.stream.slice(self.record.close.clone(), limits)
    }
}

/// Known wrapper structure, independent of whether its payload is valid Rock.
#[derive(Debug)]
pub struct StructuralGroup {
    stream: LosslessStream,
    record: GroupRange,
}

impl StructuralGroup {
    pub fn from_parts(
        delimiter: Delimiter,
        open: &LosslessStream,
        inner: &LosslessStream,
        close: &LosslessStream,
        limits: ScanLimits,
    ) -> Result<Self, GroupError> {
        for (stream, opening) in [(open, true), (close, false)] {
            if stream.len_bytes() != 1 || stream.runs().first().map(|run| run.text().as_bytes()[0])
                != Some(delimiter.character(opening))
            {
                return Err(GroupError::InvalidWrapper { delimiter, opening, span: stream.start_span() });
            }
        }
        let stream = open.join(inner, JoinBoundary::Separate, limits)
            .and_then(|stream| stream.join(close, JoinBoundary::Separate, limits))
            .map_err(GroupError::Preservation)?;
        let record = GroupRange {
            delimiter, open: 0..1, close: stream.len_bytes() - 1..stream.len_bytes(), parent: None,
        };
        Ok(Self { stream, record })
    }

    /// Retained neutral leaves; keep the GroupView when forwarding wrapper structure.
    pub fn stream(&self) -> &LosslessStream {
        &self.stream
    }

    pub fn view(&self) -> GroupView<'_> {
        GroupView { stream: &self.stream, record: &self.record }
    }
}

fn reserve<T>(entries: &mut Vec<T>, offset: usize) -> Result<(), GroupError> {
    entries.try_reserve(1).map_err(|_| GroupError::Preservation(LosslessError::AllocationFailed { offset }))
}

#[cfg(test)]
mod tests;
