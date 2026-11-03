use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::lexer::Span;

mod collection;
mod conversion;
mod dto;
mod report;
mod source_attachment;
mod validation;

pub use collection::Diagnostics;
pub use validation::DiagnosticValidationError;

/// Trait for errors that carry span information.
/// Any error type implementing this trait can be converted to a Diagnostic.
pub trait SpannedError {
    /// The error message
    fn message(&self) -> String;
    /// The primary span where the error occurred
    fn span(&self) -> Option<Span>;
    /// Additional labels with their own spans.
    fn labels(&self) -> Vec<DiagnosticLabel> {
        vec![]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

impl DiagnosticSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Hint => "hint",
        }
    }
}

/// Stable diagnostic families exposed to protocol consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    Lexer,
    Parser,
    Macro,
    Source,
    File,
    Project,
    Artifact,
    Toolchain,
    Resolve,
    Type,
    Selection,
    Borrow,
    Mono,
    Codegen,
    Internal,
}

impl DiagnosticCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lexer => "lexer",
            Self::Parser => "parser",
            Self::Macro => "macro",
            Self::Source => "source",
            Self::File => "file",
            Self::Project => "project",
            Self::Artifact => "artifact",
            Self::Toolchain => "toolchain",
            Self::Resolve => "resolve",
            Self::Type => "type",
            Self::Selection => "selection",
            Self::Borrow => "borrow",
            Self::Mono => "mono",
            Self::Codegen => "codegen",
            Self::Internal => "internal",
        }
    }
}

impl std::fmt::Display for DiagnosticCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Compatibility alias for code that used the pre-contract severity name.
pub type DiagnosticType = DiagnosticSeverity;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticLabel {
    pub message: String,
    /// Half-open byte range in the source text identified by `span.file_path`.
    pub span: Span,
}

impl DiagnosticLabel {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        validate_span(&span);
        Self {
            message: message.into(),
            span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLocation {
    /// A half-open byte range in a loaded or virtual Rock source.
    Source(Span),
    /// A filesystem input/output location without a source range.
    File(PathBuf),
    /// A project or manifest location without a source range.
    Project(PathBuf),
    /// A compiler artifact location without a source range.
    Artifact(PathBuf),
    /// A toolchain or compiler-internal failure without a source range.
    Toolchain,
}

impl DiagnosticLocation {
    /// Construct a source location with a validated half-open byte range.
    pub fn source(span: Span) -> Self {
        validate_span(&span);
        Self::Source(span)
    }

    fn validate(&self) {
        match self {
            Self::Source(span) => validate_span(span),
            Self::File(path) | Self::Project(path) | Self::Artifact(path) => validate_path(path),
            Self::Toolchain => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSourceOrigin {
    FileSystem,
    Virtual,
    Artifact { artifact_path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticSource {
    /// Display path for filesystem, virtual, or artifact-backed source text.
    pub display_path: PathBuf,
    /// Complete source snapshot used to interpret byte offsets in labels.
    pub text: String,
    pub origin: DiagnosticSourceOrigin,
    pub related: BTreeMap<PathBuf, DiagnosticRelatedSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticRelatedSource {
    pub display_path: PathBuf,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct DiagnosticSourceMap {
    by_path: BTreeMap<PathBuf, DiagnosticSource>,
}

fn validate_path(path: &Path) {
    assert!(
        !path.as_os_str().is_empty(),
        "diagnostic locations require a path"
    );
}

fn validate_span(span: &Span) {
    validate_path(&span.file_path);
    assert!(
        span.start <= span.end,
        "diagnostic span start must not exceed its end"
    );
}

fn span_is_valid(span: &Span) -> bool {
    !span.file_path.as_os_str().is_empty() && span.start <= span.end
}

fn span_is_valid_for_text(span: &Span, text: &str) -> bool {
    span_is_valid(span)
        && span.end <= text.len()
        && text.is_char_boundary(span.start)
        && text.is_char_boundary(span.end)
}

fn spans_match(left: &Span, right: &Span) -> bool {
    left.file_path == right.file_path && left.start == right.start && left.end == right.end
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: Option<DiagnosticCode>,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub location: DiagnosticLocation,
    pub primary: Option<DiagnosticLabel>,
    pub secondary: Vec<DiagnosticLabel>,
    pub notes: Vec<String>,
    pub help: Vec<String>,
    pub source: Option<DiagnosticSource>,
}

/// Owned, protocol-neutral diagnostic payload for CLI and future LSP adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticDto {
    pub code: Option<String>,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub location: DiagnosticLocation,
    pub primary: Option<DiagnosticLabel>,
    pub secondary: Vec<DiagnosticLabel>,
    pub notes: Vec<String>,
    pub help: Vec<String>,
    pub source: Option<DiagnosticSource>,
}
impl Diagnostic {
    /// Create a source diagnostic with a primary label.
    ///
    /// `Span` offsets are zero-based byte offsets into the UTF-8 source text,
    /// and the range is half-open (`start..end`). Virtual sources use the same
    /// contract; their path need not exist on disk.
    pub fn new(message: String, span: Span) -> Self {
        Self::from_source_parts(message.clone(), message, span, DiagnosticCode::Source)
    }

    pub fn for_file(message: String, path: PathBuf) -> Self {
        Self::at_location_with_code(
            message,
            DiagnosticLocation::File(path),
            DiagnosticCode::File,
        )
    }

    pub fn for_project(message: String, path: PathBuf) -> Self {
        Self::at_location_with_code(
            message,
            DiagnosticLocation::Project(path),
            DiagnosticCode::Project,
        )
    }

    pub fn for_artifact(message: String, path: PathBuf) -> Self {
        Self::at_location_with_code(
            message,
            DiagnosticLocation::Artifact(path),
            DiagnosticCode::Artifact,
        )
    }

    pub fn for_toolchain(message: String) -> Self {
        Self::at_location_with_code(
            message,
            DiagnosticLocation::Toolchain,
            DiagnosticCode::Toolchain,
        )
    }

    pub fn for_internal(message: impl Into<String>) -> Self {
        Self::internal(message)
    }

    fn at_location_with_code(
        message: String,
        location: DiagnosticLocation,
        code: DiagnosticCode,
    ) -> Self {
        location.validate();
        Self {
            code: Some(code),
            severity: DiagnosticSeverity::Error,
            message,
            location,
            primary: None,
            secondary: Vec::new(),
            notes: Vec::new(),
            help: Vec::new(),
            source: None,
        }
    }

    fn from_source_parts(
        message: String,
        primary_message: String,
        span: Span,
        code: DiagnosticCode,
    ) -> Self {
        validate_span(&span);
        Self {
            code: Some(code),
            severity: DiagnosticSeverity::Error,
            message,
            location: DiagnosticLocation::Source(span.clone()),
            primary: Some(DiagnosticLabel::new(primary_message, span)),
            secondary: Vec::new(),
            notes: Vec::new(),
            help: Vec::new(),
            source: None,
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::at_location_with_code(
            message.into(),
            DiagnosticLocation::Toolchain,
            DiagnosticCode::Internal,
        )
    }

    pub fn with_code(mut self, code: DiagnosticCode) -> Self {
        self.code = Some(code);
        self
    }

    pub fn with_severity(mut self, severity: DiagnosticSeverity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help.push(help.into());
        self
    }

    /// Add a label with its own span
    pub fn with_label(mut self, label: String, span: Span) -> Self {
        assert!(
            matches!(&self.location, DiagnosticLocation::Source(_)),
            "source labels require a source diagnostic"
        );
        self.secondary.push(DiagnosticLabel::new(label, span));
        self
    }

    pub fn with_labels(mut self, labels: Vec<(String, Span)>) -> Self {
        assert!(
            matches!(&self.location, DiagnosticLocation::Source(_)),
            "source labels require a source diagnostic"
        );
        self.secondary.extend(
            labels
                .into_iter()
                .map(|(message, span)| DiagnosticLabel::new(message, span)),
        );
        self
    }

    pub fn with_secondary(mut self, labels: Vec<DiagnosticLabel>) -> Self {
        assert!(
            matches!(&self.location, DiagnosticLocation::Source(_)),
            "source labels require a source diagnostic"
        );
        self.secondary.extend(labels);
        self
    }

    pub fn with_source(mut self, source: DiagnosticSource) -> Self {
        validate_path(&source.display_path);
        assert!(
            matches!(&self.location, DiagnosticLocation::Source(_)),
            "source text can only be attached to source diagnostics"
        );
        self.source = Some(source);
        self
    }

    /// Create a diagnostic from any SpannedError
    pub fn from_spanned<E: SpannedError>(err: &E) -> Self {
        let span = err.span();
        let message = err.message();
        let mut diagnostic = span
            .map(|span| {
                Self::from_source_parts(
                    message.clone(),
                    message.clone(),
                    span,
                    DiagnosticCode::Source,
                )
            })
            .unwrap_or_else(|| Self::internal(message));
        let labels = err.labels();
        if matches!(&diagnostic.location, DiagnosticLocation::Source(_)) {
            diagnostic.secondary = labels;
        } else {
            diagnostic
                .notes
                .extend(labels.into_iter().map(|label| label.message));
        }
        diagnostic
    }
}

#[cfg(test)]
mod tests;
