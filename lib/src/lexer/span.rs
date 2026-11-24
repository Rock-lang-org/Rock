use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// A source file path shared by every span of that file.
///
/// Spans are cloned throughout the compiler; an owned `PathBuf` made each
/// clone allocate. Equality, hashing, ordering, `Debug` and serialization are
/// those of the path itself.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourcePath(Arc<PathBuf>);

impl SourcePath {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(Arc::new(path.into()))
    }
}

impl Serialize for SourcePath {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SourcePath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PathBuf::deserialize(deserializer).map(Self::new)
    }
}

impl PartialEq<&Path> for SourcePath {
    fn eq(&self, other: &&Path) -> bool {
        self.0.as_path() == *other
    }
}

impl std::fmt::Debug for SourcePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::ops::Deref for SourcePath {
    type Target = PathBuf;

    fn deref(&self) -> &PathBuf {
        &self.0
    }
}

impl AsRef<Path> for SourcePath {
    fn as_ref(&self) -> &Path {
        self.0.as_path()
    }
}

impl std::borrow::Borrow<PathBuf> for SourcePath {
    fn borrow(&self) -> &PathBuf {
        &self.0
    }
}

impl std::borrow::Borrow<Path> for SourcePath {
    fn borrow(&self) -> &Path {
        self.0.as_path()
    }
}

impl From<PathBuf> for SourcePath {
    fn from(path: PathBuf) -> Self {
        Self::new(path)
    }
}

impl From<&str> for SourcePath {
    fn from(path: &str) -> Self {
        Self::new(path)
    }
}

impl From<String> for SourcePath {
    fn from(path: String) -> Self {
        Self::new(path)
    }
}

impl From<&Path> for SourcePath {
    fn from(path: &Path) -> Self {
        Self::new(path)
    }
}

impl From<&PathBuf> for SourcePath {
    fn from(path: &PathBuf) -> Self {
        Self::new(path.clone())
    }
}

impl From<SourcePath> for PathBuf {
    fn from(path: SourcePath) -> Self {
        Arc::unwrap_or_clone(path.0)
    }
}

impl PartialEq<PathBuf> for SourcePath {
    fn eq(&self, other: &PathBuf) -> bool {
        *self.0 == *other
    }
}

impl PartialEq<SourcePath> for PathBuf {
    fn eq(&self, other: &SourcePath) -> bool {
        *self == *other.0
    }
}

impl PartialEq<Path> for SourcePath {
    fn eq(&self, other: &Path) -> bool {
        self.0.as_path() == other
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    pub file_path: SourcePath,
    /// Zero-based byte offset into the source text.
    pub start: usize,
    /// Exclusive byte offset into the source text.
    pub end: usize,
}

impl Span {
    pub fn new(file_path: SourcePath, start: usize, end: usize) -> Self {
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
