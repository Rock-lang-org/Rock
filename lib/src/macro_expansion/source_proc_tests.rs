use crate::ast::TopLevel;
use crate::crate_system::TargetRole;
use crate::{Config, SourceProvider};

use super::registry::{MacroDefinition, MacroRegistry};
use super::{expand_macros_with_context, MacroExpansionContext};

fn config(source: &str) -> Config {
    let entry_file = "/macro-b12/lib.rk".into();
    Config {
        entry_file,
        current_crate_name: Some("provider".into()), target_role: TargetRole::ProcMacro,
        no_std: true, no_prelude: true,
        source_providers: vec![SourceProvider::Virtual { path: "/macro-b12/lib.rk".into(), text: source.into() }],
        ..Config::default()
    }
}

#[test]
fn macro_b_12_parses_proc_entries_beside_declarative_definitions() {
    let source = "macro private = context, input -> input\n< macro visible = context, input ->\n    input\nmacro ordinary\n    $value:expr =>\n        $value\n";
    let program = crate::parser::parse_string(source, &Config::default()).unwrap();
    let TopLevel::ProcMacroDecl(private) = &program.module.top_levels[0] else { panic!("private entry") };
    let TopLevel::ProcMacroDecl(public) = &program.module.top_levels[1] else { panic!("public entry") };
    assert_eq!(private.function.name.name, "private");
    assert_eq!(private.function.lambda.parameters.len(), 2);
    assert!(!private.function.exported);
    assert!(public.function.exported);
    assert_ne!(private.id, public.id);
    assert!(matches!(&program.module.top_levels[2], TopLevel::MacroDecl(_)));
}

#[test]
fn macro_b_12_derive_declarations_and_names_are_reserved() {
    for source in [
        "macro derive = context, input -> input\n",
        "macro derive\n    =>\n        1\n",
    ] {
        let error = crate::parser::parse_string(source, &Config::default()).unwrap_err();
        assert!(format!("{error:?}").contains("reserved for derive"), "{source}: {error:?}");
    }
}

#[test]
fn macro_b_12_discovery_uses_only_explicit_macro_exports() {
    let source = "helper = context, input -> input\n< runtime = -> 7\nmacro hidden = context, input -> input\n< macro visible = context, input -> input\nmacro separate = context, input -> input\n< separate\n";
    let expanded = crate::discover_proc_macros(&config(source)).unwrap();
    assert_eq!(expanded.proc_exports.iter().map(|entry| entry.export.name.as_str()).collect::<Vec<_>>(), ["separate", "visible"]);
    for entry in &expanded.proc_exports {
        assert_eq!(entry.export.identity, format!("provider::{}", entry.export.name));
        assert_eq!(entry.export.kind, super::proc_macro::ProcMacroKind::FunctionLike);
        assert_eq!(entry.export.input_shape, super::proc_macro::ProcMacroInputShape::TokenStream);
        assert!(entry.module.is_empty());
        assert!(expanded.ast.module.top_levels.iter().any(|item| matches!(item,
            TopLevel::ProcMacroDecl(declaration) if declaration.id == entry.definition)));
    }
}

#[test]
fn macro_b_12_discovery_expands_source_macros_before_reading_exports() {
    let source = "macro publish\n    $name:ident =>\n        macro $name = context, input -> input\n        < $name\n%publish identity\n";
    let expanded = crate::discover_proc_macros(&config(source)).unwrap();
    assert_eq!(expanded.proc_exports.len(), 1);
    assert_eq!(expanded.proc_exports[0].export.name, "identity");
    assert!(!expanded.ast.module.has_macro_invoc());
    assert_eq!(expanded.source_map.record(super::ExpansionId(1)).unwrap().macro_name, "publish");
    assert!(expanded.source_map.record(super::ExpansionId(2)).is_none());
}

#[test]
fn macro_b_12_file_module_exports_require_public_module_and_member() {
    let mut config = config("< mod public\nmod private\n> public::visible\n");
    for (name, source) in [
        ("public", "< macro visible = context, input -> input\nmacro private = context, input -> input\n< ordinary = -> 1\n"),
        ("private", "< macro hidden = context, input -> input\n"),
    ] {
        config.source_providers.push(SourceProvider::Virtual { path: format!("/macro-b12/{name}.rk").into(), text: source.into() });
    }
    let expanded = crate::discover_proc_macros(&config).unwrap();
    assert_eq!(expanded.proc_exports.len(), 1);
    let entry = &expanded.proc_exports[0];
    assert_eq!(entry.export.name, "public::visible");
    assert_eq!(entry.export.identity, "provider::public::visible");
    assert_eq!(entry.module.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["public"]);
}

#[test]
fn macro_b_12_duplicate_macro_names_reject_in_both_orders() {
    for source in [
        "macro make = context, input -> input\nmacro make = context, input -> input\n",
        "macro make\n    =>\n        1\nmacro make = context, input -> input\n",
        "macro make = context, input -> input\nmacro make\n    =>\n        1\n",
    ] {
        let errors = crate::discover_proc_macros(&config(source)).unwrap_err();
        assert!(format!("{errors:?}").contains("conflicting macro declaration"), "{errors:?}");
    }
}

#[test]
fn macro_b_12_proc_declarations_reject_on_runtime_targets_including_generated_items() {
    for source in [
        "< macro make = context, input -> input\n",
        "macro make = context, input -> input\n",
        "macro publish\n    =>\n        macro make = context, input -> input\n%publish\n",
    ] {
        for role in [TargetRole::Binary, TargetRole::Library] {
            let mut config = config(source); config.target_role = role;
            let errors = crate::discover_proc_macros(&config).unwrap_err();
            assert!(format!("{errors:?}").contains("require a [lib] with proc_macro = true"), "{errors:?}");
            assert!(errors.0.iter().all(|error| error.code == Some(crate::diagnostic::DiagnosticCode::Macro)));
        }
    }
}

#[test]
fn macro_b_12_source_entries_cannot_self_execute() {
    for use_site in ["%make\n", "helper = -> %make!\n"] {
        let source = format!("macro make = context, input -> input\n{use_site}");
        let errors = crate::discover_proc_macros(&config(&source)).unwrap_err();
        assert!(format!("{errors:?}").contains("cannot execute while building its own provider"), "{errors:?}");
    }
}

#[test]
fn macro_b_12_metadata_resolution_never_executes_a_provider() {
    let config = config("< macro local = context, input -> input\n> dependency::remote\n");
    let program = crate::parser::parse_string("< macro local = context, input -> input\n> dependency::remote\n", &config).unwrap();
    let artifact = super::proc_macro::ProcMacroArtifact {
        artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "dependency".into(), protocol_version: super::proc_macro::PROC_MACRO_PROTOCOL_VERSION,
        host_triple: "x86_64-unknown-linux-gnu".into(), executable: "/must-not-execute-during-discovery".into(),
        compiler_version: env!("CARGO_PKG_VERSION").into(), executable_identity: Default::default(), required_files: Vec::new(),
        capabilities: Vec::new(), definition_dependencies: Vec::new(),
        exports: vec![super::proc_macro::ProcMacroExport { name: "remote".into(), identity: "dependency::remote".into(),
            kind: super::proc_macro::ProcMacroKind::FunctionLike, input_shape: super::proc_macro::ProcMacroInputShape::TokenStream }],
    };
    let context = MacroExpansionContext::new(&config).with_proc_macro_artifact(artifact);
    let program = expand_macros_with_context(program, &context).unwrap();
    let registry = MacroRegistry::build(&program.module, &context).unwrap();
    assert!(matches!(&registry.resolve(&[], "remote", &crate::lexer::Span::test()).unwrap().unwrap().definition,
        MacroDefinition::Procedural(_)));
    let exports = registry.source_proc_exports("provider").unwrap();
    assert_eq!(exports.len(), 1);
    assert_eq!(exports[0].export.name, "local");
}

#[test]
fn macro_b_12_facade_publication_stays_rejected() {
    let mut config = config("mod helpers\n< helpers::make\n");
    config.source_providers.push(SourceProvider::Virtual { path: "/macro-b12/helpers.rk".into(),
        text: "< macro make = context, input -> input\n".into() });
    let errors = crate::discover_proc_macros(&config).unwrap_err();
    assert!(format!("{errors:?}").contains("facade publication is not implemented"), "{errors:?}");
}

#[test]
fn macro_b_12_runtime_compilation_cannot_drop_proc_implementations() {
    let errors = crate::compile(&config("< macro identity = context, input -> input\n")).unwrap_err();
    assert!(format!("{errors:?}").contains("requires an explicitly supplied SDK contract"), "{errors:?}");
}

#[test]
fn macro_b_12_ordinary_function_names_remain_a_separate_namespace() {
    let config = config("< macro same = context, input -> input\nsame = -> 1\n");
    let expanded = crate::discover_proc_macros(&config).unwrap();
    assert_eq!(expanded.proc_exports.len(), 1);
    assert_eq!(expanded.proc_exports[0].export.name, "same");
}

#[test]
fn macro_b_12_default_crate_identity_matches_the_compiler_source_identity() {
    let mut config = config("< macro identity = context, input -> input\n");
    config.current_crate_name = None;
    let expanded = crate::discover_proc_macros(&config).unwrap();
    assert_eq!(expanded.proc_exports[0].export.identity, "lib::identity");
}
