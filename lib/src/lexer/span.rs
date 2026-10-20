use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

    #[cfg(test)]
    pub fn test() -> Self {
        Self::new(PathBuf::from("/test.rk"), 0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::Span;

    #[test]
    fn span_equality_compares_path_and_range() {
        assert_eq!(
            Span::new("/virtual/main.rk".into(), 1, 3),
            Span::new("/virtual/main.rk".into(), 1, 3)
        );
        assert_ne!(
            Span::new("/virtual/main.rk".into(), 1, 3),
            Span::new("/virtual/main.rk".into(), 2, 3)
        );
    }
}
