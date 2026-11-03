use std::collections::HashMap;

use crate::ast::{MacroDecl, Module, TopLevel};
use crate::macro_expansion::declarative::DeclarativeMacro;
use crate::macro_expansion::proc_macro::{ProcMacroArtifact, ProcMacroExport};

#[derive(Debug, Clone)]
pub struct ProcMacroRegistryEntry {
    pub artifact: ProcMacroArtifact,
    pub export: ProcMacroExport,
}

#[derive(Debug, Clone, Default)]
pub struct MacroRegistry {
    declarative: HashMap<String, DeclarativeMacro>,
    declarations: HashMap<String, MacroDecl>,
    proc_macros: HashMap<String, ProcMacroRegistryEntry>,
}

impl MacroRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_module(module: &Module) -> Self {
        let mut registry = Self::new();
        registry.add_module_declarations(module);
        registry
    }

    /// Adds the macros declared in `module`, replacing same-named ones.
    pub fn add_module_declarations(&mut self, module: &Module) {
        for top_level in &module.top_levels {
            if let TopLevel::MacroDecl(decl) = top_level {
                self.insert_declaration(decl.name.name.clone(), decl.clone());
            }
        }
    }

    /// Makes `decl` callable as `%name`, replacing any macro of that name.
    pub fn insert_declaration(&mut self, name: String, decl: MacroDecl) {
        self.declarative
            .insert(name.clone(), DeclarativeMacro::from_ast(&decl));
        self.declarations.insert(name, decl);
    }

    pub fn declarative(&self, name: &str) -> Option<&DeclarativeMacro> {
        self.declarative.get(name)
    }

    pub(crate) fn declaration(&self, name: &str) -> Option<&MacroDecl> {
        self.declarations.get(name)
    }

    pub fn add_proc_macro_artifact(&mut self, artifact: ProcMacroArtifact) {
        for export in &artifact.exports {
            self.proc_macros.insert(
                export.name.clone(),
                ProcMacroRegistryEntry {
                    artifact: artifact.clone(),
                    export: export.clone(),
                },
            );
        }
    }

    pub fn proc_macro(&self, name: &str) -> Option<&ProcMacroRegistryEntry> {
        self.proc_macros.get(name)
    }
}

#[cfg(test)]
mod tests;
