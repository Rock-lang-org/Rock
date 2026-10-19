use super::LosslessStream;
use crate::macro_expansion::{ExpansionId, GeneratedSourceId, TokenOrigin};
use crate::lexer::Span;

impl LosslessStream {
    pub(crate) fn captured(&self, expansion: ExpansionId, invocation_span: &Span) -> Self {
        let mut stream = self.clone();
        for run in &mut stream.runs {
            run.origin = Some(TokenOrigin::Captured {
                expansion, invocation_span: invocation_span.clone(), capture_span: run.span(),
            });
        }
        stream
    }

    /// A virtual diagnostic location does not replace the underlying snapshot:
    /// separately sliced template pieces must retain their provable adjacency.
    pub(crate) fn generated(&self, expansion: ExpansionId, generated_source: GeneratedSourceId) -> Self {
        let mut stream = self.clone();
        stream.anchor.origin = Some(TokenOrigin::Generated {
            expansion, generated_source, definition_span: self.start_span(),
        });
        for run in &mut stream.runs {
            run.origin = Some(TokenOrigin::Generated {
                expansion, generated_source, definition_span: run.span(),
            });
        }
        stream
    }
}
