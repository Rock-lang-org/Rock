//! Capture-capable lossless backing for parser cursors.

use std::cell::RefCell;
use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::lexer::interpretation::layout::{LayoutToken, LayoutView};
use crate::lexer::interpretation::RockError;
use crate::lexer::lossless::{LosslessError, LosslessStream, ScanLimits};
use crate::lexer::{Span, TokenType};

#[derive(Debug, Clone, Copy)]
pub struct SourceLimits {
    /// All cached results, including errors and EOF, across every branch.
    pub max_entries: usize,
    pub max_branches: usize,
}

#[derive(Debug, Clone)]
pub enum SourceError {
    Rock(RockError),
    Preservation(LosslessError),
    UnexpectedEOF(Span),
    UnexpectedIndent(u8, Span),
    LimitExceeded { resource: &'static str, span: Span },
}

#[derive(Clone)]
struct Checkpoint<'a> {
    view: LayoutView<'a>,
    indent_step: Option<usize>,
}

struct Entry<'a> {
    before: Checkpoint<'a>,
    result: Result<Option<LayoutToken>, SourceError>,
}

struct Branch<'a> {
    entries: Vec<Entry<'a>>,
    literals: BTreeMap<usize, (usize, usize)>,
    parent: Option<(usize, usize)>,
    tail: Checkpoint<'a>,
    stopped: bool,
}

struct Cache<'a> {
    branches: Vec<Branch<'a>>,
    entries: usize,
    reads: usize,
}

/// Owns lazy cache storage while cursor/checkpoint handles stay cheap Copy values.
/// A capture forks only the reader state, never mutable speculative results.
pub struct SourceInput<'a> {
    stream: &'a LosslessStream,
    retained: OnceCell<Arc<LosslessStream>>,
    eof: Span,
    limits: SourceLimits,
    cache: RefCell<Cache<'a>>,
}

impl std::fmt::Debug for SourceInput<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceInput").field("eof", &self.eof).finish_non_exhaustive()
    }
}

impl<'a> SourceInput<'a> {
    pub fn new(stream: &'a LosslessStream, terminal_layout: bool, limits: SourceLimits)
        -> Result<Self, SourceError>
    {
        Self::with_layout(stream, terminal_layout, limits, None)
    }

    pub(crate) fn with_layout(stream: &'a LosslessStream, terminal_layout: bool, limits: SourceLimits, relative_indent: Option<usize>)
        -> Result<Self, SourceError>
    {
        if limits.max_branches == 0 {
            return Err(SourceError::LimitExceeded { resource: "branches", span: stream.start_span() });
        }
        let mut branches = Vec::new();
        reserve(&mut branches, 0)?;
        branches.push(Branch {
            entries: Vec::new(),
            literals: BTreeMap::new(), parent: None,
            tail: Checkpoint { view: match relative_indent {
                Some(indent) => LayoutView::new(stream, terminal_layout).with_relative_indent(indent),
                None => LayoutView::new(stream, terminal_layout),
            }, indent_step: None },
            stopped: false,
        });
        Ok(Self {
            stream, retained: OnceCell::new(), eof: stream.end_span(), limits,
            cache: RefCell::new(Cache { branches, entries: 0, reads: 0 }),
        })
    }

    pub fn cursor(&'a self) -> SourceCursor<'a> {
        SourceCursor { input: self, branch: 0, start: 0, end: None }
    }

    /// Debug observers read already requested tokens, never trigger interpretation.
    pub(crate) fn tokens_debug(&'a self) -> impl std::fmt::Debug + 'a {
        struct Tokens<'a, 'b>(&'b SourceInput<'a>);
        impl std::fmt::Debug for Tokens<'_, '_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let cache = self.0.cache.borrow();
                f.debug_list().entries(cache.branches.iter().flat_map(|branch| {
                    branch.entries.iter().filter_map(|entry| match &entry.result {
                        Ok(Some(piece)) => Some(&piece.token),
                        _ => None,
                    })
                })).finish()
            }
        }
        Tokens(self)
    }

    fn peek(&self, branch: usize, index: usize) -> Result<Option<LayoutToken>, SourceError> {
        let mut cache = self.cache.borrow_mut();
        loop {
            let current = &cache.branches[branch];
            if let Some(entry) = current.entries.get(index) {
                return entry.result.clone();
            }
            if current.stopped {
                // A deferred error terminates this interpretation, not the source.
                return current.entries.last().map(|entry| entry.result.clone()).unwrap_or(Ok(None));
            }
            if cache.entries == self.limits.max_entries {
                return Err(SourceError::LimitExceeded { resource: "entries", span: self.eof.clone() });
            }
            let before = current.tail.clone();
            reserve(&mut cache.branches[branch].entries, before.view.offset())?;
            let mut after = before.clone();
            cache.reads += 1;
            let result = after.view.next_token().map_err(SourceError::Rock).and_then(|token| {
                if let Some(token) = &token {
                    if let TokenType::Indent(width) = token.token.token_type {
                        if width % 2 != 0 {
                            return Err(SourceError::UnexpectedIndent(width, token.token.span.clone()));
                        }
                        if width != 0 && after.indent_step.is_none() {
                            after.indent_step = Some(usize::from(width));
                        }
                    }
                }
                Ok(token)
            });
            if let Ok(Some(token)) = &result {
                if matches!(token.token.token_type, TokenType::String(_) | TokenType::Char(_)) {
                    if let crate::lexer::interpretation::layout::LayoutOrigin::Source(range) = &token.origin {
                        let current = &mut cache.branches[branch];
                        current.literals.insert(range.start, (range.end, current.entries.len()));
                    }
                }
            }
            let current = &mut cache.branches[branch];
            current.stopped = !matches!(result, Ok(Some(_)));
            current.tail = if result.is_err() { before.clone() } else { after };
            current.entries.push(Entry { before, result });
            cache.entries += 1;
        }
    }

    fn checkpoint(&self, branch: usize, index: usize) -> Result<Checkpoint<'a>, SourceError> {
        // Materialize only the prefix before this checkpoint, never its next token.
        if index > 0 {
            self.peek(branch, index - 1)?
                .ok_or_else(|| SourceError::UnexpectedEOF(self.eof.clone()))?;
        }
        let cache = self.cache.borrow();
        let current = &cache.branches[branch];
        if let Some(entry) = current.entries.get(index) {
            Ok(entry.before.clone())
        } else if index == current.entries.len() {
            Ok(current.tail.clone())
        } else {
            Err(SourceError::UnexpectedEOF(self.eof.clone()))
        }
    }

    fn fork(&self, checkpoint: Checkpoint<'a>, parent: (usize, usize)) -> Result<usize, SourceError> {
        let mut cache = self.cache.borrow_mut();
        if cache.branches.len() == self.limits.max_branches {
            return Err(SourceError::LimitExceeded { resource: "branches", span: self.eof.clone() });
        }
        reserve(&mut cache.branches, checkpoint.view.offset())?;
        let branch = cache.branches.len();
        cache.branches.push(Branch {
            entries: Vec::new(), literals: BTreeMap::new(), parent: Some(parent), tail: checkpoint, stopped: false,
        });
        Ok(branch)
    }

    #[cfg(test)]
    fn read_count(&self) -> usize {
        self.cache.borrow().reads
    }
}

/// Token indices name cached results, not guessed offsets in printed text.
#[derive(Debug, Clone, Copy)]
pub struct SourceCursor<'a> {
    input: &'a SourceInput<'a>,
    branch: usize,
    start: usize,
    end: Option<usize>,
}

impl SourceCursor<'_> {
    pub(super) fn source_eof(&self) -> Span {
        self.input.eof.clone()
    }

    /// Explicit full inspection, not used by ordinary parser lookahead.
    /// A terminal deferred error is one input event, never an empty suffix.
    pub(super) fn event_count(&self) -> usize {
        let mut count = 0;
        loop {
            match self.peek(count) {
                Ok(Some(_)) => count += 1,
                Ok(None) => return count,
                Err(_) => return count + 1,
            }
        }
    }

    pub fn peek(&self, distance: usize) -> Result<Option<LayoutToken>, SourceError> {
        let Some(index) = self.start.checked_add(distance) else { return Ok(None) };
        if self.end.is_some_and(|end| index >= end) { return Ok(None) }
        self.input.peek(self.branch, index)
    }

    pub fn advance(&self, count: usize) -> Result<Self, SourceError> {
        if count > 0 && self.peek(count - 1)?.is_none() {
            return Err(SourceError::UnexpectedEOF(self.eof_span()?));
        }
        let start = self.start.checked_add(count)
            .ok_or_else(|| SourceError::UnexpectedEOF(self.input.eof.clone()))?;
        Ok(Self { start, ..*self })
    }

    /// Does not inspect the token following the prefix, even for EOF metadata.
    pub fn prefix(&self, count: usize) -> Result<Self, SourceError> {
        let advanced = self.advance(count)?;
        Ok(Self { end: Some(advanced.start), ..*self })
    }

    pub fn offset(&self) -> Result<usize, SourceError> {
        Ok(self.input.checkpoint(self.branch, self.start)?.view.offset())
    }

    pub(crate) fn header_indent(&self) -> Result<usize, SourceError> {
        Ok(self.input.checkpoint(self.branch, self.start)?.view.header_indent())
    }

    pub fn slice_until(&self, end: Self, limits: ScanLimits) -> Result<LosslessStream, SourceError> {
        // Capturing nested blocks forks interpretation, not the retained source.
        if !std::ptr::eq(self.input, end.input) {
            return Err(SourceError::Rock(RockError::InvalidCaptureBoundary { span: self.input.eof.clone() }));
        }
        self.input.stream.slice(self.offset()?..end.offset()?, limits).map_err(SourceError::Preservation)
    }

    pub(crate) fn declaration_until(&self, end: Self, header_indent: usize)
        -> Result<crate::ast::DeclarationInput, SourceError>
    {
        if !std::ptr::eq(self.input, end.input) {
            return Err(SourceError::Rock(RockError::InvalidCaptureBoundary { span: self.input.eof.clone() }));
        }
        let range = self.offset()?..end.offset()?;
        if range.start > range.end || range.end > self.input.stream.len_bytes() {
            return Err(SourceError::Rock(RockError::InvalidCaptureBoundary { span: self.input.eof.clone() }));
        }
        let source = self.input.retained.get_or_init(|| Arc::new(self.input.stream.clone()));
        Ok(crate::ast::DeclarationInput::new(Arc::clone(source), range, header_indent))
    }

    /// Inspect cached Rock interpretations only, including branches resumed after
    /// nested captures. Never request tokens in an opaque captured body.
    pub(crate) fn literal_ranges_until(&self, end: Self) -> Result<Vec<std::ops::Range<usize>>, SourceError> {
        if !std::ptr::eq(self.input, end.input) {
            return Err(SourceError::Rock(RockError::InvalidCaptureBoundary { span: self.input.eof.clone() }));
        }
        let start = self.offset()?;
        let end_offset = end.offset()?;
        if end_offset < start {
            return Err(SourceError::Rock(RockError::InvalidCaptureBoundary { span: self.input.eof.clone() }));
        }
        let cache = self.input.cache.borrow();
        let mut branch = Some((end.branch, end.start));
        let mut ranges = Vec::new();
        while let Some((index, prefix)) = branch {
            let current = &cache.branches[index];
            ranges.extend(current.literals.range(start..end_offset)
                .filter(|(_, (literal_end, token))| *literal_end <= end_offset && *token < prefix)
                .map(|(literal_start, (literal_end, _))| literal_start - start..literal_end - start));
            branch = current.parent;
        }
        ranges.sort_unstable_by_key(|range| (range.start, range.end));
        Ok(ranges)
    }

    pub fn eof_span(&self) -> Result<Span, SourceError> {
        match self.end {
            Some(end) => {
                let offset = self.input.checkpoint(self.branch, end)?.view.offset();
                self.input.stream.slice(offset..offset, ScanLimits { max_source_bytes: 0, max_runs: 0 })
                    .map(|stream| stream.start_span()).map_err(SourceError::Preservation)
            }
            None => Ok(self.input.eof.clone()),
        }
    }

    /// Discover layout at this position only; speculative later results cannot
    /// select a step or make an earlier checkpoint invalid.
    pub fn indent_step(&self) -> Result<usize, SourceError> {
        if let Some(step) = self.input.checkpoint(self.branch, self.start)?.indent_step {
            return Ok(step);
        }
        let mut cursor = *self;
        while let Some(token) = cursor.peek(0)? {
            match token.token.token_type {
                TokenType::Indent(width) if width > 0 => return Ok(usize::from(width)),
                TokenType::Indent(0) | TokenType::Eol => cursor = cursor.advance(1)?,
                _ => break,
            }
        }
        Ok(4)
    }

    pub fn same_remaining(&self, other: &Self) -> bool {
        std::ptr::eq(self.input, other.input) && self.branch == other.branch
            && self.start == other.start && self.end == other.end
    }

    pub fn capture_to(&self, end: usize, limits: ScanLimits)
        -> Result<(Self, LosslessStream), SourceError>
    {
        let mut checkpoint = self.input.checkpoint(self.branch, self.start)?;
        // A bounded branch needs its own physically bounded view, not permission
        // to interpret past the token prefix after skipping any of its contents.
        if self.end.is_some() {
            return Err(SourceError::Rock(RockError::InvalidCaptureBoundary { span: self.eof_span()? }));
        }
        let captured = checkpoint.view.capture_to(end, limits).map_err(SourceError::Rock)?;
        let branch = self.input.fork(checkpoint, (self.branch, self.start))?;
        Ok((Self { branch, start: 0, end: None, ..*self }, captured))
    }

    pub fn capture_indented_block(&self, limits: ScanLimits)
        -> Result<Option<(Self, crate::lexer::interpretation::layout::CapturedBlock)>, SourceError>
    {
        if self.end.is_some() { return Ok(None); }
        let mut checkpoint = self.input.checkpoint(self.branch, self.start)?;
        let Some(captured) = checkpoint.view.capture_indented_block(limits).map_err(SourceError::Rock)? else {
            return Ok(None);
        };
        let branch = self.input.fork(checkpoint, (self.branch, self.start))?;
        Ok(Some((Self { branch, start: 0, end: None, ..*self }, captured)))
    }
}

fn reserve<T>(entries: &mut Vec<T>, offset: usize) -> Result<(), SourceError> {
    entries.try_reserve(1).map_err(|_| SourceError::Preservation(LosslessError::AllocationFailed { offset }))
}

#[cfg(test)]
mod tests;
