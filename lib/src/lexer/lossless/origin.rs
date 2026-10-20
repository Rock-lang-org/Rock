use super::LosslessStream;
use crate::macro_expansion::{ExpansionId, GeneratedSourceId, OutputProjection, TokenOrigin, TokenProvenance};
use crate::lexer::Span;

impl LosslessStream {
    /// Project locations only; snapshots, contexts, adjacency and opaque captures
    /// are unchanged. Reconstructed text is solely for diagnostic inspection.
    pub(crate) fn project_output(&self, expansion: ExpansionId, generated_source: GeneratedSourceId, namespace: u64) -> Self {
        let mut stream = self.clone();
        let mut output_start = 0;
        for run in &mut stream.runs {
            run.origin = Some(TokenOrigin { provenance: run.origin().provenance,
                projection: Some(OutputProjection { namespace, expansion, generated_source, source_start: run.range.start, output_start }) });
            output_start += run.text().len();
        }
        let provenance = stream.anchor.origin.as_ref().map(|origin| origin.provenance.clone())
            .unwrap_or_else(|| TokenProvenance::Source(self.start_span()));
        stream.anchor.origin = Some(TokenOrigin { provenance,
            projection: Some(OutputProjection { namespace, expansion, generated_source, source_start: stream.anchor.offset, output_start: 0 }) });
        stream
    }

    pub(crate) fn captured(&self, expansion: ExpansionId, invocation_span: &Span) -> Self {
        let mut stream = self.clone();
        for run in &mut stream.runs {
            run.origin = Some(TokenOrigin {
                projection: run.origin.as_ref().and_then(|origin| origin.projection),
                provenance: TokenProvenance::Captured {
                    expansion, invocation_span: invocation_span.clone(), capture_span: run.span(),
                },
            });
        }
        stream
    }

    /// A virtual diagnostic location does not replace the underlying snapshot:
    /// separately sliced template pieces must retain their provable adjacency.
    pub(crate) fn generated(&self, expansion: ExpansionId, generated_source: GeneratedSourceId) -> Self {
        let mut stream = self.clone();
        stream.anchor.origin = Some(TokenProvenance::Generated {
            expansion, generated_source, definition_span: self.start_span(),
        }.into());
        for run in &mut stream.runs {
            run.origin = Some(TokenProvenance::Generated {
                expansion, generated_source, definition_span: run.span(),
            }.into());
        }
        stream
    }
}
