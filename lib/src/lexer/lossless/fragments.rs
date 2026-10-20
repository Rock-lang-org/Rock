//! Optional expression boundaries over the same retained bytes, never printed
//! substitutions or a stored compiler-token/AST payload.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use super::{LosslessError, LosslessStream, ScanLimits};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ExpressionRange {
    pub range: Range<usize>,
    pub header_indent: usize,
}

/// An interpreted atom. Its contents still undergo ordinary bounded parsing
/// when requested as Rock, including validation of any nested fragment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpressionFragment {
    pub input: LosslessStream,
    pub header_indent: usize,
}

impl LosslessStream {
    pub(crate) fn expression_fragment(mut self, header_indent: usize, limits: ScanLimits) -> Result<Self, LosslessError> {
        if self.len_bytes > limits.max_source_bytes || self.runs.len() > limits.max_runs {
            return Err(LosslessError::InvalidExpressionFragment);
        }
        if self.len_bytes == 0 { return Err(LosslessError::InvalidExpressionFragment); }
        if self.expressions.first().is_some_and(|fragment| fragment.range == (0..self.len_bytes)) {
            return Ok(self);
        }
        if self.expressions.len() >= limits.max_runs { return Err(LosslessError::InvalidExpressionFragment); }
        self.expressions.try_reserve(1).map_err(|_| LosslessError::AllocationFailed { offset: 0 })?;
        self.expressions.insert(0, ExpressionRange { range: 0..self.len_bytes, header_indent });
        self.validate_expressions()?;
        Ok(self)
    }

    pub(crate) fn expression_end_at(&self, offset: usize) -> Option<usize> {
        self.expressions.iter().find(|fragment| fragment.range.start == offset).map(|fragment| fragment.range.end)
    }

    pub(crate) fn expression_start_containing(&self, offset: usize) -> Option<usize> {
        self.expressions.iter().find(|fragment| fragment.range.contains(&offset)).map(|fragment| fragment.range.start)
    }

    pub(crate) fn expression_at(&self, offset: usize, limits: ScanLimits)
        -> Result<Option<(usize, ExpressionFragment)>, LosslessError>
    {
        let Some(fragment) = self.expressions.iter().find(|fragment| fragment.range.start == offset) else { return Ok(None); };
        let mut input = self.slice(fragment.range.clone(), limits)?;
        // Strict containment makes recursive interpretation decrease. Equal or
        // crossing ranges are rejected by transport validation.
        input.expressions.remove(0);
        Ok(Some((fragment.range.end, ExpressionFragment { input, header_indent: fragment.header_indent })))
    }

    pub(super) fn validate_expressions(&self) -> Result<(), LosslessError> {
        let mut stack: Vec<&Range<usize>> = Vec::new();
        let mut previous = None;
        for fragment in &self.expressions {
            let range = &fragment.range;
            self.checked_range(range)?;
            if range.is_empty() || fragment.header_indent > 16 * 1024 * 1024 || previous.is_some_and(|start| range.start < start) {
                return Err(LosslessError::InvalidExpressionFragment);
            }
            while stack.last().is_some_and(|outer| outer.end <= range.start) { stack.pop(); }
            if let Some(outer) = stack.last() {
                if range.end > outer.end || *range == **outer { return Err(LosslessError::InvalidExpressionFragment); }
            }
            if stack.len() >= 128 { return Err(LosslessError::InvalidExpressionFragment); }
            stack.try_reserve(1).map_err(|_| LosslessError::AllocationFailed { offset: range.start })?;
            stack.push(range);
            previous = Some(range.start);
        }
        Ok(())
    }
}
