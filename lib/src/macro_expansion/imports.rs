//! Source bindings consumed by macro expansion, retained for import lints.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::ast::{IdentOrType, Path};
use crate::lexer::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MacroImportId(usize);

/// A source import and whether expansion selected that binding.
#[derive(Debug, Clone)]
pub struct MacroImportUsage {
    pub written: String,
    pub span: Span,
    pub used: bool,
}

/// File and inline-module lineage, independent of a macro template's spans.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct MacroImportScope {
    pub file_path: PathBuf,
    pub inline_modules: Vec<usize>,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct ImportKey {
    scope: MacroImportScope,
    file_path: PathBuf,
    start: usize,
    end: usize,
    written: String,
}

#[derive(Debug, Default)]
pub(super) struct MacroImportUses {
    by_site: HashMap<ImportKey, MacroImportId>,
    sites: Vec<MacroImportUsage>,
}

impl MacroImportUses {
    pub(super) fn register(
        &mut self,
        scope: &MacroImportScope,
        path: &Path,
        written: String,
    ) -> Option<MacroImportId> {
        let segment_span = |segment: &IdentOrType| match segment {
            IdentOrType::Ident(ident) => ident.span.clone(),
            IdentOrType::Type(ty) => ty.span(),
        };
        let first = segment_span(path.segments().first()?);
        let last = segment_span(path.segments().last()?);
        // Generated paths may mix template and argument spans.
        let span = if first.file_path == last.file_path && first.start <= last.end {
            Span::new(first.file_path, first.start, last.end)
        } else {
            first
        };
        let key = ImportKey {
            scope: scope.clone(),
            file_path: span.file_path.to_path_buf(),
            start: span.start,
            end: span.end,
            written: written.clone(),
        };
        if let Some(id) = self.by_site.get(&key) {
            return Some(*id);
        }
        let id = MacroImportId(self.sites.len());
        self.sites.push(MacroImportUsage {
            written,
            span,
            used: false,
        });
        self.by_site.insert(key, id);
        Some(id)
    }

    pub(super) fn mark_used(&mut self, id: MacroImportId) {
        self.sites[id.0].used = true;
    }

    pub(super) fn snapshot(&self) -> Vec<MacroImportUsage> {
        self.sites.clone()
    }
}

#[cfg(test)]
mod tests;
