use std::ops::Range;
use std::sync::Arc;

use super::{
    JoinBoundary, LosslessError, LosslessRun, LosslessStream, ScanLimits, SourcePosition,
    SyntaxContext,
};
use crate::lexer::Span;

impl SourcePosition {
    fn span(&self) -> Span {
        Span::new(self.source.file_path.clone(), self.offset, self.offset)
    }
}

impl LosslessStream {
    pub fn len_bytes(&self) -> usize {
        self.len_bytes
    }

    pub fn start_span(&self) -> Span {
        match self.runs.first() {
            Some(run) => Span::new(run.source.file_path.clone(), run.range.start, run.range.start),
            None => self.anchor.span(),
        }
    }

    pub fn end_span(&self) -> Span {
        match self.runs.last() {
            Some(run) => Span::new(run.source.file_path.clone(), run.range.end, run.range.end),
            None => self.anchor.span(),
        }
    }

    fn checked_range(&self, range: &Range<usize>) -> Result<(), LosslessError> {
        if range.start > range.end || range.end > self.len_bytes {
            return Err(LosslessError::InvalidRange {
                start: range.start,
                end: range.end,
                len: self.len_bytes,
            });
        }
        self.position_at(range.start)?;
        self.position_at(range.end)?;
        Ok(())
    }

    /// Boundaries between segments prefer the right-hand origin, except at EOF.
    fn position_at(&self, offset: usize) -> Result<SourcePosition, LosslessError> {
        let mut base = 0;
        for run in &self.runs {
            let end = base + run.text().len();
            if offset < end || (offset == end && offset == self.len_bytes) {
                let source_offset = run.range.start + (offset - base);
                if !run.source.text.is_char_boundary(source_offset) {
                    return Err(LosslessError::NotCharBoundary { offset });
                }
                return Ok(SourcePosition {
                    source: Arc::clone(&run.source),
                    offset: source_offset,
                    context: run.context,
                });
            }
            base = end;
        }
        // A validated offset in an empty stream is its retained anchor.
        debug_assert_eq!(offset, 0);
        debug_assert!(self.runs.is_empty());
        Ok(self.anchor.clone())
    }

    /// Cuts use logical stream bytes, retaining each source/context segment.
    pub fn slice(&self, range: Range<usize>, limits: ScanLimits) -> Result<Self, LosslessError> {
        self.checked_range(&range)?;
        let len_bytes = range.end - range.start;
        check_bytes(len_bytes, limits)?;
        let anchor = self.position_at(range.start)?;
        let mut runs = Vec::new();
        let mut base = 0;
        for run in &self.runs {
            let end = base + run.text().len();
            let start_in_stream = base.max(range.start);
            let end_in_stream = end.min(range.end);
            if start_in_stream < end_in_stream {
                reserve_run(&mut runs, limits, start_in_stream - range.start)?;
                let mut sliced = run.clone();
                sliced.range = run.range.start + (start_in_stream - base)
                    ..run.range.start + (end_in_stream - base);
                if start_in_stream > base {
                    sliced.boundary = JoinBoundary::SourceAdjacent;
                }
                runs.push(sliced);
            }
            base = end;
            if base >= range.end {
                break;
            }
        }
        Ok(Self { runs, len_bytes, anchor })
    }

    /// Splicing preserves segments; it does not guess gaps or re-lex spelling.
    /// Restored source adjacency must be explicitly requested and proven.
    pub fn join(
        &self,
        right: &Self,
        boundary: JoinBoundary,
        limits: ScanLimits,
    ) -> Result<Self, LosslessError> {
        let len_bytes = self.len_bytes.checked_add(right.len_bytes)
            .ok_or(LosslessError::LengthOverflow)?;
        check_bytes(len_bytes, limits)?;
        if boundary == JoinBoundary::SourceAdjacent {
            if let (Some(left), Some(right)) = (self.runs.last(), right.runs.first()) {
                if !Arc::ptr_eq(&left.source, &right.source)
                    || left.range.end != right.range.start
                    || right.boundary != JoinBoundary::SourceAdjacent
                {
                    return Err(LosslessError::NotSourceAdjacent);
                }
            }
        }
        let count = self.runs.len().checked_add(right.runs.len())
            .ok_or(LosslessError::LengthOverflow)?;
        if count > limits.max_runs {
            return Err(LosslessError::TooManyRuns {
                offset: self.len_bytes,
                limit: limits.max_runs,
            });
        }
        let mut runs = Vec::new();
        runs.try_reserve_exact(count).map_err(|_| LosslessError::AllocationFailed {
            offset: self.len_bytes,
        })?;
        runs.extend(self.runs.iter().cloned());
        for (index, run) in right.runs.iter().enumerate() {
            let mut run = run.clone();
            if index == 0 && !self.runs.is_empty() {
                run.boundary = boundary;
            }
            runs.push(run);
        }
        let anchor = if self.runs.is_empty() && !right.runs.is_empty() {
            right.anchor.clone()
        } else {
            self.anchor.clone()
        };
        Ok(Self { runs, len_bytes, anchor })
    }

    /// A future identifier view must request context agreement across its range;
    /// equal spelling, file names or spans never grant binding authority.
    pub fn binding_context(&self, range: Range<usize>) -> Result<SyntaxContext, LosslessError> {
        self.checked_range(&range)?;
        if range.is_empty() {
            return Err(LosslessError::EmptyBindingRange);
        }
        let first = self.position_at(range.start)?.context;
        let mut base = 0;
        for run in &self.runs {
            let end = base + run.text().len();
            if base < range.end && end > range.start && run.context != first {
                return Err(LosslessError::ConflictingContexts {
                    offset: base.max(range.start),
                    left: first,
                    right: run.context,
                });
            }
            base = end;
            if base >= range.end {
                break;
            }
        }
        Ok(first)
    }
}

fn check_bytes(bytes: usize, limits: ScanLimits) -> Result<(), LosslessError> {
    if bytes > limits.max_source_bytes {
        Err(LosslessError::SourceTooLarge { bytes, limit: limits.max_source_bytes })
    } else {
        Ok(())
    }
}

fn reserve_run(
    runs: &mut Vec<LosslessRun>,
    limits: ScanLimits,
    offset: usize,
) -> Result<(), LosslessError> {
    if runs.len() == limits.max_runs {
        return Err(LosslessError::TooManyRuns { offset, limit: limits.max_runs });
    }
    runs.try_reserve(1).map_err(|_| LosslessError::AllocationFailed { offset })
}
