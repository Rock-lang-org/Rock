//! Macros share the frontend's lossless source substrate. Rock tokens and groups
//! are optional interpretations, never a second stored or wire representation.

pub use crate::lexer::lossless::LosslessStream as TokenStream;
use crate::lexer::lossless::{JoinBoundary, ScanLimits};

pub const MACRO_LIMITS: ScanLimits = ScanLimits {
    max_source_bytes: 16 * 1024 * 1024,
    max_runs: 1024 * 1024,
};

pub(crate) fn append(output: &mut TokenStream, input: &TokenStream) -> Result<(), String> {
    // Restore only provable snapshot adjacency. A splice is otherwise a lexical
    // boundary even when its displayed characters happen to touch.
    *output = output.join(input, JoinBoundary::SourceAdjacent, MACRO_LIMITS)
        .or_else(|_| output.join(input, JoinBoundary::Separate, MACRO_LIMITS))
        .map_err(|error| format!("macro stream join failed: {error:?}"))?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn source(text: &str) -> TokenStream {
    TokenStream::from_bytes("/virtual/macro-test.rk".into(), text.as_bytes().to_vec(), MACRO_LIMITS).unwrap()
}
