use crate::diagnostic::DiagnosticCode;
use crate::lexer::Span;
use crate::lower::ResolveError;

#[derive(Debug, Default, Clone)]
pub(crate) struct LowerDiagnostics {
    errors: Vec<ResolveError>,
}

impl LowerDiagnostics {
    pub(crate) fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    pub(crate) fn errors(&self) -> &[ResolveError] {
        &self.errors
    }

    pub(crate) fn into_errors(self) -> Vec<ResolveError> {
        self.errors
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct LowerDiagnosticSink {
    diagnostics: LowerDiagnostics,
    retractable: std::collections::BTreeMap<usize, usize>,
    next_retractable: usize,
}

impl LowerDiagnosticSink {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&mut self, message: String, span: Span) {
        self.push_with_code(message, span, DiagnosticCode::Resolve);
    }

    pub(crate) fn push_with_span(&mut self, message: String, span: Span) {
        self.push(message, span);
    }

    pub(crate) fn push_with_span_and_labels(
        &mut self,
        message: String,
        span: Span,
        labels: Vec<crate::diagnostic::DiagnosticLabel>,
    ) {
        self.diagnostics
            .errors
            .push(ResolveError::with_span_and_labels(message, span, labels));
    }

    pub(crate) fn push_with_code(&mut self, message: String, span: Span, code: DiagnosticCode) {
        self.diagnostics
            .errors
            .push(ResolveError::with_span_code(message, span, code));
    }

    pub(crate) fn push_type_with_span(&mut self, message: String, span: Span) {
        self.push_with_code(message, span, DiagnosticCode::Type);
    }

    pub(crate) fn push_type_with_span_and_labels(
        &mut self,
        message: String,
        span: Span,
        labels: Vec<crate::diagnostic::DiagnosticLabel>,
    ) {
        let mut error = ResolveError::with_span_and_labels(message, span, labels);
        error.code = DiagnosticCode::Type;
        self.diagnostics.errors.push(error);
    }

    pub(crate) fn push_selection_with_span(&mut self, message: String, span: Span) {
        self.push_with_code(message, span, DiagnosticCode::Selection);
    }

    pub(crate) fn push_retractable_selection(&mut self, message: String, span: Span) -> usize {
        let id = self.next_retractable;
        self.next_retractable += 1;
        self.retractable.insert(id, self.diagnostics.errors.len());
        self.push_selection_with_span(message, span);
        id
    }

    pub(crate) fn retract_selection(&mut self, id: usize) {
        if let Some(index) = self.retractable.remove(&id) {
            self.diagnostics.errors.remove(index);
            for other in self.retractable.values_mut() {
                if *other > index {
                    *other -= 1;
                }
            }
        }
    }

    pub(crate) fn push_toolchain(&mut self, message: String) {
        self.diagnostics.errors.push(ResolveError::non_source_code(
            message,
            DiagnosticCode::Toolchain,
        ));
    }

    pub(crate) fn push_toolchain_once(&mut self, message: String) {
        if !self
            .diagnostics
            .errors
            .iter()
            .any(|error| error.message == message)
        {
            self.push_toolchain(message);
        }
    }

    pub(crate) fn extend(&mut self, errors: Vec<ResolveError>) {
        self.diagnostics.errors.extend(errors);
    }

    pub(crate) fn has_message(&self, message: &str) -> bool {
        self.diagnostics
            .errors
            .iter()
            .any(|error| error.message == message)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub(crate) fn errors(&self) -> &[ResolveError] {
        self.diagnostics.errors()
    }

    pub(crate) fn finish(self) -> LowerDiagnostics {
        self.diagnostics
    }

    pub(crate) fn into_errors(self) -> Vec<ResolveError> {
        self.finish().into_errors()
    }
}

#[cfg(test)]
mod tests;
