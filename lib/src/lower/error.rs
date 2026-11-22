//! Resolution error types

use crate::diagnostic::DiagnosticCode;
use crate::diagnostic::DiagnosticLabel;
use crate::lexer::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveErrorKind {
    Source,
    NonSource,
}

/// A resolution error whose origin is explicit.
#[derive(Debug, Clone)]
pub struct ResolveError {
    pub message: String,
    origin: ResolveErrorOrigin,
    pub code: DiagnosticCode,
    pub labels: Vec<DiagnosticLabel>,
    /// A method receiver no caller fixed; usually a consequence of another error.
    undetermined_receiver: bool,
}

#[derive(Debug, Clone)]
enum ResolveErrorOrigin {
    Source(Span),
    NonSource,
}

impl ResolveError {
    /// Create a source error at the operation that caused it.
    pub fn with_span(message: String, span: Span) -> Self {
        Self::with_span_code(message, span, DiagnosticCode::Resolve)
    }

    pub fn with_span_code(message: String, span: Span, code: DiagnosticCode) -> Self {
        Self {
            message,
            origin: ResolveErrorOrigin::Source(span),
            code,
            labels: Vec::new(),
            undetermined_receiver: false,
        }
    }

    pub fn with_span_and_labels(message: String, span: Span, labels: Vec<DiagnosticLabel>) -> Self {
        Self {
            message,
            origin: ResolveErrorOrigin::Source(span),
            code: DiagnosticCode::Resolve,
            labels,
            undetermined_receiver: false,
        }
    }

    /// A method call whose receiver type nothing fixed.
    pub fn undetermined_receiver(message: String, span: Span) -> Self {
        Self {
            undetermined_receiver: true,
            ..Self::with_span_code(message, span, DiagnosticCode::Type)
        }
    }

    /// Marks an error as caused by a receiver type nothing fixed.
    pub fn into_undetermined_receiver(mut self) -> Self {
        self.undetermined_receiver = true;
        self
    }

    pub fn is_undetermined_receiver(&self) -> bool {
        self.undetermined_receiver
    }

    /// Create an error that is not attributable to a source range.
    pub fn non_source(message: String) -> Self {
        Self::non_source_code(message, DiagnosticCode::Internal)
    }

    pub fn non_source_code(message: String, code: DiagnosticCode) -> Self {
        Self {
            message,
            origin: ResolveErrorOrigin::NonSource,
            code,
            labels: Vec::new(),
            undetermined_receiver: false,
        }
    }

    pub fn from_optional_span(message: String, span: Option<Span>) -> Self {
        Self::from_optional_span_code(message, span, DiagnosticCode::Resolve)
    }

    pub fn from_optional_span_code(
        message: String,
        span: Option<Span>,
        code: DiagnosticCode,
    ) -> Self {
        match span {
            Some(span) => Self::with_span_code(message, span, code),
            None => Self::non_source_code(message, code),
        }
    }

    pub fn span(&self) -> Option<Span> {
        match &self.origin {
            ResolveErrorOrigin::Source(span) => Some(span.clone()),
            ResolveErrorOrigin::NonSource => None,
        }
    }

    pub fn kind(&self) -> ResolveErrorKind {
        match &self.origin {
            ResolveErrorOrigin::Source(_) => ResolveErrorKind::Source,
            ResolveErrorOrigin::NonSource => ResolveErrorKind::NonSource,
        }
    }
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

#[cfg(test)]
mod tests;
