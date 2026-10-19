//! Test-only token fixture adapter; no independent source scanner.

use std::path::PathBuf;

use super::interpretation::layout::LayoutView;
use super::interpretation::RockError;
use super::lossless::{LosslessStream, ScanLimits};
use super::Token;

pub struct Lexer {
    stream: LosslessStream,
    terminal_layout: bool,
}

impl Lexer {
    pub fn new(path: PathBuf, source: &str) -> Result<Self, RockError> {
        let limits = ScanLimits { max_source_bytes: 16 * 1024 * 1024, max_runs: 1024 * 1024 };
        let stream = LosslessStream::from_bytes(path, source.as_bytes().to_vec(), limits)
            .map_err(RockError::Preservation)?;
        Ok(Self { stream, terminal_layout: true })
    }

    pub fn with_newline_at_end(mut self, enabled: bool) -> Self {
        self.terminal_layout = enabled;
        self
    }

    pub fn collect(&mut self) -> Result<Vec<Token>, RockError> {
        let mut view = LayoutView::new(&self.stream, self.terminal_layout);
        let mut tokens = Vec::new();
        while let Some(token) = view.next_token()? {
            tokens.push(token.token);
        }
        Ok(tokens)
    }
}
