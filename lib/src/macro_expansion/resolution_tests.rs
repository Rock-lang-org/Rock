use super::*;
use super::registry::{MacroIdentity, MacroRegistry};

fn module(text: &str) -> Module {
    crate::parser::parse_string(text, &crate::Config::default()).unwrap().module
}

fn child(root: &mut Module, name: &str, text: &str) {
    let mut inner = module(text);
    inner.name = Some(crate::ast::Ident { name: name.into(), span: Span::test() });
    root.top_levels.push(TopLevel::Module(crate::ast::ModuleDecl(inner)));
}

fn identity(registry: &MacroRegistry, scope: &[&str], path: &str) -> MacroIdentity {
    registry.resolve(&scope.iter().map(|name| name.to_string()).collect::<Vec<_>>(), path, &Span::test())
        .unwrap().unwrap().identity.clone()
}

fn artifact(crate_name: &str) -> proc_macro::ProcMacroArtifact {
    proc_macro::ProcMacroArtifact {
        artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: crate_name.into(),
        protocol_version: proc_macro::PROC_MACRO_PROTOCOL_VERSION,
        host_triple: "x86_64-unknown-linux-gnu".into(),
        executable: "/never-execute-during-resolution".into(),
        capabilities: vec![],
        exports: vec![proc_macro::ProcMacroExport {
            name: "make".into(), identity: format!("{crate_name}::definition-7"),
            kind: proc_macro::ProcMacroKind::FunctionLike,
            input_shape: proc_macro::ProcMacroInputShape::TokenStream,
        }],
    }
}

#[test]
fn macro_a_07_local_forward_references_and_value_names_use_separate_namespaces() {
    let config = crate::Config::default();
    let input = module("%make\nmake = -> 1\nmacro make\n    =>\n        main = -> 42\n");
    let expanded = expand_macros_with_context(Program { module: input }, &MacroExpansionContext::new(&config)).unwrap();
    assert!(matches!(expanded.module.top_level_from_ident("main"), Some(TopLevel::FunctionDecl(_))));
}

#[test]
fn macro_a_07_source_imports_aliases_and_qualified_paths_have_one_identity() {
    let config = crate::Config::default();
    let mut root = module("> outer::tools\n> outer::tools::make\n");
    let mut outer = module("< tools\n");
    child(&mut outer, "tools", "< make\nmacro make\n    =>\n        main = -> 42\n");
    outer.name = Some(crate::ast::Ident { name: "outer".into(), span: Span::test() });
    root.top_levels.push(TopLevel::Module(crate::ast::ModuleDecl(outer)));
    let registry = MacroRegistry::build(&root, &MacroExpansionContext::new(&config)).unwrap();
    assert_eq!(identity(&registry, &[], "make"), identity(&registry, &[], "tools::make"));
    assert_eq!(identity(&registry, &[], "make"), identity(&registry, &[], "outer::tools::make"));
}

#[test]
fn macro_a_07_same_spelled_source_definitions_do_not_leak_between_modules() {
    let config = crate::Config::default();
    let mut root = module("");
    child(&mut root, "one", "< make\nmacro make\n    =>\n        main = -> 1\n");
    child(&mut root, "two", "< make\nmacro make\n    =>\n        main = -> 2\n");
    let registry = MacroRegistry::build(&root, &MacroExpansionContext::new(&config)).unwrap();
    assert_ne!(identity(&registry, &["one"], "make"), identity(&registry, &["two"], "make"));
    assert!(registry.resolve(&[], "make", &Span::test()).unwrap().is_none());
    assert_eq!(identity(&registry, &["one"], "make"), identity(&registry, &[], "one::make"));
}

#[test]
fn macro_a_07_duplicate_declarations_and_conflicting_imports_reject_without_overwrite() {
    let config = crate::Config::default();
    let root = module("macro make\n    =>\n        1\nmacro make\n    =>\n        2\n");
    assert!(MacroRegistry::build(&root, &MacroExpansionContext::new(&config)).is_err());
    for reverse in [false, true] {
        let context = if reverse {
            MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact("two")).with_proc_macro_artifact(artifact("one"))
        } else {
            MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact("one")).with_proc_macro_artifact(artifact("two"))
        };
        let root = module("> one::make\n> two::make\n");
        let diagnostics = MacroRegistry::build(&root, &context).unwrap_err();
        assert!(format!("{diagnostics:?}").contains("conflict"));
    }
}

#[test]
fn macro_a_07_proc_artifacts_require_explicit_imports_or_dependency_qualified_paths() {
    let config = crate::Config::default();
    let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact("one")).with_proc_macro_artifact(artifact("two"));
    let registry = MacroRegistry::build(&module(""), &context).unwrap();
    assert!(registry.resolve(&[], "make", &Span::test()).unwrap().is_none());
    assert_ne!(identity(&registry, &[], "one::make"), identity(&registry, &[], "two::make"));
    let registry = MacroRegistry::build(&module("> two::make\n"), &context).unwrap();
    assert_eq!(identity(&registry, &[], "make"), identity(&registry, &[], "two::make"));
}

#[test]
fn macro_a_07_dependency_aliases_do_not_change_canonical_identity() {
    let config = crate::Config::default();
    let context = MacroExpansionContext::new(&config).with_named_proc_macro_artifact("renamed".into(), artifact("provider"));
    let registry = MacroRegistry::build(&module("> renamed::make\n"), &context).unwrap();
    assert_eq!(identity(&registry, &[], "make"), identity(&registry, &[], "renamed::make"));
    assert!(registry.resolve(&[], "provider::make", &Span::test()).unwrap().is_none());
}

#[test]
fn macro_a_07_private_macros_and_deferred_globs_or_facades_diagnose() {
    let config = crate::Config::default();
    for (export, import, call, expected) in [
        ("", "", "tools::make", "private"),
        ("< make\n", "> tools::*\n", "make", "glob"),
    ] {
        let mut root = module(import);
        child(&mut root, "tools", &format!("{export}macro make\n    =>\n        42\n"));
        let registry = MacroRegistry::build(&root, &MacroExpansionContext::new(&config)).unwrap();
        let diagnostics = registry.resolve(&[], call, &Span::test()).unwrap_err();
        assert!(format!("{diagnostics:?}").contains(expected), "{diagnostics:?}");
    }
    let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact("provider"));
    let mut root = module("");
    child(&mut root, "facade", "> provider::make\n< make\n");
    let registry = MacroRegistry::build(&root, &context).unwrap();
    let diagnostics = registry.resolve(&[], "facade::make", &Span::test()).unwrap_err();
    assert!(format!("{diagnostics:?}").contains("re-export"));
}

#[test]
fn macro_a_07_loaded_source_module_definitions_resolve_before_collection() {
    let config = crate::Config { current_crate_name: Some("app".into()), ..crate::Config::default() };
    let mut sources = crate::source_loader::SourceDatabase::new();
    sources.add_virtual_source("/macro-source/main.rk".into(), "mod tools\n> tools::make\n%make\n");
    sources.add_virtual_source("/macro-source/tools.rk".into(), "< make\nmacro make\n    =>\n        main = -> 42\n");
    let graph = sources.load_entry("/macro-source/main.rk".into(), &config).unwrap();
    let context = MacroExpansionContext::new(&config).with_source_modules(graph.source_modules());
    let expanded = expand_macros_with_context(Program { module: graph.root_module().clone() }, &context).unwrap();
    assert!(matches!(expanded.module.top_level_from_ident("main"), Some(TopLevel::FunctionDecl(_))));
}

#[test]
fn macro_a_07_a_declaration_and_explicit_proc_import_cannot_compete_by_kind() {
    let config = crate::Config::default();
    let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact("provider"));
    let diagnostics = MacroRegistry::build(&module("> provider::make\nmacro make\n    =>\n        42\n"), &context).unwrap_err();
    assert!(format!("{diagnostics:?}").contains("conflict"));
}

#[test]
fn macro_a_07_deferred_facade_imports_diagnose_at_the_macro_use() {
    let config = crate::Config::default();
    let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact("provider"));
    let mut root = module("> facade::make\n");
    child(&mut root, "facade", "> provider::make\n< make\n");
    let registry = MacroRegistry::build(&root, &context).unwrap();
    let diagnostics = registry.resolve(&[], "make", &Span::test()).unwrap_err();
    assert!(format!("{diagnostics:?}").contains("re-export"));
}

#[test]
fn macro_a_07_ordinary_value_reexports_and_globs_are_not_macro_bindings() {
    let config = crate::Config::default();
    let mut root = module("> facade::value\n> facade::*\n");
    child(&mut root, "facade", "< other::value\n");
    let registry = MacroRegistry::build(&root, &MacroExpansionContext::new(&config)).unwrap();
    assert!(registry.resolve(&[], "missing", &Span::test()).unwrap().is_none());
}

#[test]
fn macro_a_07_duplicate_proc_surfaces_and_cross_crate_declarative_attempts_reject() {
    let config = crate::Config::default();
    let mut provider = artifact("provider");
    provider.exports.push(provider.exports[0].clone());
    provider.exports[1].kind = proc_macro::ProcMacroKind::DeriveLike;
    assert!(MacroRegistry::build(&module(""), &MacroExpansionContext::new(&config).with_proc_macro_artifact(provider)).is_err());
    let config = crate::Config { extern_artifacts: vec![("source_only".into(), "unused.rkca".into())], ..config };
    let registry = MacroRegistry::build(&module(""), &MacroExpansionContext::new(&config)).unwrap();
    let diagnostics = registry.resolve(&[], "source_only::make", &Span::test()).unwrap_err();
    assert!(format!("{diagnostics:?}").contains("declarative macro publication"));
}

#[test]
fn macro_a_07_private_nested_modules_do_not_gain_visibility_from_macro_exports() {
    let config = crate::Config::default();
    let mut root = module("");
    let mut outer = module("");
    child(&mut outer, "hidden", "< make\nmacro make\n    =>\n        42\n");
    outer.name = Some(crate::ast::Ident { name: "outer".into(), span: Span::test() });
    root.top_levels.push(TopLevel::Module(crate::ast::ModuleDecl(outer)));
    let registry = MacroRegistry::build(&root, &MacroExpansionContext::new(&config)).unwrap();
    let diagnostics = registry.resolve(&[], "outer::hidden::make", &Span::test()).unwrap_err();
    assert!(format!("{diagnostics:?}").contains("private source module"));
    identity(&registry, &["outer"], "hidden::make");
}

#[test]
fn macro_a_07_explicit_proc_import_executes_the_selected_canonical_export() {
    let config = crate::Config::default();
    let (mut provider, directory) = super::tests::proc_macro_artifact_with_response("make", proc_macro::ProcMacroResponse::Expand {
        output: proc_macro::encode_stream(&super::token_tree::source("main = -> 42\n")).unwrap(),
    });
    provider.crate_identity = "canonical-provider".into();
    provider.exports[0].identity = "opaque-definition-identity".into();
    let request_file = provider.executable.with_extension("request");
    let context = MacroExpansionContext::new(&config).with_named_proc_macro_artifact("renamed".into(), provider);
    let result = expand_macros_with_context(Program { module: module("> renamed::make\n%make\n") }, &context);
    let request = proc_macro::decode_request(&std::fs::read(request_file).unwrap()).unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(result.unwrap().module.top_level_from_ident("main").is_some());
    assert_eq!(request.macro_identity, "opaque-definition-identity");
}

#[test]
fn macro_a_07_forward_module_aliases_and_nested_proc_surfaces_are_explicit() {
    let config = crate::Config::default();
    let mut root = module("> tools::nested\n> outer::tools\n");
    let mut outer = module("< tools\n");
    let mut tools = module("< nested\n");
    child(&mut tools, "nested", "< make\nmacro make\n    =>\n        42\n");
    tools.name = Some(crate::ast::Ident { name: "tools".into(), span: Span::test() });
    outer.top_levels.push(TopLevel::Module(crate::ast::ModuleDecl(tools)));
    outer.name = Some(crate::ast::Ident { name: "outer".into(), span: Span::test() });
    root.top_levels.push(TopLevel::Module(crate::ast::ModuleDecl(outer)));
    let registry = MacroRegistry::build(&root, &MacroExpansionContext::new(&config)).unwrap();
    assert_eq!(identity(&registry, &[], "nested::make"), identity(&registry, &[], "outer::tools::nested::make"));

    let mut provider = artifact("provider");
    provider.exports[0].name = "syntax::make".into();
    let context = MacroExpansionContext::new(&config).with_named_proc_macro_artifact("renamed".into(), provider);
    let registry = MacroRegistry::build(&module("> renamed::syntax\n> syntax::make\n"), &context).unwrap();
    assert_eq!(identity(&registry, &[], "make"), identity(&registry, &[], "renamed::syntax::make"));
    assert!(registry.resolve(&[], "provider::syntax::make", &Span::test()).unwrap().is_none());
}

#[test]
fn macro_a_07_unused_value_imports_do_not_attempt_macro_facade_resolution() {
    let config = crate::Config::default();
    let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact("provider"));
    let mut root = module("");
    child(&mut root, "facade", "> provider::make\n< make\nmake = -> 42\n");
    child(&mut root, "zconsumer", "> facade::make\n");
    // Both module orders must be inert until a use requests the macro namespace.
    for reverse in [false, true] {
        if reverse { root.top_levels.reverse(); }
        let registry = MacroRegistry::build(&root, &context).unwrap();
        let diagnostics = registry.resolve(&["zconsumer".into()], "make", &Span::test()).unwrap_err();
        assert!(format!("{diagnostics:?}").contains("re-export"));
    }
}
