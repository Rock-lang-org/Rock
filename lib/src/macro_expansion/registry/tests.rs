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
