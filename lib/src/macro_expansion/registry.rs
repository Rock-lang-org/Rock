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
mod tests {
    use super::*;

    use crate::ast::{Ident, MacroDecl, Module, TopLevel};
    use crate::lexer::Span;

    fn macro_decl(name: &str) -> MacroDecl {
        MacroDecl {
            name: Ident {
                name: name.to_string(),
                span: Span::test(),
            },
            entries: Vec::new(),
            exported: false,
        }
    }

    #[test]
    fn registry_discovers_current_module_declarative_macros() {
        let module = Module {
            name: None,
            top_levels: vec![TopLevel::MacroDecl(macro_decl("make"))],
            is_inline: true,
            filepath: None,
        };

        let registry = MacroRegistry::from_module(&module);

        assert!(registry.declarative("make").is_some());
    }

    #[test]
    fn registry_records_function_like_proc_macro_exports() {
        let artifact = crate::macro_expansion::proc_macro::ProcMacroArtifact {
            artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
            crate_identity: "macros".to_string(),
            protocol_version: crate::macro_expansion::proc_macro::PROC_MACRO_PROTOCOL_VERSION,
            host_triple: "x86_64-unknown-linux-gnu".to_string(),
            executable: "macro-host".into(),
            capabilities: vec![crate::macro_expansion::proc_macro::ProcMacroCapability::Stdio],
            exports: vec![crate::macro_expansion::proc_macro::ProcMacroExport {
                name: "make_main".to_string(),
                identity: "macros::make_main".to_string(),
                kind: crate::macro_expansion::proc_macro::ProcMacroKind::FunctionLike,
                input_shape: crate::macro_expansion::proc_macro::ProcMacroInputShape::TokenStream,
            }],
        };
        let mut registry = MacroRegistry::new();

        registry.add_proc_macro_artifact(artifact);

        assert!(registry.proc_macro("make_main").is_some());
    }
}
