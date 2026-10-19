use std::cell::RefCell;

use crate::{
    diagnostic::{Diagnostic, DiagnosticLocation, Diagnostics},
    lexer::Span,
    macro_expansion::proc_macro::ProcMacroArtifact,
    Config,
};

use super::{ExpansionId, GeneratedSourceId, MacroExpansionRecord, MacroSourceMap};

#[derive(Debug)]
pub struct MacroExpansionContext<'a> {
    pub config: &'a Config,
    pub max_depth: usize,
    pub proc_macro_artifacts: Vec<ProcMacroArtifact>,
    source_map: RefCell<MacroSourceMap>,
    latest_expansion: RefCell<Option<ExpansionId>>,
}

impl<'a> MacroExpansionContext<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self {
            config,
            max_depth: 100,
            proc_macro_artifacts: Vec::new(),
            source_map: RefCell::new(MacroSourceMap::new()),
            latest_expansion: RefCell::new(None),
        }
    }

    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    pub fn with_proc_macro_artifact(mut self, artifact: ProcMacroArtifact) -> Self {
        self.proc_macro_artifacts.push(artifact);
        self
    }

    pub fn depth_exceeded_diagnostic(&self, span: Span) -> Diagnostic {
        Diagnostic::new("Macro expansion depth exceeded".to_string(), span)
            .with_code(crate::diagnostic::DiagnosticCode::Macro)
    }

    pub fn record_expansion(&self, record: MacroExpansionRecord) -> ExpansionId {
        let expansion_id = self.source_map.borrow_mut().record_expansion(record);
        *self.latest_expansion.borrow_mut() = Some(expansion_id);
        expansion_id
    }

    pub fn latest_expansion_id(&self) -> Option<ExpansionId> {
        *self.latest_expansion.borrow()
    }

    pub fn generated_source_id(&self, id: ExpansionId) -> Option<GeneratedSourceId> {
        self.source_map.borrow().generated_source_id(id)
    }

    pub fn trace_labels(&self, id: ExpansionId) -> Vec<(String, Span)> {
        self.source_map.borrow().trace_labels(id)
    }

    /// Generated parser spans use a virtual path when the macro host does not
    /// provide source locations. Point those diagnostics at the invocation so
    /// the compilation source database can attach the real source snapshot.
    pub fn map_generated_diagnostics(&self, diagnostics: &mut Diagnostics, id: ExpansionId) {
        let Some(invocation_span) = self
            .source_map
            .borrow()
            .record(id)
            .map(|record| record.invocation_span.clone())
        else {
            return;
        };
        for diagnostic in &mut diagnostics.0 {
            let DiagnosticLocation::Source(span) = &diagnostic.location else {
                continue;
            };
            if span
                .file_path
                .to_string_lossy()
                .starts_with("<macro-expansion:")
            {
                diagnostic.location = DiagnosticLocation::Source(invocation_span.clone());
                if let Some(primary) = &mut diagnostic.primary {
                    primary.span = invocation_span.clone();
                }
                for label in &mut diagnostic.secondary {
                    if label
                        .span
                        .file_path
                        .to_string_lossy()
                        .starts_with("<macro-expansion:")
                    {
                        label.span = invocation_span.clone();
                    }
                }
            }
        }
    }

}
