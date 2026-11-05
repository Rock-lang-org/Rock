use std::path::PathBuf;

use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::lexer::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodegenErrorKind {
    /// Caused by the source operation being compiled; `with_operation_span`
    /// points it at that operation.
    SourceOperation,
    BackendContract,
    Layout,
    Output,
    Link,
    Llvm,
    Toolchain,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodegenErrorLocation {
    Source(Span),
    File(PathBuf),
    Toolchain,
}

/// A structured code generation failure.
#[derive(Debug, Clone)]
pub(crate) struct CodegenError {
    pub(super) message: String,
    kind: CodegenErrorKind,
    location: CodegenErrorLocation,
    notes: Vec<String>,
}

impl CodegenError {
    pub(super) fn new(message: String) -> Self {
        Self {
            message,
            kind: CodegenErrorKind::Llvm,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    /// A failure the program itself causes, such as an unsupported cast. The
    /// function compiler attaches the span of the statement or terminator.
    pub(super) fn source_operation(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::SourceOperation,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    pub(super) fn internal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::Internal,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    pub(super) fn backend_contract(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::BackendContract,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    pub(super) fn layout(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::Layout,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    pub(super) fn output(message: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::Output,
            location: CodegenErrorLocation::File(path.into()),
            notes: Vec::new(),
        }
    }

    pub(super) fn link(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::Link,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    pub(super) fn toolchain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: CodegenErrorKind::Toolchain,
            location: CodegenErrorLocation::Toolchain,
            notes: Vec::new(),
        }
    }

    pub(super) fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub(crate) fn internalize(mut self) -> Self {
        self.kind = CodegenErrorKind::Internal;
        self.location = CodegenErrorLocation::Toolchain;
        self
    }

    pub(super) fn with_operation_span(mut self, span: Option<Span>) -> Self {
        if matches!(self.kind, CodegenErrorKind::SourceOperation) {
            if let Some(span) = span {
                self.location = CodegenErrorLocation::Source(span);
            }
        }
        self
    }

    pub(crate) fn into_diagnostic(self) -> Diagnostic {
        let CodegenError {
            message: raw_message,
            kind,
            location,
            notes,
        } = self;
        let message = Self::user_message(&raw_message);
        let mut diagnostic = match location {
            CodegenErrorLocation::Source(span) => {
                Diagnostic::new(message, span).with_code(DiagnosticCode::Codegen)
            }
            CodegenErrorLocation::File(path) => {
                Diagnostic::for_file(message, path).with_code(DiagnosticCode::Codegen)
            }
            CodegenErrorLocation::Toolchain => match kind {
                CodegenErrorKind::Internal
                | CodegenErrorKind::BackendContract
                | CodegenErrorKind::Layout => Diagnostic::for_internal(message),
                CodegenErrorKind::Output => {
                    Diagnostic::for_toolchain(message).with_code(DiagnosticCode::Codegen)
                }
                CodegenErrorKind::Link | CodegenErrorKind::Toolchain => {
                    Diagnostic::for_toolchain(message)
                }
                CodegenErrorKind::Llvm => {
                    Diagnostic::for_toolchain("LLVM code generation failed".to_string())
                        .with_code(DiagnosticCode::Codegen)
                        .with_note(message)
                }
                CodegenErrorKind::SourceOperation => Diagnostic::for_internal(message),
            },
        };
        for note in notes {
            diagnostic = diagnostic.with_note(Self::user_message(&note));
        }
        diagnostic
    }

    fn user_message(message: &str) -> String {
        const INTERNAL_IDENTITIES: [&str; 21] = [
            "DefId(",
            "DefId {",
            "InstanceId(",
            "TypeId(",
            "TypeVarId(",
            "TypeVarId {",
            "GenericParamId(",
            "GenericParamId {",
            "FieldId(",
            "FieldId {",
            "VariantId(",
            "VariantId {",
            "Local(",
            "Local {",
            "HirLocalId(",
            "HirLocalId {",
            "struct#",
            "enum#",
            "generic#",
            "trait#",
            "?T",
        ];
        if INTERNAL_IDENTITIES
            .iter()
            .any(|identity| message.contains(identity))
        {
            "Internal compiler error during MIR code generation".to_string()
        } else {
            message.to_string()
        }
    }
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "codegen error: {}", self.message)
    }
}

impl From<String> for CodegenError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for CodegenError {
    fn from(message: &str) -> Self {
        Self::new(message.to_string())
    }
}
