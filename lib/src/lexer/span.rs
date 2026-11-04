use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub file_path: PathBuf,
    /// Zero-based byte offset into the source text.
    pub start: usize,
    /// Exclusive byte offset into the source text.
    pub end: usize,
}

impl Span {
    pub fn new(file_path: PathBuf, start: usize, end: usize) -> Self {
        assert!(
            !file_path.as_os_str().is_empty(),
            "span requires a source path"
        );
        assert!(start <= end, "span start must not exceed its end");
        Self {
            file_path,
            start,
            end,
        }
    }
}

#[cfg(test)]
mod tests;
