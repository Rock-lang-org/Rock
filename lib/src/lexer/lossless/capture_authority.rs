//! Opaque captures are forwarded atoms, not worker-editable grouping hints.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use super::{fragments::ExpressionRange, JoinBoundary, LosslessRun, LosslessStream, RunKind, SyntaxContext};
use crate::macro_expansion::TokenOrigin;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Authority {
    source: u64,
    context: SyntaxContext,
    kind: RunKind,
    origin: Option<TokenOrigin>,
}

impl From<&LosslessRun> for Authority {
    fn from(run: &LosslessRun) -> Self {
        Self { source: run.source.identity, context: run.context, kind: run.kind, origin: run.origin.clone() }
    }
}

#[derive(PartialEq, Eq, Hash)]
struct Piece {
    authority: Authority,
    range: Range<usize>,
    boundary: JoinBoundary,
}

#[derive(PartialEq, Eq, Hash)]
struct Capture {
    pieces: Vec<Piece>,
    expressions: Vec<ExpressionRange>,
}

struct Indexed<'a> {
    stream: &'a LosslessStream,
    starts: Vec<usize>,
    roots: Vec<Range<usize>>,
}

impl<'a> Indexed<'a> {
    fn new(stream: &'a LosslessStream) -> Self {
        let mut offset = 0;
        let starts = stream.runs.iter().map(|run| {
            let start = offset;
            offset += run.range.len();
            start
        }).collect();
        let mut roots = Vec::new();
        let mut index = 0;
        while index < stream.expressions.len() {
            let start = index;
            let end = stream.expressions[index].range.end;
            index += 1;
            while index < stream.expressions.len() && stream.expressions[index].range.start < end { index += 1; }
            roots.push(start..index);
        }
        Self { stream, starts, roots }
    }

    fn pieces(&self, range: Range<usize>, mut visit: impl FnMut(&LosslessRun, Range<usize>) -> Result<(), String>) -> Result<(), String> {
        if range.is_empty() { return Ok(()); }
        let first = self.starts.partition_point(|start| *start <= range.start).saturating_sub(1);
        for (run, start) in self.stream.runs[first..].iter().zip(&self.starts[first..]) {
            if *start >= range.end { break; }
            let left = range.start.max(*start) - start;
            let right = range.end.min(start + run.range.len()) - start;
            visit(run, run.range.start + left..run.range.start + right)?;
        }
        Ok(())
    }

    fn capture(&self, indices: Range<usize>) -> Result<Capture, String> {
        let outer = &self.stream.expressions[indices.start].range;
        let mut pieces: Vec<Piece> = Vec::new();
        self.pieces(outer.clone(), |run, range| {
            if run.construction.is_some() { return Err("proc output invented an expression capture".into()); }
            let authority = Authority::from(run);
            // Segment subdivision is immaterial; internal splice boundaries are not.
            if let Some(previous) = pieces.last_mut() {
                if run.boundary == JoinBoundary::SourceAdjacent && previous.authority == authority && previous.range.end == range.start {
                    previous.range.end = range.end;
                    return Ok(());
                }
            }
            let boundary = if pieces.is_empty() { JoinBoundary::Separate } else { run.boundary };
            pieces.push(Piece { authority, range, boundary });
            Ok(())
        })?;
        let expressions = self.stream.expressions[indices].iter().map(|fragment| ExpressionRange {
            range: fragment.range.start - outer.start..fragment.range.end - outer.start,
            header_indent: fragment.header_indent,
        }).collect();
        Ok(Capture { pieces, expressions })
    }

    fn ordinary(&self, mut visit: impl FnMut(&LosslessRun, Range<usize>) -> Result<(), String>) -> Result<(), String> {
        let mut start = 0;
        for indices in &self.roots {
            let range = &self.stream.expressions[indices.start].range;
            self.pieces(start..range.start, &mut visit)?;
            start = range.end;
        }
        self.pieces(start..self.stream.len_bytes, visit)
    }
}

impl LosslessStream {
    pub(super) fn validate_forwarded_captures(&self, input: &Self) -> Result<(), String> {
        if input.expressions.is_empty() && self.expressions.is_empty() { return Ok(()); }
        let input = Indexed::new(input);
        let output = Indexed::new(self);
        let captures = input.roots.iter().map(|indices| input.capture(indices.clone())).collect::<Result<HashSet<_>, _>>()?;
        for indices in &output.roots {
            if !captures.contains(&output.capture(indices.clone())?) {
                return Err("proc output changed or invented an opaque expression capture".into());
            }
        }
        let mut ordinary = HashMap::<Authority, Vec<Range<usize>>>::new();
        input.ordinary(|run, range| {
            ordinary.entry(Authority::from(run)).or_default().push(range);
            Ok(())
        })?;
        for ranges in ordinary.values_mut() {
            ranges.sort_unstable_by_key(|range| range.start);
            let mut end = 0;
            for range in ranges { end = end.max(range.end); range.end = end; }
        }
        output.ordinary(|run, range| {
            if run.construction.is_some() { return Ok(()); }
            if let Some(ranges) = ordinary.get(&Authority::from(run)) {
                let end = ranges.partition_point(|allowed| allowed.start <= range.start);
                if end > 0 && ranges[end - 1].end >= range.end { return Ok(()); }
            }
            Err("proc output removed or split an opaque expression capture".into())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macro_expansion::{ExpansionId, GeneratedSourceId};
    use crate::macro_expansion::token_tree::{source, MACRO_LIMITS};

    #[test]
    fn macro_review_forwarded_capture_metadata_is_authorized_not_just_well_formed() {
        let raw = source("1 + 2");
        let capture = raw.clone().expression_fragment(4, MACRO_LIMITS).unwrap();
        let input = source("prefix ").join(&capture, JoinBoundary::Separate, MACRO_LIMITS).unwrap()
            .join(&source(" suffix"), JoinBoundary::Separate, MACRO_LIMITS).unwrap();
        let invocation = crate::ast::InvocationId::fresh().unwrap();
        let resolve = |stream: &LosslessStream| stream.resolve_proc_output(&input, invocation, ExpansionId(1), GeneratedSourceId(1), |_| unreachable!());
        assert!(resolve(&input).is_ok());
        assert!(resolve(&capture).is_ok());
        assert!(resolve(&capture.join(&capture, JoinBoundary::Separate, MACRO_LIMITS).unwrap()).is_ok());
        assert!(resolve(&input.slice(0..7, MACRO_LIMITS).unwrap()).is_ok());
        assert!(resolve(&raw).is_err(), "removing metadata must not flatten a capture");
        assert!(resolve(&capture.slice(0..1, MACRO_LIMITS).unwrap()).is_err(), "partial capture");
        let mut changed = capture.clone();
        changed.expressions[0].header_indent += 1;
        assert!(resolve(&changed).is_err(), "changed layout");
        changed.expressions[0].range = 0..1;
        assert!(resolve(&changed).is_err(), "changed capture extent");
        let invented = input.clone().expression_fragment(0, MACRO_LIMITS).unwrap();
        assert!(resolve(&invented).is_err(), "invented outer capture");
    }

    #[test]
    fn macro_review_nested_capture_metadata_cannot_be_removed_inside_a_valid_outer() {
        let inner = source("1 + 2").expression_fragment(0, MACRO_LIMITS).unwrap();
        let outer = source("3 * ").join(&inner, JoinBoundary::Separate, MACRO_LIMITS).unwrap()
            .expression_fragment(0, MACRO_LIMITS).unwrap();
        assert!(outer.validate_forwarded_captures(&outer).is_ok());
        let mut changed = outer.clone();
        changed.expressions.pop();
        assert!(changed.validate_forwarded_captures(&outer).is_err());
    }
}
