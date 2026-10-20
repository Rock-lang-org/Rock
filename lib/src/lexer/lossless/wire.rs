//! Checked snapshot-table transport: aliases are explicit, never inferred from
//! file paths, equal text, or spans. Deserialization cannot construct invalid runs.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{JoinBoundary, LosslessRun, LosslessStream, RunKind, SourcePosition, SourceSnapshot, SyntaxContext};

const MAGIC: [u8; 8] = *b"RKLOSS03";
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_RUNS: usize = 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct WireRun {
    source: usize,
    range: Range<usize>,
    kind: RunKind,
    context: u32,
    boundary: JoinBoundary,
    origin: Option<crate::macro_expansion::TokenOrigin>,
    construction: Option<super::ConstructionContext>,
}

#[derive(Serialize, Deserialize)]
struct WireAnchor {
    source: usize,
    offset: usize,
    context: u32,
    origin: Option<crate::macro_expansion::TokenOrigin>,
}

#[derive(Serialize)]
struct WireRef<'a> {
    magic: [u8; 8],
    sources: Vec<&'a SourceSnapshot>,
    runs: Vec<WireRun>,
    anchor: WireAnchor,
    expressions: &'a [super::fragments::ExpressionRange],
}

#[derive(Serialize, Deserialize)]
struct WireOwned {
    magic: [u8; 8],
    sources: Vec<SourceSnapshot>,
    runs: Vec<WireRun>,
    anchor: WireAnchor,
    expressions: Vec<super::fragments::ExpressionRange>,
}

impl Serialize for LosslessStream {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sources = Vec::new();
        let mut source_ids = HashMap::new();
        let mut runs = Vec::new();
        sources.try_reserve(1).map_err(serde::ser::Error::custom)?;
        source_ids.try_reserve(1).map_err(serde::ser::Error::custom)?;
        sources.push(self.anchor.source.as_ref());
        source_ids.insert(Arc::as_ptr(&self.anchor.source), 0);
        runs.try_reserve_exact(self.runs.len()).map_err(serde::ser::Error::custom)?;
        for run in &self.runs {
            let key = Arc::as_ptr(&run.source);
            let source = match source_ids.get(&key) {
                Some(id) => *id,
                None => {
                    sources.try_reserve(1).map_err(serde::ser::Error::custom)?;
                    source_ids.try_reserve(1).map_err(serde::ser::Error::custom)?;
                    let id = sources.len();
                    sources.push(run.source.as_ref());
                    source_ids.insert(key, id);
                    id
                }
            };
            runs.push(WireRun { source, range: run.range.clone(), kind: run.kind, context: run.context.0, boundary: run.boundary, origin: run.origin.clone(), construction: run.construction });
        }
        WireRef { magic: MAGIC, sources, runs, anchor: WireAnchor { source: 0, offset: self.anchor.offset, context: self.anchor.context.0, origin: self.anchor.origin.clone() }, expressions: &self.expressions }
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LosslessStream {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = WireOwned::deserialize(deserializer)?;
        if wire.magic != MAGIC { return Err(serde::de::Error::custom("unsupported lossless stream wire format")) }
        if wire.sources.is_empty() || wire.sources.len() > MAX_RUNS || wire.runs.len() > MAX_RUNS || wire.expressions.len() > MAX_RUNS {
            return Err(serde::de::Error::custom("lossless source/run limit exceeded"));
        }
        let mut bytes = 0usize;
        let mut sources = Vec::new();
        sources.try_reserve_exact(wire.sources.len()).map_err(serde::de::Error::custom)?;
        for source in wire.sources {
            bytes = bytes.checked_add(source.text.len()).ok_or_else(|| serde::de::Error::custom("source length overflow"))?;
            if bytes > MAX_BYTES || source.file_path.as_os_str().is_empty() {
                return Err(serde::de::Error::custom("invalid lossless source table"));
            }
            sources.push(Arc::new(source));
        }
        let source = sources.get(wire.anchor.source).ok_or_else(|| serde::de::Error::custom("invalid anchor source"))?;
        if wire.anchor.offset > source.text.len() || !source.text.is_char_boundary(wire.anchor.offset) {
            return Err(serde::de::Error::custom("invalid lossless anchor"));
        }
        let anchor = SourcePosition { source: Arc::clone(source), offset: wire.anchor.offset, context: SyntaxContext(wire.anchor.context), origin: wire.anchor.origin };
        let mut runs = Vec::<LosslessRun>::new();
        runs.try_reserve_exact(wire.runs.len()).map_err(serde::de::Error::custom)?;
        let mut len_bytes = 0usize;
        for run in wire.runs {
            let source = sources.get(run.source).ok_or_else(|| serde::de::Error::custom("invalid run source"))?;
            let Some(text) = source.text.get(run.range.clone()) else { return Err(serde::de::Error::custom("invalid lossless range")) };
            if text.is_empty() || !text.chars().all(|ch| RunKind::of(ch) == run.kind)
                || (run.kind == RunKind::LineEnding && !matches!(text, "\r" | "\n" | "\r\n"))
            {
                return Err(serde::de::Error::custom("invalid lossless run"));
            }
            if run.boundary == JoinBoundary::SourceAdjacent {
                if let Some(previous) = runs.last() {
                    if !Arc::ptr_eq(&previous.source, source) || previous.range.end != run.range.start {
                        return Err(serde::de::Error::custom("forged source adjacency"));
                    }
                }
            }
            len_bytes = len_bytes.checked_add(text.len()).ok_or_else(|| serde::de::Error::custom("stream length overflow"))?;
            if len_bytes > MAX_BYTES { return Err(serde::de::Error::custom("lossless stream byte limit exceeded")) }
            runs.push(LosslessRun { source: Arc::clone(source), range: run.range, kind: run.kind, context: SyntaxContext(run.context), boundary: run.boundary, origin: run.origin, construction: run.construction });
        }
        let stream = Self { runs, len_bytes, anchor, expressions: wire.expressions };
        stream.validate_expressions().map_err(|error| serde::de::Error::custom(format!("invalid expression fragments: {error:?}")))?;
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> WireOwned {
        WireOwned {
            magic: MAGIC,
            sources: vec![SourceSnapshot::new("/wire.rk".into(), "é a".into()).unwrap()],
            runs: vec![WireRun { source: 0, range: 0..2, kind: RunKind::Text, context: 0, boundary: JoinBoundary::SourceAdjacent, origin: None, construction: None }],
            anchor: WireAnchor { source: 0, offset: 0, context: 0, origin: None },
            expressions: Vec::new(),
        }
    }

    fn rejects(wire: WireOwned) {
        let bytes = bincode::serialize(&wire).unwrap();
        assert!(bincode::deserialize::<LosslessStream>(&bytes).is_err());
    }

    #[test]
    fn macro_a_01_wire_rejects_invalid_ranges_sources_and_kinds() {
        let mut wire = valid(); wire.runs[0].range = 1..2; rejects(wire);
        let mut wire = valid(); wire.runs[0].range = 0..100; rejects(wire);
        let mut wire = valid(); wire.runs[0].range = 0..0; rejects(wire);
        let mut wire = valid(); wire.runs[0].source = 9; rejects(wire);
        let mut wire = valid(); wire.runs[0].kind = RunKind::Whitespace; rejects(wire);
        let mut wire = valid(); wire.anchor.offset = 1; rejects(wire);
        let mut wire = valid(); wire.anchor.source = 9; rejects(wire);
        let mut wire = valid(); wire.sources[0].file_path.clear(); rejects(wire);
        let mut wire = valid(); wire.magic = [0; 8]; rejects(wire);
    }

    #[test]
    fn macro_a_01_wire_rejects_forged_adjacency_and_multi_line_runs() {
        let mut wire = valid();
        wire.runs.push(WireRun { source: 0, range: 3..4, kind: RunKind::Text, context: 0, boundary: JoinBoundary::SourceAdjacent, origin: None, construction: None });
        rejects(wire);
        let mut wire = valid();
        wire.sources[0].text = "\n\n".into();
        wire.runs[0].kind = RunKind::LineEnding;
        rejects(wire);
    }

    #[test]
    fn macro_a_06_wire_rejects_invalid_expression_boundaries_and_old_streams() {
        use super::super::fragments::ExpressionRange;
        let fragment = |range| ExpressionRange { range, header_indent: 0 };
        let mut wire = valid(); wire.magic = *b"RKLOSS01"; rejects(wire);
        for ranges in [vec![0..0], vec![1..2], vec![0..3], vec![0..2, 0..2]] {
            let mut wire = valid(); wire.expressions = ranges.into_iter().map(fragment).collect(); rejects(wire);
        }
        for ranges in [vec![0..4, 2..6], vec![2..4, 0..6]] {
            let mut wire = valid(); wire.sources[0].text = "abcdef".into(); wire.runs[0].range = 0..6;
            wire.expressions = ranges.into_iter().map(fragment).collect(); rejects(wire);
        }
        let mut wire = valid(); wire.sources[0].text = "a".repeat(129); wire.runs[0].range = 0..129;
        wire.expressions = (1..=129).rev().map(|end| fragment(0..end)).collect(); rejects(wire);
    }
}
