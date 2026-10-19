use crate::lexer::Token;

/// The sole owner of parser token-slice access; grammar code uses ParseCtx.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TokenCursor<'a> {
    tokens: &'a [Token],
    start: usize,
    end: usize,
}

impl<'a> TokenCursor<'a> {
    pub(super) fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, start: 0, end: tokens.len() }
    }

    pub(super) fn len(&self) -> usize {
        self.end - self.start
    }

    pub(super) fn peek(&self, index: usize) -> Option<&'a Token> {
        let index = self.start.checked_add(index)?;
        if index < self.end { self.tokens.get(index) } else { None }
    }

    pub(super) fn advance(&self, count: usize) -> Option<Self> {
        if count <= self.len() {
            Some(Self { start: self.start + count, ..*self })
        } else {
            None
        }
    }

    pub(super) fn prefix(&self, count: usize) -> Option<Self> {
        if count <= self.len() {
            Some(Self { end: self.start + count, ..*self })
        } else {
            None
        }
    }

    pub(super) fn same_remaining(&self, other: &Self) -> bool {
        std::ptr::eq(self.tokens, other.tokens) && self.start == other.start && self.end == other.end
    }
}
