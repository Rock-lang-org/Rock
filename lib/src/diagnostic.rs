use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ariadne::{Color, ColorGenerator, Label, Report, ReportKind};
use serde::{Deserialize, Serialize};

use crate::lexer::{LexerError, Span};
use crate::parser::ParseError;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticValidationError {
    InvalidLocation,
    MissingPrimaryLabel,
    PrimaryLabelDoesNotMatchLocation,
    InvalidLabel,
    SourceAttachedToNonSource,
}

impl DiagnosticSourceMap {
    pub fn from_source_database(database: &crate::source_loader::SourceDatabase) -> Self {
        let mut sources = Self::default();
        for file in database.source_files() {
            sources.insert_source_file(file);
        }
        sources
    }

    pub fn source_for_path(&self, path: &Path) -> Option<&DiagnosticSource> {
        self.by_path.get(path)
    }

    pub fn attach(&self, diagnostic: &mut Diagnostic) {
        let DiagnosticLocation::Source(primary_span) = &diagnostic.location else {
            return;
        };
        let source = diagnostic
            .source
            .clone()
            .or_else(|| self.source_for_path(&primary_span.file_path).cloned());
        if let Some(mut source) = source {
            for label in diagnostic.primary.iter().chain(diagnostic.secondary.iter()) {
                let span = &label.span;
                if span.file_path != primary_span.file_path {
                    if let Some(related) = self.source_for_path(&span.file_path) {
                        source.related.insert(
                            span.file_path.clone(),
                            DiagnosticRelatedSource {
                                display_path: related.display_path.clone(),
                                text: related.text.clone(),
                            },
                        );
                    }
                }
            }
            diagnostic.source = Some(source);
            if diagnostic.primary.is_none() {
                diagnostic.primary = Some(DiagnosticLabel::new(
                    diagnostic.message.clone(),
                    primary_span.clone(),
                ));
            }
        }
    }

    fn insert_source_file(&mut self, file: &crate::source_loader::SourceFile) {
        let origin = match &file.origin {
            crate::source_loader::SourceOrigin::FileSystem => DiagnosticSourceOrigin::FileSystem,
            crate::source_loader::SourceOrigin::Virtual => DiagnosticSourceOrigin::Virtual,
            crate::source_loader::SourceOrigin::Artifact { artifact_path } => {
                DiagnosticSourceOrigin::Artifact {
                    artifact_path: artifact_path.clone(),
                }
            }
        };
        let source = DiagnosticSource {
            display_path: file.display_path.clone(),
            text: file.text.clone(),
            origin,
            related: BTreeMap::new(),
        };
        for path in [
            &file.original_path,
            &file.canonical_path,
            &file.display_path,
        ] {
            self.by_path.insert(path.clone(), source.clone());
        }
    }
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
    /// Convert a resolution error while preserving its owning diagnostic family.
    pub fn from_resolve_error(error: &crate::lower::ResolveError) -> Self {
        let origin_valid = matches!(
            (error.kind(), error.span()),
            (crate::lower::ResolveErrorKind::Source, Some(_))
                | (crate::lower::ResolveErrorKind::NonSource, None)
        );
        if !origin_valid {
            return Self::for_internal("resolution error has inconsistent source origin")
                .with_code(DiagnosticCode::Internal);
        }
        let message = error.message.clone();
        let mut diagnostic = error
            .span()
            .map(|span| Self::from_source_parts(message.clone(), message.clone(), span, error.code))
            .unwrap_or_else(|| Self::internal(message));
        diagnostic.code = Some(error.code);
        if matches!(&diagnostic.location, DiagnosticLocation::Source(_)) {
            diagnostic.secondary = error.labels.clone();
        } else {
            diagnostic
                .notes
                .extend(error.labels.iter().map(|label| label.message.clone()));
        }
        diagnostic
    }

    /// Validate the public payload before handing it to a protocol adapter.
    pub fn validate(&self) -> Result<(), DiagnosticValidationError> {
        let location_valid = match &self.location {
            DiagnosticLocation::Source(span) => span_is_valid(span),
            DiagnosticLocation::File(path)
            | DiagnosticLocation::Project(path)
            | DiagnosticLocation::Artifact(path) => !path.as_os_str().is_empty(),
            DiagnosticLocation::Toolchain => true,
        };
        if !location_valid {
            return Err(DiagnosticValidationError::InvalidLocation);
        }

        match &self.location {
            DiagnosticLocation::Source(span) => {
                let Some(primary) = &self.primary else {
                    return Err(DiagnosticValidationError::MissingPrimaryLabel);
                };
                if !spans_match(&primary.span, span) {
                    return Err(DiagnosticValidationError::PrimaryLabelDoesNotMatchLocation);
                }
            }
            DiagnosticLocation::File(_)
            | DiagnosticLocation::Project(_)
            | DiagnosticLocation::Artifact(_)
            | DiagnosticLocation::Toolchain => {
                if self.primary.is_some() || !self.secondary.is_empty() || self.source.is_some() {
                    return Err(DiagnosticValidationError::SourceAttachedToNonSource);
                }
            }
        }

        if self
            .primary
            .iter()
            .chain(self.secondary.iter())
            .any(|label| !span_is_valid(&label.span))
        {
            return Err(DiagnosticValidationError::InvalidLabel);
        }

        if let DiagnosticLocation::Source(primary_span) = &self.location {
            if let Some(source) = &self.source {
                if source.display_path.as_os_str().is_empty()
                    || source.related.iter().any(|(path, related)| {
                        path.as_os_str().is_empty() || related.display_path.as_os_str().is_empty()
                    })
                {
                    return Err(DiagnosticValidationError::InvalidLocation);
                }
                if !span_is_valid_for_text(primary_span, &source.text) {
                    return Err(DiagnosticValidationError::InvalidLocation);
                }
                for label in self.primary.iter().chain(self.secondary.iter()) {
                    let text = if label.span.file_path == primary_span.file_path {
                        Some(source.text.as_str())
                    } else {
                        source
                            .related
                            .get(&label.span.file_path)
                            .map(|related| related.text.as_str())
                    };
                    let Some(text) = text else {
                        return Err(DiagnosticValidationError::InvalidLabel);
                    };
                    if !span_is_valid_for_text(&label.span, text) {
                        return Err(DiagnosticValidationError::InvalidLabel);
                    }
                }
            }
        }
        Ok(())
    }
}

impl From<ParseError> for Diagnostic {
    fn from(err: ParseError) -> Self {
        match err {
            ParseError::UnexpectedToken(_expected_desc, got_token) => {
                let got_display = if got_token.token_type.to_string().is_empty() {
                    format!("end of file")
                } else {
                    format!("'{}'", got_token.token_type.to_string())
                };

                Diagnostic::from_source_parts(
                    format!("Unexpected token: {}", got_display),
                    format!("Unexpected {}", got_display),
                    got_token.span,
                    DiagnosticCode::Parser,
                )
            }
            /* ParseError::UnexpectedKeyword(token, expected) => Diagnostic {
                message: format!("Unexpected keyword: {:?}", token.token_type,),
                labels: vec![(
                    format!("Expected one of {:?}", expected),
                    token.span.clone(),
                )],
                span: token.span,
                kind: DiagnosticType::Error,
            }, */
            ParseError::Lexer(LexerError::UnknownToken(c, span)) => Diagnostic::from_source_parts(
                format!("Lexer: Unknown token: {:?}", c),
                "Unknown token".to_string(),
                span,
                DiagnosticCode::Lexer,
            ),
            ParseError::MacroNoCorrespondance {
                macro_name,
                invoc_name,
                invoc_arg,
            } => {
                let mut diagnostic = Diagnostic::from_source_parts(
                    "Macro: Nothing expected this token".to_string(),
                    "For this macro".to_string(),
                    macro_name,
                    DiagnosticCode::Macro,
                );
                diagnostic
                    .secondary
                    .push(DiagnosticLabel::new("In this macro invocation", invoc_name));
                if let Some(invoc_arg) = invoc_arg {
                    diagnostic
                        .secondary
                        .push(DiagnosticLabel::new("With this token", invoc_arg));
                }
                diagnostic
            }
            /* ParseError::InvalidPrecedence(precedence, token) => Diagnostic {
                message: format!("Invalid precedence: {:?}", precedence),
                labels: vec![(
                    format!("Precedence must be between 0 and 9"),
                    token.span.clone(),
                )],
                span: token.span,
                kind: DiagnosticType::Error,
            },
            ParseError::IndentMismatch(got, expected, span) => Diagnostic {
                message: format!("Indent mismatch: got {}, expected {}", got, expected),
                labels: vec![
                    (format!("Expected indent level: {}", expected), span.clone()),
                    (format!("Got indent level: {}", got), span.clone()),
                ],
                kind: DiagnosticType::Error,
            }, */
            ParseError::UnexpectedEOF(span) => Diagnostic::from_source_parts(
                "Unexpected end of file".to_string(),
                "Expected more input".to_string(),
                span,
                DiagnosticCode::Parser,
            ),
            /* ParseError::LeftoverTokens(tokens) => Diagnostic {
                message: format!("Leftover tokens"),
                labels: vec![(format!("Expected end of file"), tokens[0].span.clone())],
                span: tokens[0].span.clone(),
                kind: DiagnosticType::Error,
            }, */
            ParseError::UnknownFile(file) => Diagnostic::at_location_with_code(
                format!("Unknown file: {:?}", file),
                DiagnosticLocation::File(PathBuf::from(file)),
                DiagnosticCode::File,
            ),
            ParseError::UnexpectedIndent(level, span) => Diagnostic::from_source_parts(
                format!("Unexpected indent level: {level}"),
                "Unexpected indentation".to_string(),
                span,
                DiagnosticCode::Parser,
            ),
            ParseError::ExpectedOneOrMore(span) => Diagnostic::from_source_parts(
                "Expected one or more".to_string(),
                "Expected at least one item".to_string(),
                span,
                DiagnosticCode::Parser,
            ),
            ParseError::Fail => Diagnostic::internal("Parser control-flow error escaped: Fail"),
            ParseError::ShortCircuit => {
                Diagnostic::internal("Parser control-flow error escaped: ShortCircuit")
            }
            ParseError::AssertFailed => {
                Diagnostic::internal("Parser control-flow error escaped: AssertFailed")
            }
            ParseError::HardError(msg, span) => {
                Diagnostic::from_source_parts(msg.clone(), msg, span, DiagnosticCode::Parser)
            }
            ParseError::WithContext { context, error } => {
                let mut context_chain = vec![context];
                let mut current = error;
                while let ParseError::WithContext {
                    context: inner_context,
                    error: inner_error,
                } = current.as_ref()
                {
                    context_chain.push(inner_context.clone());
                    current = inner_error.clone();
                }

                let inner_error = *current;
                let mut diagnostic = Diagnostic::from(inner_error);
                diagnostic.message = format!(
                    "{}\n\nParsing context (innermost first):\n{}",
                    diagnostic.message,
                    context_chain
                        .iter()
                        .enumerate()
                        .map(|(index, context)| format!("  {}. {}", index + 1, context))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                diagnostic.notes.extend(context_chain);
                diagnostic
            }
        }
    }
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

    /// Convert into an owned payload with string codes for protocol adapters.
    pub fn to_dto(&self) -> Result<DiagnosticDto, DiagnosticValidationError> {
        self.validate()?;
        Ok(DiagnosticDto::from_validated(self))
    }

    pub fn report(&self) {
        if let Err(error) = self.validate() {
            eprintln!(
                "error [internal]: invalid diagnostic payload ({error:?}): {}",
                self.message
            );
            return;
        }
        let source_content = match &self.location {
            DiagnosticLocation::Source(span) => {
                if let Some(source) = &self.source {
                    Some((
                        source.display_path.to_string_lossy().to_string(),
                        source.text.clone(),
                        source.related.clone(),
                        span.clone(),
                    ))
                } else if span.file_path.exists() {
                    std::fs::read_to_string(&span.file_path).ok().map(|source| {
                        (
                            span.file_path.to_string_lossy().to_string(),
                            source,
                            BTreeMap::new(),
                            span.clone(),
                        )
                    })
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some((file_id, source, related_sources, primary_span)) = source_content {
            if !span_is_valid_for_text(&primary_span, &source) {
                eprintln!(
                    "error [internal]: invalid diagnostic source range: {}",
                    self.message
                );
                return;
            }
            if self
                .primary
                .iter()
                .chain(self.secondary.iter())
                .any(|label| {
                    label.span.file_path == primary_span.file_path
                        && !span_is_valid_for_text(&label.span, &source)
                })
            {
                eprintln!(
                    "error [internal]: invalid diagnostic label range: {}",
                    self.message
                );
                return;
            }
            // File exists - use ariadne for pretty printing
            let mut colors = ColorGenerator::new();

            let report_kind = match self.severity {
                DiagnosticSeverity::Error => ReportKind::Error,
                DiagnosticSeverity::Warning => ReportKind::Warning,
                DiagnosticSeverity::Info | DiagnosticSeverity::Hint => ReportKind::Advice,
            };
            let mut builder = Report::build(report_kind, file_id.clone(), primary_span.start)
                .with_message(self.message.clone());
            if let Some(code) = self.code {
                builder = builder.with_code(code);
            }
            if !self.notes.is_empty() {
                builder = builder.with_note(self.notes.join("\n"));
            }
            if !self.help.is_empty() {
                builder = builder.with_help(self.help.join("\n"));
            }

            for (i, label) in self.primary.iter().chain(self.secondary.iter()).enumerate() {
                let span = &label.span;
                let label_file_id = if span.file_path == primary_span.file_path {
                    file_id.clone()
                } else if let Some(related) = related_sources.get(&span.file_path) {
                    related.display_path.to_string_lossy().to_string()
                } else {
                    span.file_path.to_string_lossy().to_string()
                };
                let color = if i == 0 {
                    Color::Fixed(9)
                } else {
                    colors.next()
                };
                builder = builder.with_label(
                    Label::new((label_file_id, span.start..span.end))
                        .with_message(&label.message)
                        .with_color(color),
                );
            }

            let mut report_sources = BTreeMap::from([(file_id, source)]);
            for related in related_sources.values() {
                report_sources.insert(
                    related.display_path.to_string_lossy().to_string(),
                    related.text.clone(),
                );
            }
            for label in self.primary.iter().chain(self.secondary.iter()) {
                let span = &label.span;
                if span.file_path != primary_span.file_path
                    && !related_sources.contains_key(&span.file_path)
                {
                    if let Ok(text) = std::fs::read_to_string(&span.file_path) {
                        if !span_is_valid_for_text(span, &text) {
                            eprintln!(
                                "error [internal]: invalid diagnostic label range: {}",
                                self.message
                            );
                            return;
                        }
                        report_sources.insert(span.file_path.to_string_lossy().to_string(), text);
                    }
                }
            }
            if let Err(error) = builder.finish().print(ariadne::sources(report_sources)) {
                eprintln!("error [internal]: failed to render diagnostic: {error}");
                return;
            }

            println!();
        } else {
            // No source available - print a simple error message with location info
            let code = self
                .code
                .map(|code| format!(" [{}]", code))
                .unwrap_or_default();
            eprintln!("{}{}: {}", self.severity.as_str(), code, self.message);

            match &self.location {
                DiagnosticLocation::Source(span) => eprintln!(
                    "  --> {}:{}-{}",
                    span.file_path.display(),
                    span.start,
                    span.end
                ),
                DiagnosticLocation::File(path) => eprintln!("  --> file {}", path.display()),
                DiagnosticLocation::Project(path) => {
                    eprintln!("  --> project {}", path.display())
                }
                DiagnosticLocation::Artifact(path) => {
                    eprintln!("  --> artifact {}", path.display())
                }
                DiagnosticLocation::Toolchain => eprintln!("  --> toolchain"),
            }

            for label in self.primary.iter().chain(self.secondary.iter()) {
                let span = &label.span;
                if !span.file_path.as_os_str().is_empty() {
                    eprintln!(
                        "      {}: {} ({}-{})",
                        label.message,
                        span.file_path.display(),
                        span.start,
                        span.end
                    );
                }
            }

            for note in &self.notes {
                eprintln!("  note: {note}");
            }
            for help in &self.help {
                eprintln!("  help: {help}");
            }

            println!();
        }
    }
}

impl DiagnosticDto {
    fn from_validated(diagnostic: &Diagnostic) -> Self {
        Self {
            code: diagnostic.code.map(|code| code.as_str().to_string()),
            severity: diagnostic.severity,
            message: diagnostic.message.clone(),
            location: diagnostic.location.clone(),
            primary: diagnostic.primary.clone(),
            secondary: diagnostic.secondary.clone(),
            notes: diagnostic.notes.clone(),
            help: diagnostic.help.clone(),
            source: diagnostic.source.clone(),
        }
    }
}

impl TryFrom<&Diagnostic> for DiagnosticDto {
    type Error = DiagnosticValidationError;

    fn try_from(diagnostic: &Diagnostic) -> Result<Self, Self::Error> {
        diagnostic.to_dto()
    }
}

impl TryFrom<Diagnostic> for DiagnosticDto {
    type Error = DiagnosticValidationError;

    fn try_from(diagnostic: Diagnostic) -> Result<Self, Self::Error> {
        diagnostic.to_dto()
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    pub fn from_resolve_errors(errors: Vec<crate::lower::ResolveError>) -> Self {
        Self(errors.iter().map(Diagnostic::from_resolve_error).collect())
    }

    pub fn push<T: Into<Diagnostic>>(&mut self, diagnostic: T) -> Self {
        let diagnostic = diagnostic.into();

        self.0.push(diagnostic);

        self.clone()
    }

    /// Push a SpannedError as a diagnostic
    pub fn push_error<E: SpannedError>(&mut self, err: &E) -> Self {
        self.0.push(Diagnostic::from_spanned(err));
        self.clone()
    }

    /// Push multiple errors
    pub fn extend_errors<E: SpannedError>(&mut self, errors: &[E]) -> Self {
        for err in errors {
            self.0.push(Diagnostic::from_spanned(err));
        }
        self.clone()
    }

    pub fn merge(&mut self, mut diagnostics: Diagnostics) {
        self.0.append(&mut diagnostics.0);
    }

    pub fn attach_sources(&mut self, sources: &DiagnosticSourceMap) {
        for diagnostic in &mut self.0 {
            sources.attach(diagnostic);
        }
    }

    pub fn with_sources(mut self, sources: &DiagnosticSourceMap) -> Self {
        self.attach_sources(sources);
        self
    }

    pub fn to_dto(&self) -> Result<Vec<DiagnosticDto>, DiagnosticValidationError> {
        self.0.iter().map(Diagnostic::to_dto).collect()
    }

    pub fn report(&self) {
        for diagnostic in &self.0 {
            diagnostic.report();
        }
    }

    pub fn return_if_err(&self) -> Result<(), Self> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(self.clone())
        }
    }

    pub fn contains(&self, text: &str) -> bool {
        self.0.iter().any(|diagnostic| {
            diagnostic.message.contains(text)
                || diagnostic.notes.iter().any(|note| note.contains(text))
        })
    }
}

impl std::fmt::Display for Diagnostics {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, diagnostic) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str("\n")?;
            }
            formatter.write_str(&diagnostic.message)?;
        }
        Ok(())
    }
}

impl From<ParseError> for Diagnostics {
    fn from(err: ParseError) -> Self {
        let mut diagnostics = Diagnostics::default();

        diagnostics.push(Diagnostic::from(err));

        diagnostics
    }
}

impl From<crate::lower::ResolveError> for Diagnostic {
    fn from(err: crate::lower::ResolveError) -> Self {
        Diagnostic::from_resolve_error(&err)
    }
}

impl From<Vec<crate::lower::ResolveError>> for Diagnostics {
    fn from(errors: Vec<crate::lower::ResolveError>) -> Self {
        let mut diagnostics = Diagnostics::default();
        for err in errors {
            diagnostics.push(Diagnostic::from_resolve_error(&err));
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests;

/* // Old
impl From<crate::parser::ParseError> for Diagnostics {
    fn from(err: crate::parser::ParseError) -> Self {
        let mut diagnostics = Diagnostics::default();

        diagnostics.push(Diagnostic::from(err));

        diagnostics
    }
}

impl From<crate::parser::ParseError> for Diagnostic {
    fn from(err: crate::parser::ParseError) -> Self {
        match err {
            crate::parser::ParseError::UnexpectedToken(got, expected) => Diagnostic {
                message: format!("Unexpected token: {:?}", got),
                labels: vec![(format!("Expected {:?}", expected), got.span.clone())],
                span: got.span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::UnexpectedKeyword(token, expected) => Diagnostic {
                message: format!("Unexpected keyword: {:?}", token.token_type,),
                labels: vec![(
                    format!("Expected one of {:?}", expected),
                    token.span.clone(),
                )],
                span: token.span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::Lexer(LexerError::UnknownToken(c, span)) => Diagnostic {
                message: format!("Lexer: Unknown token: {:?}", c),
                labels: vec![],
                span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::MacroNoCorrespondance {
                macro_name,
                invoc_name,
                invoc_arg,
            } => {
                let mut labels = vec![
                    (format!("For this macro"), macro_name.clone()),
                    (format!("In this macro invocation"), invoc_name.clone()),
                ];

                if let Some(invoc_arg) = invoc_arg {
                    labels.push((format!("With this token"), invoc_arg.clone()));
                }

                Diagnostic {
                    message: format!("Macro: Nothing expected this token"),
                    labels,
                    span: macro_name.clone(),
                    kind: DiagnosticType::Error,
                }
            }
            crate::parser::ParseError::InvalidPrecedence(precedence, token) => Diagnostic {
                message: format!("Invalid precedence: {:?}", precedence),
                labels: vec![(
                    format!("Precedence must be between 0 and 9"),
                    token.span.clone(),
                )],
                span: token.span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::IndentMismatch(got, expected, span) => Diagnostic {
                message: format!("Indent mismatch: got {}, expected {}", got, expected),
                labels: vec![
                    (format!("Expected indent level: {}", expected), span.clone()),
                    (format!("Got indent level: {}", got), span.clone()),
                ],
                span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::UnexpectedEof(_token) => Diagnostic {
                message: format!("Unexpected end of file"),
                labels: vec![],
                span: Span::test(),
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::LeftoverTokens(tokens) => Diagnostic {
                message: format!("Leftover tokens"),
                labels: vec![(format!("Expected end of file"), tokens[0].span.clone())],
                span: tokens[0].span.clone(),
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::UnknownFile(file) => Diagnostic {
                message: format!("Unknown file: {:?}", file),
                labels: vec![],
                span: Span::test(),
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::InternalError(message) => Diagnostic {
                message: format!("Internal error: {:?}", message),
                labels: vec![],
                span: Span::test(),
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::ShortCircuit => Diagnostic {
                message: format!("Short circuit, should never be printed !"),
                labels: vec![],
                span: Span::test(),
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::ExpectedType(span) => Diagnostic {
                message: format!("Expected a type"),
                labels: vec![(format!("But got"), span.clone())],
                span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::InvalidLHS(span) => Diagnostic {
                message: format!("Malformed assignment"),
                labels: vec![(format!("Invalid left-hand side"), span.clone())],
                span,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::InvalidVariant(enum_name, variant) => Diagnostic {
                message: format!("Invalid variant"),
                labels: vec![
                    (format!("In this enum"), enum_name.clone()),
                    (format!("Expected a variant"), variant.clone()),
                ],
                span: enum_name,
                kind: DiagnosticType::Error,
            },
            crate::parser::ParseError::InvalidType(span) => Diagnostic {
                message: format!("Invalid type"),
                labels: vec![(format!("Expected a type"), span.clone())],
                span,
                kind: DiagnosticType::Error,
            },
        }
    }
} */
