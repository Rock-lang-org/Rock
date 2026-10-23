use std::cell::RefCell;

use crate::{
    diagnostic::{Diagnostic, DiagnosticLocation, Diagnostics},
    lexer::Span,
    macro_expansion::proc_macro::ProcMacroArtifact,
    Config,
};

use super::{ExpansionId, GeneratedSourceId, MacroExpansionRecord, MacroSourceMap};

#[derive(Debug, Clone)]
pub struct ProcMacroDependency {
    pub name: String,
    pub artifact: ProcMacroArtifact,
}

#[derive(Debug)]
pub struct MacroExpansionContext<'a> {
    pub(crate) cancellation: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) reports: RefCell<Diagnostics>,
    pub(super) metadata_only_derives: bool,
    consumed_derives: RefCell<std::collections::HashSet<crate::ast::NodeId>>,
    pub config: &'a Config,
    pub max_depth: usize,
    pub proc_macro_artifacts: Vec<ProcMacroDependency>,
    runtime_dependencies: Vec<crate::products::ProductCrateIdentity>,
    pub source_modules: crate::source_loader::SourceModuleSet,
    pub(crate) binding_environments: RefCell<super::binding_environment::BindingEnvironments>,
    pub(super) hygiene: RefCell<super::hygiene::Hygiene>,
    source_map: RefCell<MacroSourceMap>,
    latest_expansion: RefCell<Option<ExpansionId>>,
    budget: RefCell<super::budget::Budget>,
}

impl<'a> MacroExpansionContext<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self {
            cancellation: None,
            reports: RefCell::new(Diagnostics::default()),
            metadata_only_derives: false,
            consumed_derives: RefCell::new(std::collections::HashSet::new()),
            config,
            max_depth: 100,
            proc_macro_artifacts: Vec::new(),
            runtime_dependencies: Vec::new(),
            source_modules: crate::source_loader::SourceModuleSet::default(),
            binding_environments: RefCell::new(super::binding_environment::BindingEnvironments::default()),
            hygiene: RefCell::new(super::hygiene::Hygiene::default()),
            source_map: RefCell::new(MacroSourceMap::for_config(config)),
            latest_expansion: RefCell::new(None),
            budget: RefCell::new(super::budget::Budget::default()),
        }
    }

    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    pub(crate) fn with_derive_discovery(mut self) -> Self {
        self.metadata_only_derives = true;
        self
    }

    pub(super) fn consume_derive(&self, node: crate::ast::NodeId, span: &Span) -> Result<bool, Diagnostics> {
        let mut consumed = self.consumed_derives.borrow_mut();
        if consumed.contains(&node) { return Ok(false); }
        if consumed.len() >= self.budget.borrow().limits.max_invocations {
            return Err(super::proc_macro_error("macro invocation budget exceeded", span.clone()));
        }
        consumed.insert(node);
        Ok(true)
    }

    /// Only explicitly supplied consumer-side ordinary products grant runtime authority.
    pub fn with_runtime_dependency_identities(mut self, dependencies: Vec<crate::products::ProductCrateIdentity>) -> Self {
        self.runtime_dependencies = dependencies;
        self
    }

    pub(super) fn runtime_dependency(&self, required: &crate::products::ProductCrateIdentity) -> Result<crate::products::ProductCrateIdentity, String> {
        let mut matches = self.runtime_dependencies.iter().filter(|identity|
            identity.name == required.name && identity.version == required.version && identity.format_version == required.format_version);
        let identity = matches.next().ok_or_else(|| format!(
            "required runtime dependency {}@{} was not explicitly supplied by the consumer", required.name, required.version))?;
        if matches.next().is_some() {
            return Err(format!("required runtime dependency {}@{} has ambiguous consumer identities", required.name, required.version));
        }
        Ok(identity.clone())
    }

    pub fn with_limits(self, limits: super::ExpansionLimits) -> Self {
        self.budget.borrow_mut().limits = limits;
        self
    }

    pub(super) fn begin_expansion(&self, record: MacroExpansionRecord) -> Result<ExpansionId, Diagnostics> {
        let failure = |message: String| {
            let diagnostics = super::proc_macro_error(message, record.invocation_span.clone());
            record.parent.map(|parent| super::with_expansion_trace(diagnostics.clone(), self, parent)).unwrap_or(diagnostics)
        };
        let mut depth = 1usize;
        let mut parent = record.parent;
        let map = self.source_map.borrow();
        let mut visited = std::collections::HashSet::new();
        while let Some(id) = parent {
            if !visited.insert(id) { return Err(failure("invalid cyclic macro invocation ancestry".into())); }
            let Some(ancestor) = map.record(id) else { return Err(failure("unknown macro expansion parent".into())); };
            depth = depth.checked_add(1).ok_or_else(|| failure("macro ancestry depth overflow".into()))?;
            parent = ancestor.parent;
        }
        drop(map);
        if depth > self.max_depth { return Err(failure("Macro expansion depth exceeded".into())); }
        let budget = self.budget.borrow();
        if budget.executed.contains_key(&record.invocation) { return Err(failure("macro invocation ID already executed".into())); }
        if budget.executed.len() >= budget.limits.max_invocations || budget.executed.len() >= u32::MAX as usize {
            return Err(failure("macro invocation budget exceeded".into()));
        }
        let invocation = record.invocation;
        drop(budget);
        let expansion = self.record_expansion(record);
        self.budget.borrow_mut().executed.insert(invocation, expansion);
        Ok(expansion)
    }

    pub(super) fn note_source_modules(&self, items: &[crate::ast::TopLevel], scope: &[String], invocation: crate::ast::InvocationId) {
        let Some(parent) = self.budget.borrow().executed.get(&invocation).copied() else { return; };
        self.note_source_module_items(items, scope, parent);
    }

    fn note_source_module_items(&self, items: &[crate::ast::TopLevel], scope: &[String], parent: ExpansionId) {
        for item in items {
            match item {
                crate::ast::TopLevel::Mod(name, _) => {
                    let mut path = scope.to_vec(); path.push(name.name.clone());
                    self.budget.borrow_mut().source_parents.entry(path).or_insert(parent);
                }
                crate::ast::TopLevel::Module(child) => {
                    let mut path = scope.to_vec();
                    if let Some(name) = &child.0.name { path.push(name.name.clone()); }
                    self.budget.borrow_mut().source_parents.entry(path.clone()).or_insert(parent);
                    self.note_source_module_items(&child.0.top_levels, &path, parent);
                }
                _ => {}
            }
        }
    }

    pub(super) fn source_budget_diagnostic(&self, path: &std::path::Path, qualified: Option<&str>, message: String) -> Diagnostic {
        let mut diagnostic = Diagnostic::new(message, Span::new(path.into(), 0, 0))
            .with_code(crate::diagnostic::DiagnosticCode::Macro);
        let mut scope: Vec<_> = qualified.unwrap_or("").split("::").filter(|name| !name.is_empty()).map(String::from).collect();
        if scope.first().map(String::as_str) == self.config.current_crate_name.as_deref() && !scope.is_empty() { scope.remove(0); }
        loop {
            if let Some(parent) = self.budget.borrow().source_parents.get(&scope).copied() {
                diagnostic = diagnostic.with_labels(self.trace_labels(parent));
                break;
            }
            if scope.pop().is_none() { break; }
        }
        diagnostic
    }

    pub(super) fn charge_matching(&self, amount: usize, span: &Span) -> Result<(), Diagnostics> {
        let mut budget = self.budget.borrow_mut();
        let limit = budget.limits.max_matching_work;
        super::budget::charge(&mut budget.matching_work, amount, limit, "matching work")
            .map_err(|message| super::proc_macro_error(message, span.clone()))
    }

    pub(super) fn remaining_matching_work(&self) -> usize {
        let budget = self.budget.borrow();
        budget.limits.max_matching_work.saturating_sub(budget.matching_work)
    }

    pub(super) fn parse_probe<T>(&self, input: &super::TokenStream,
        parse: impl FnOnce() -> Result<T, crate::parser::ParseError>) -> Result<Option<T>, Diagnostics> {
        let work = crate::parser::ParseWorkScope::new(self.remaining_matching_work()
            .min(crate::parser::MAX_PARSE_WORK));
        let result = parse();
        self.charge_matching(work.used(), &input.start_span())?;
        // Grammar mismatches may backtrack; resource failures are always fatal.
        work.finish(Ok(())).map_err(Diagnostics::from)?;
        Ok(result.ok())
    }

    pub(super) fn charge_output(&self, stream: &super::TokenStream) -> Result<(), Diagnostics> {
        let mut budget = self.budget.borrow_mut();
        let bytes = budget.limits.max_output_bytes;
        let units = budget.limits.max_output_units;
        super::budget::charge(&mut budget.output_bytes, stream.len_bytes(), bytes, "output byte")
            .and_then(|_| super::budget::charge(&mut budget.output_units, stream.runs().len(), units, "output unit"))
            .map_err(|message| super::proc_macro_error(message, stream.start_span()))
    }

    pub(super) fn charge_source(&self, bytes: Option<usize>) -> Result<(), String> {
        let mut budget = self.budget.borrow_mut();
        match bytes {
            None => {
                let limit = budget.limits.max_source_discovery_work;
                super::budget::charge(&mut budget.source_work, 1, limit, "source discovery")
            }
            Some(bytes) => {
                let limit = budget.limits.max_source_bytes;
                super::budget::charge(&mut budget.source_bytes, bytes, limit, "source byte")
            }
        }
    }

    pub fn with_proc_macro_artifact(self, artifact: ProcMacroArtifact) -> Self {
        self.with_named_proc_macro_artifact(artifact.crate_identity.clone(), artifact)
    }

    pub fn with_named_proc_macro_artifact(mut self, name: String, artifact: ProcMacroArtifact) -> Self {
        self.proc_macro_artifacts.push(ProcMacroDependency { name, artifact });
        self
    }

    pub fn with_source_modules(mut self, modules: crate::source_loader::SourceModuleSet) -> Self {
        self.source_modules = modules;
        self
    }

    /// Cooperatively cancel native workers without leaving IO or descendants
    /// behind; this flag is host control, never data supplied by a worker.
    pub fn with_cancellation(mut self, cancellation: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.cancellation = Some(cancellation);
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

    pub fn source_map(&self) -> MacroSourceMap { self.source_map.borrow().clone() }

    pub(super) fn register_literal_source(&self, stream: &super::TokenStream) {
        self.source_map.borrow_mut().register_literal_source(stream);
    }

    pub(super) fn project_output(&self, stream: &super::TokenStream, id: ExpansionId) -> super::TokenStream {
        self.source_map.borrow_mut().record_output(id, stream);
        stream.project_output(id, self.generated_source_id(id).unwrap_or(GeneratedSourceId(0)), self.source_map.borrow().namespace())
    }

    /// Generated parser spans use a virtual path when the macro host does not
    /// provide source locations. Point those diagnostics at the invocation so
    /// the compilation source database can attach the real source snapshot.
    pub fn map_generated_diagnostics(&self, diagnostics: &mut Diagnostics, id: ExpansionId) {
        *diagnostics = self.source_map.borrow().map_diagnostics(std::mem::take(diagnostics));
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
                .starts_with("<macro-expansion:") || span.file_path.to_string_lossy().starts_with("<macro-output:")
            {
                diagnostic.location = DiagnosticLocation::Source(invocation_span.clone());
                diagnostic.source = None;
                if let Some(primary) = &mut diagnostic.primary {
                    primary.span = invocation_span.clone();
                }
                for label in &mut diagnostic.secondary {
                    if label
                        .span
                        .file_path
                        .to_string_lossy()
                        .starts_with("<macro-expansion:") || label.span.file_path.to_string_lossy().starts_with("<macro-output:")
                    {
                        label.span = invocation_span.clone();
                    }
                }
            }
        }
    }

}
