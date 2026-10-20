//! Source preservation only; Rock syntax is an optional, separate interpretation.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::Span;

/// Storage hints, not Rock token kinds: quotes, comments and groups are just text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RunKind {
    Text,
    /// Unicode whitespace other than the physical line endings CR and LF.
    Whitespace,
    /// One LF, CR, or CRLF, including inside unterminated quotes/comments.
    LineEnding,
}

impl RunKind {
    fn of(character: char) -> Self {
        match character {
            '\r' | '\n' => Self::LineEnding,
            character if character.is_whitespace() => Self::Whitespace,
            _ => Self::Text,
        }
    }
}

/// Binding authority is independent of source locations. Ordinary source starts
/// in ROOT; only the later hygiene owner may allocate expansion contexts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SyntaxContext(u32);

impl SyntaxContext {
    pub const ROOT: Self = Self(0);
    pub(crate) fn allocated(id: u32) -> Self { Self(id) }
}

/// Worker-side symbolic requests are not compiler binding identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstructionContext {
    pub invocation: crate::ast::InvocationId,
    pub kind: ConstructionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConstructionKind {
    Fresh(u32),
    CallSite,
    Dependency(u32),
}

/// A splice is a lexical boundary even if its printed neighbours could touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JoinBoundary {
    Separate,
    /// Explicitly restore adjacency of contiguous slices of the same snapshot.
    SourceAdjacent,
}

/// Both retained spelling and run metadata count toward preservation budgets.
#[derive(Debug, Clone, Copy)]
pub struct ScanLimits {
    pub max_source_bytes: usize,
    pub max_runs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LosslessError {
    EmptySourcePath,
    SourceTooLarge { bytes: usize, limit: usize },
    InvalidUtf8 { valid_up_to: usize, error_len: Option<usize> },
    TooManyRuns { offset: usize, limit: usize },
    AllocationFailed { offset: usize },
    InvalidRange { start: usize, end: usize, len: usize },
    NotCharBoundary { offset: usize },
    NotSourceAdjacent,
    LengthOverflow,
    EmptyBindingRange,
    ConflictingContexts { offset: usize, left: SyntaxContext, right: SyntaxContext },
    InvalidExpressionFragment,
    UnresolvedConstruction,
    SnapshotIdentityExhausted,
}

#[derive(Debug, Serialize, Deserialize)]
struct SourceSnapshot {
    identity: u64,
    file_path: PathBuf,
    text: String,
}

impl SourceSnapshot {
    fn new(file_path: PathBuf, text: String) -> Result<Self, LosslessError> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let identity = NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |next| next.checked_add(1))
            .map_err(|_| LosslessError::SnapshotIdentityExhausted)?;
        Ok(Self { identity, file_path, text })
    }
}

/// An immutable UTF-8 byte range backed by a retained source, never decoded text.
#[derive(Debug, Clone)]
pub struct LosslessRun {
    source: Arc<SourceSnapshot>,
    kind: RunKind,
    range: std::ops::Range<usize>,
    context: SyntaxContext,
    boundary: JoinBoundary,
    origin: Option<crate::macro_expansion::TokenOrigin>,
    construction: Option<ConstructionContext>,
}

impl PartialEq for LosslessRun {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source) && self.kind == other.kind
            && self.range == other.range && self.context == other.context && self.boundary == other.boundary && self.origin == other.origin
            && self.construction == other.construction
    }
}

impl Eq for LosslessRun {}

impl LosslessRun {
    pub(crate) fn has_construction_request(&self) -> bool { self.construction.is_some() }

    pub fn origin(&self) -> crate::macro_expansion::TokenOrigin {
        self.origin.clone().unwrap_or_else(|| crate::macro_expansion::TokenOrigin::Source(self.span()))
    }

    pub fn kind(&self) -> RunKind {
        self.kind
    }

    pub fn range(&self) -> std::ops::Range<usize> {
        self.range.clone()
    }

    pub fn context(&self) -> SyntaxContext {
        self.context
    }

    pub fn boundary(&self) -> JoinBoundary {
        self.boundary
    }

    pub fn text(&self) -> &str {
        // Scanning and checked slicing are the only range constructors.
        &self.source.text[self.range.clone()]
    }

    pub fn span(&self) -> Span {
        let path = match &self.origin {
            Some(crate::macro_expansion::TokenOrigin::Generated { expansion, .. }) => format!("<macro-expansion:{}>", expansion.0).into(),
            Some(crate::macro_expansion::TokenOrigin::Captured { capture_span, .. }) => capture_span.file_path.clone(),
            Some(crate::macro_expansion::TokenOrigin::Source(span)) => span.file_path.clone(),
            _ => self.source.file_path.clone(),
        };
        Span::new(path, self.range.start, self.range.end)
    }
}

#[derive(Debug, Clone)]
struct SourcePosition {
    source: Arc<SourceSnapshot>,
    offset: usize,
    context: SyntaxContext,
    origin: Option<crate::macro_expansion::TokenOrigin>,
}

impl PartialEq for SourcePosition {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source) && self.offset == other.offset && self.context == other.context && self.origin == other.origin
    }
}

impl Eq for SourcePosition {}

/// Ordered source segments; logical byte offsets need not be source offsets.
/// Reconstruction is inspection only, never a syntax-forwarding mechanism.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LosslessStream {
    runs: Vec<LosslessRun>,
    len_bytes: usize,
    /// Retains a precise location/context even for an empty source or slice.
    anchor: SourcePosition,
    expressions: Vec<fragments::ExpressionRange>,
}

impl LosslessStream {
    /// Takes ownership of bytes without copying spelling or consulting the filesystem.
    pub fn from_bytes(
        file_path: PathBuf,
        bytes: Vec<u8>,
        limits: ScanLimits,
    ) -> Result<Self, LosslessError> {
        if file_path.as_os_str().is_empty() {
            return Err(LosslessError::EmptySourcePath);
        }
        if bytes.len() > limits.max_source_bytes {
            return Err(LosslessError::SourceTooLarge {
                bytes: bytes.len(),
                limit: limits.max_source_bytes,
            });
        }
        let text = String::from_utf8(bytes).map_err(|error| {
            let error = error.utf8_error();
            LosslessError::InvalidUtf8 {
                valid_up_to: error.valid_up_to(),
                error_len: error.error_len(),
            }
        })?;
        let source = Arc::new(SourceSnapshot::new(file_path, text)?);
        let mut runs = Vec::new();
        let mut characters = source.text.char_indices().peekable();

        // Every iteration consumes at least one scalar, without recursion/lookbehind.
        while let Some((start, character)) = characters.next() {
            if runs.len() == limits.max_runs {
                return Err(LosslessError::TooManyRuns {
                    offset: start,
                    limit: limits.max_runs,
                });
            }
            let kind = RunKind::of(character);
            let mut end = start + character.len_utf8();
            if kind == RunKind::LineEnding {
                if character == '\r' && characters.peek().map(|(_, ch)| *ch) == Some('\n') {
                    characters.next();
                    end += 1;
                }
            } else {
                while let Some(&(offset, next)) = characters.peek() {
                    if RunKind::of(next) != kind {
                        break;
                    }
                    characters.next();
                    end = offset + next.len_utf8();
                }
            }
            runs.try_reserve(1).map_err(|_| LosslessError::AllocationFailed {
                offset: start,
            })?;
            runs.push(LosslessRun {
                source: Arc::clone(&source),
                kind,
                range: start..end,
                context: SyntaxContext::ROOT,
                boundary: JoinBoundary::SourceAdjacent,
                origin: None,
                construction: None,
            });
        }

        Ok(Self {
            len_bytes: source.text.len(),
            anchor: SourcePosition { source, offset: 0, context: SyntaxContext::ROOT, origin: None },
            runs,
            expressions: Vec::new(),
        })
    }

    pub fn runs(&self) -> &[LosslessRun] {
        &self.runs
    }
}

mod splice;
mod wire;
mod origin;
mod fragments;
mod construction;

pub use fragments::ExpressionFragment;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod splice_tests;

#[cfg(test)]
mod wire_tests;

#[cfg(test)]
mod context_delivery_tests;

#[cfg(test)]
mod local_resolution_tests;

#[cfg(test)]
mod type_resolution_tests;
