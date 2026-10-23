use std::collections::HashMap;
use std::sync::Arc;

use super::{ConstructionContext, LosslessError, LosslessStream, ScanLimits, SourcePosition, SourceSnapshot, SyntaxContext};
use crate::ast::InvocationId;
use crate::macro_expansion::{ExpansionId, GeneratedSourceId, TokenOrigin, TokenProvenance};

pub(crate) struct LiteralSourceRange {
    pub decoded: std::ops::Range<usize>,
    pub source: crate::lexer::Span,
}

impl LosslessStream {
    /// Neutral scanning only. Scalar splits retain exact escape source extents
    /// while adjacency still comes exclusively from this new retained snapshot.
    pub(crate) fn mapped_literal(
        text: String, mappings: Vec<LiteralSourceRange>, context: SyntaxContext,
        expansion: ExpansionId, generated_source: GeneratedSourceId, limits: ScanLimits,
    ) -> Result<Self, String> {
        let mut stream = Self::from_bytes("<macro-literal>".into(), text.into_bytes(), limits)
            .map_err(|error| format!("literal fragment preservation: {error:?}"))?;
        let mut runs = Vec::new();
        let mut raw = 0;
        let mut end = 0;
        for mapping in mappings {
            if mapping.decoded.start != end || mapping.decoded.is_empty()
                || !stream.anchor.source.get(mapping.decoded.clone()).is_some_and(|text| text.chars().count() == 1) {
                return Err("invalid canonical literal source mapping".into());
            }
            while raw < stream.runs.len() && stream.runs[raw].range.end <= mapping.decoded.start { raw += 1; }
            let mut run = stream.runs.get(raw).filter(|run| mapping.decoded.end <= run.range.end)
                .ok_or("invalid literal scalar boundary")?.clone();
            if runs.len() == limits.max_runs { return Err("literal fragment unit limit exceeded".into()); }
            end = mapping.decoded.end;
            run.range = mapping.decoded;
            run.context = context;
            run.boundary = super::JoinBoundary::SourceAdjacent;
            run.origin = Some(TokenProvenance::Generated { expansion, generated_source, definition_span: mapping.source }.into());
            runs.push(run);
        }
        if end != stream.len_bytes { return Err("incomplete literal source mapping".into()); }
        stream.anchor.context = context;
        if let Some(first) = runs.first() { stream.anchor.origin = first.origin.clone(); }
        stream.runs = runs;
        Ok(stream)
    }

    /// Retain generated UTF-8 without granting binding or source-file authority.
    pub fn source_fragment(text: String, context: ConstructionContext, limits: ScanLimits) -> Result<Self, LosslessError> {
        let mut stream = Self::from_bytes("<macro-construction>".into(), text.into_bytes(), limits)?;
        for run in &mut stream.runs { run.construction = Some(context); }
        Ok(stream)
    }

    pub(crate) fn map_contexts(&self, mut map: impl FnMut(SyntaxContext) -> Result<SyntaxContext, String>) -> Result<Self, String> {
        let mut stream = self.clone();
        stream.anchor.context = map(stream.anchor.context)?;
        for run in &mut stream.runs {
            if run.construction.is_some() { return Err("unresolved construction in compiler template".into()); }
            run.context = map(run.context)?;
        }
        Ok(stream)
    }

    /// Validate against request data before applying compiler-owned origins and
    /// contexts. Equal text alone cannot authorize a forwarded source range.
    pub(crate) fn resolve_proc_output(
        &self,
        input: &Self,
        invocation: InvocationId,
        expansion: ExpansionId,
        generated_source: GeneratedSourceId,
        mut resolve: impl FnMut(ConstructionContext) -> Result<SyntaxContext, String>,
    ) -> Result<Self, String> {
        self.validate_forwarded_captures(input)?;
        let mut stream = self.generated(expansion, generated_source);
        let mut generated = HashMap::new();
        let mut sources = HashMap::new();
        let mut ranges = HashMap::<_, Vec<(usize, usize)>>::new();
        for run in &input.runs {
            if run.construction.is_some() { return Err("unresolved construction in proc input".into()); }
            sources.insert(run.source.identity, &run.source);
            ranges.entry((run.source.identity, run.context, run.kind, run.origin.as_ref())).or_default()
                .push((run.range.start, run.range.end));
        }
        // Prefix maxima authorize contained slices without quadratic rescanning
        // of repeated/overlapping input runs or repeated whole-source comparisons.
        for intervals in ranges.values_mut() {
            intervals.sort_unstable();
            let mut end = 0;
            for interval in intervals { end = end.max(interval.1); interval.1 = end; }
        }
        let mut checked_sources = std::collections::HashSet::new();
        for (original, output) in self.runs.iter().zip(&mut stream.runs) {
            if let Some(request) = original.construction {
                if request.invocation != invocation { return Err("construction context belongs to another invocation".into()); }
                if original.context != SyntaxContext::ROOT || original.source.file_path != std::path::Path::new("<macro-construction>")
                    || original.origin.is_some() {
                    return Err("forged generated context or source provenance".into());
                }
                output.context = resolve(request)?;
                output.construction = None;
                let key = Arc::as_ptr(&original.source);
                if let std::collections::hash_map::Entry::Vacant(entry) = generated.entry(key) {
                    let mut source = SourceSnapshot::new(original.source.file_path.clone(), String::new())
                        .map_err(|error| format!("generated snapshot identity: {error:?}"))?;
                    source.windows = original.source.windows.clone();
                    entry.insert(Arc::new(source));
                }
                output.source = Arc::clone(&generated[&key]);
            } else {
                let allowed = sources.get(&original.source.identity)
                    .ok_or_else(|| "proc output contains an unauthorized source snapshot".to_string())?;
                if checked_sources.insert(Arc::as_ptr(&original.source))
                    && (original.source.file_path != allowed.file_path
                        || original.source.windows.iter().any(|window| window.offset.checked_add(window.text.len())
                            .and_then(|end| allowed.get(window.offset..end)) != Some(window.text.as_str()))) {
                    return Err("proc output changed a forwarded source snapshot".into());
                }
                let intervals = ranges.get(&(original.source.identity, original.context, original.kind, original.origin.as_ref()))
                    .ok_or_else(|| "proc output contains an unauthorized syntax context or provenance".to_string())?;
                let end = intervals.partition_point(|(start, _)| *start <= original.range.start);
                if end == 0 || intervals[end - 1].1 < original.range.end {
                    return Err("proc output contains an unauthorized source range".into());
                }
                output.source = Arc::clone(allowed);
                output.context = original.context;
                output.origin = Some(TokenOrigin {
                    projection: original.origin.as_ref().and_then(|origin| origin.projection),
                    provenance: TokenProvenance::Captured {
                        expansion, invocation_span: input.start_span(), capture_span: original.span(),
                    },
                });
            }
        }
        // The anchor is not an output capability: rebuild it from validated data,
        // including when a worker returns only an empty stream.
        stream.anchor = match stream.runs.first() {
            Some(run) => SourcePosition { source: Arc::clone(&run.source), offset: run.range.start, context: run.context, origin: run.origin.clone() },
            None => input.anchor.clone(),
        };
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lossless::{ConstructionKind, JoinBoundary};
    const MACRO_LIMITS: ScanLimits = ScanLimits { max_source_bytes: 65536, max_runs: 4096 };
    fn source(text: &str) -> LosslessStream { LosslessStream::from_bytes("/virtual/test.rk".into(), text.as_bytes().to_vec(), MACRO_LIMITS).unwrap() }

    #[test]
    fn macro_a_14_construction_requests_survive_slicing_splicing_and_wire() {
        let invocation = InvocationId::fresh().unwrap();
        let request = ConstructionContext { invocation, kind: ConstructionKind::Fresh(3) };
        let stream = LosslessStream::source_fragment("variable".into(), request, MACRO_LIMITS).unwrap();
        assert_eq!(stream.binding_context(0..8), Err(LosslessError::UnresolvedConstruction));
        assert!(matches!(crate::lexer::interpretation::RockView::new(&stream).next_piece(),
            Err(crate::lexer::interpretation::RockError::Preservation(LosslessError::UnresolvedConstruction))));
        let left = stream.slice(0..3, MACRO_LIMITS).unwrap();
        let right = stream.slice(3..8, MACRO_LIMITS).unwrap();
        let joined = left.join(&right, JoinBoundary::SourceAdjacent, MACRO_LIMITS).unwrap();
        let wire = bincode::serialize(&joined).unwrap();
        let joined: LosslessStream = bincode::deserialize(&wire).unwrap();
        let output = joined.resolve_proc_output(&source(""), invocation, ExpansionId(1), GeneratedSourceId(1), |context| {
            assert_eq!(context, request);
            Ok(SyntaxContext::allocated(9))
        }).unwrap();
        assert_eq!(output.binding_context(0..8).unwrap(), SyntaxContext::allocated(9));
    }

    #[test]
    fn macro_a_14_proc_forwarding_rejects_forged_contexts_sources_and_owners() {
        let invocation = InvocationId::fresh().unwrap();
        let input = source("name");
        let resolve = |stream: &LosslessStream| stream.resolve_proc_output(&input, invocation, ExpansionId(1), GeneratedSourceId(1), |_| Ok(SyntaxContext::ROOT));
        assert!(resolve(&input.slice(1..3, MACRO_LIMITS).unwrap()).is_ok());
        let mut forged = input.clone();
        forged.runs[0].context = SyntaxContext::allocated(77);
        assert!(resolve(&forged).is_err());
        let full = source("hidden name");
        let permitted = full.slice(7..11, MACRO_LIMITS).unwrap();
        assert!(full.resolve_proc_output(&permitted, invocation, ExpansionId(1), GeneratedSourceId(1), |_| unreachable!()).is_err());
        let forged = LosslessStream::from_bytes("/elsewhere.rk".into(), b"name".to_vec(), MACRO_LIMITS).unwrap();
        assert!(resolve(&forged).is_err());
        assert!(resolve(&source("name")).is_err(), "equal source bytes are not snapshot authority");
        let forged = LosslessStream::source_fragment("name".into(), ConstructionContext {
            invocation: InvocationId::fresh().unwrap(), kind: ConstructionKind::CallSite,
        }, MACRO_LIMITS).unwrap();
        assert!(resolve(&forged).is_err());
    }

    #[test]
    fn macro_a_14_forwarding_contexts_roundtrip_but_forged_anchors_cannot_grant_authority() {
        let invocation = InvocationId::fresh().unwrap();
        let input = source("name").map_contexts(|_| Ok(SyntaxContext::allocated(37))).unwrap();
        let wire = bincode::serialize(&input).unwrap();
        let mut forwarded: LosslessStream = bincode::deserialize(&wire).unwrap();
        forwarded.anchor.context = SyntaxContext::allocated(900);
        forwarded.anchor.source = source("elsewhere").anchor.source;
        let output = forwarded.resolve_proc_output(&input, invocation, ExpansionId(1), GeneratedSourceId(1), |_| unreachable!()).unwrap();
        assert_eq!(output.binding_context(0..4).unwrap(), SyntaxContext::allocated(37));
        assert_eq!(output.anchor.context, SyntaxContext::allocated(37));
        let empty = forwarded.slice(0..0, MACRO_LIMITS).unwrap();
        let output = empty.resolve_proc_output(&input, invocation, ExpansionId(1), GeneratedSourceId(1), |_| unreachable!()).unwrap();
        assert_eq!(output.anchor, input.anchor);
    }

    #[test]
    fn macro_a_14_constructed_identifier_cannot_merge_conflicting_contexts() {
        let invocation = InvocationId::fresh().unwrap();
        let stream = LosslessStream::source_fragment("identifier".into(), ConstructionContext { invocation, kind: ConstructionKind::Fresh(0) }, MACRO_LIMITS).unwrap();
        let mut left = stream.slice(0..5, MACRO_LIMITS).unwrap();
        let mut right = stream.slice(5..10, MACRO_LIMITS).unwrap();
        right.runs[0].construction = Some(ConstructionContext { invocation, kind: ConstructionKind::Fresh(1) });
        left = left.join(&right, JoinBoundary::SourceAdjacent, MACRO_LIMITS).unwrap();
        let output = left.resolve_proc_output(&source(""), invocation, ExpansionId(1), GeneratedSourceId(1), |request| {
            let ConstructionKind::Fresh(id) = request.kind else { unreachable!() };
            Ok(SyntaxContext::allocated(id + 1))
        }).unwrap();
        assert!(matches!(output.binding_context(0..10), Err(LosslessError::ConflictingContexts { .. })));
    }

    #[test]
    fn macro_a_15_worker_cannot_forge_a_forwarded_diagnostic_projection() {
        let invocation = InvocationId::fresh().unwrap();
        let input = source("name").project_output(ExpansionId(1), GeneratedSourceId(1), 0);
        let wire = bincode::serialize(&input).unwrap();
        let mut forwarded: LosslessStream = bincode::deserialize(&wire).unwrap();
        let resolve = |output: &LosslessStream| output.resolve_proc_output(&input, invocation, ExpansionId(2), GeneratedSourceId(2), |_| unreachable!());
        assert!(resolve(&forwarded).is_ok());
        forwarded.runs[0].origin.as_mut().unwrap().projection.as_mut().unwrap().output_start += 1;
        assert!(resolve(&forwarded).is_err());
    }

    #[test]
    fn macro_review_transport_windows_preserve_offsets_without_whole_files() {
        use crate::macro_expansion::proc_macro::{decode_stream, encode_stream};
        let text = format!("{}雪{}", " ".repeat(1_100_000), " ".repeat(1_100_000));
        let full = LosslessStream::from_bytes("/large.rk".into(), text.into_bytes(), crate::parser::SOURCE_LIMITS).unwrap();
        let input = full.slice(1_100_000..1_100_003, MACRO_LIMITS).unwrap();
        let bytes = encode_stream(&input).unwrap();
        assert!(bytes.len() < 256, "{}", bytes.len());
        let forwarded = decode_stream(&bytes).unwrap();
        assert_eq!(forwarded.start_span(), input.start_span());
        assert_eq!(forwarded.end_span(), input.end_span());
        assert_eq!(forwarded.runs[0].text(), "雪");
        let invocation = InvocationId::fresh().unwrap();
        let resolve = |stream: &LosslessStream| stream.resolve_proc_output(&input, invocation, ExpansionId(1), GeneratedSourceId(1), |_| unreachable!());
        assert!(resolve(&forwarded).is_ok());
        let repeated = forwarded.join(&forwarded, JoinBoundary::Separate, MACRO_LIMITS).unwrap();
        assert!(encode_stream(&repeated).unwrap().len() < 512);
        assert!(resolve(&repeated).is_ok());
        let empty = decode_stream(&encode_stream(&input.slice(3..3, MACRO_LIMITS).unwrap()).unwrap()).unwrap();
        assert_eq!(empty.start_span(), input.end_span());
        let mut forged = forwarded.clone();
        forged.runs[0].range = 1_099_999..1_100_000;
        assert!(resolve(&forged).is_err());
    }
}
