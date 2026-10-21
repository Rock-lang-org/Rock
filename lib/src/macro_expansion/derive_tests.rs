use crate::ast::TopLevel;
use crate::crate_system::TargetRole;
use crate::macro_expansion::proc_macro::*;
use crate::{Config, SourceProvider};

use super::{expand_macros_with_context, MacroExpansionContext};

fn config(source: &str) -> Config {
    Config { entry_file: "/macro-c02/lib.rk".into(), current_crate_name: Some("provider".into()),
        target_role: TargetRole::ProcMacro, no_std: true, no_prelude: true,
        source_providers: vec![SourceProvider::Virtual { path: "/macro-c02/lib.rk".into(), text: source.into() }], ..Default::default() }
}

fn descriptor(kind: ProcMacroKind) -> ProcMacroArtifact {
    ProcMacroArtifact { artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "dependency".into(), protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        host_triple: crate::sysroot::host_target_triple(), executable: "/must-not-run-c02-derive".into(),
        compiler_version: env!("CARGO_PKG_VERSION").into(), executable_identity: ProcMacroFileIdentity { bytes: 1, sha256: [0; 32] },
        required_files: Vec::new(), capabilities: vec![ProcMacroCapability::Stdio], definition_dependencies: Vec::new(),
        exports: vec![ProcMacroExport { name: "Show".into(), identity: "dependency::Show".into(), kind, input_shape: ProcMacroInputShape::TokenStream }] }
}

#[test]
fn macro_c_02_discovers_public_derive_metadata_without_running_bodies() {
    let source = "macro derive Hidden = context, input -> missing!\n< macro derive Show = context, input -> missing!\nmacro derive Eq = context, input -> missing!\n< Eq\n< macro ordinary = context, input -> input\n";
    let expanded = crate::discover_proc_macros(&config(source)).unwrap();
    assert_eq!(expanded.proc_exports.iter().map(|entry| (entry.export.name.as_str(), entry.export.kind.clone())).collect::<Vec<_>>(),
        [("Eq", ProcMacroKind::DeriveLike), ("Show", ProcMacroKind::DeriveLike), ("ordinary", ProcMacroKind::FunctionLike)]);
    assert!(expanded.source_map.record(super::ExpansionId(1)).is_none());
}

#[test]
fn macro_c_02_validates_external_and_local_attachments_without_execution() {
    let cfg = Config { no_proc_execution: true, ..Config::default() };
    let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("dep".into(), descriptor(ProcMacroKind::DeriveLike));
    let program = crate::parser::parse_string("> dep::Show\n%derive Show\nstruct S\nmod nested\n    %derive dep::Show\n    enum E\n        A\n", &cfg).unwrap();
    let program = expand_macros_with_context(program, &context).unwrap();
    assert!(matches!(program.module.top_levels[1], TopLevel::DerivedDecl(_)));
    assert!(context.source_map().record(super::ExpansionId(1)).is_none());
    let source = "< macro derive Show = context, input -> missing!\n%derive Show\nstruct S\n";
    let expanded = crate::discover_proc_macros(&config(source)).unwrap();
    assert_eq!(expanded.proc_exports[0].export.kind, ProcMacroKind::DeriveLike);
    let errors = crate::analyze(&config(source)).unwrap_err();
    assert!(format!("{errors:?}").contains("derive execution is not implemented"));
}

#[test]
fn macro_c_02_canonical_duplicates_wrong_kinds_and_missing_exports_diagnose() {
    for (source, kind, expected) in [
        ("> dep::Show\n%derive Show, dep::Show\nstruct S\n", ProcMacroKind::DeriveLike, "duplicate canonical"),
        ("%derive dep::Show\nstruct S\n", ProcMacroKind::FunctionLike, "DeriveLike"),
        ("%derive dep::Show\nstruct S\n", ProcMacroKind::AttributeLike, "DeriveLike"),
        ("%derive missing\nstruct S\n", ProcMacroKind::DeriveLike, "unresolved derive"),
        ("%derive dep::Missing\nstruct S\n", ProcMacroKind::DeriveLike, "no exported procedural macro"),
    ] {
        let cfg = Config { no_proc_execution: true, ..Config::default() };
        let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("dep".into(), descriptor(kind));
        let program = crate::parser::parse_string(source, &cfg).unwrap();
        let errors = expand_macros_with_context(program, &context).unwrap_err();
        assert!(format!("{errors:?}").contains(expected), "{source}: {errors:?}");
    }
    for source in ["macro plain\n    =>\n        1\n%derive plain\nstruct S\n",
        "macro plain = context, input -> input\n%derive plain\nstruct S\n"] {
        let errors = crate::discover_proc_macros(&config(source)).unwrap_err();
        assert!(format!("{errors:?}").contains("DeriveLike"), "{errors:?}");
    }
}

#[test]
fn macro_c_02_generated_exports_and_attachments_validate_at_stable_frontier() {
    let source = "macro publish\n    =>\n        < macro derive Show = context, input -> input\n        %derive Show\n        struct S\n%publish\n";
    let expanded = crate::discover_proc_macros(&config(source)).unwrap();
    assert_eq!(expanded.proc_exports[0].export.kind, ProcMacroKind::DeriveLike);
    assert!(expanded.ast.module.top_levels.iter().any(|item| matches!(item, TopLevel::DerivedDecl(_))));
    assert!(expanded.source_map.record(super::ExpansionId(1)).is_some());
    assert!(expanded.source_map.record(super::ExpansionId(2)).is_none(), "derive did not run");
}

#[test]
fn macro_c_02_wrong_targets_duplicate_source_names_and_wrong_call_kind_reject() {
    for source in ["< macro derive Show = context, input -> input\n",
        "macro publish\n    =>\n        < macro derive Show = context, input -> input\n%publish\n"] {
        for role in [TargetRole::Library, TargetRole::Binary] {
            let mut cfg = config(source); cfg.target_role = role;
            assert!(format!("{:?}", crate::discover_proc_macros(&cfg).unwrap_err()).contains("proc_macro = true"));
        }
    }
    let source = "macro same = context, input -> input\nmacro derive same = context, input -> input\n";
    assert!(format!("{:?}", crate::discover_proc_macros(&config(source)).unwrap_err()).contains("conflicting macro declaration"));
    for invocation in ["%show\n", "helper = -> %show!\n"] {
        let source = format!("macro derive show = context, input -> input\n{invocation}");
        let errors = crate::discover_proc_macros(&config(&source)).unwrap_err();
        assert!(format!("{errors:?}").contains("not function-like"), "{source:?}: {errors:?}");
    }
    for source in ["%dep::Show\n", "helper = -> %dep::Show!\n"] {
        let cfg = Config::default();
        let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("dep".into(), descriptor(ProcMacroKind::DeriveLike));
        let program = crate::parser::parse_string(source, &cfg).unwrap();
        let errors = expand_macros_with_context(program, &context).unwrap_err();
        assert!(format!("{errors:?}").contains("not function-like"), "{source:?}: {errors:?}");
    }
    let mut artifact = descriptor(ProcMacroKind::DeriveLike);
    artifact.validate_metadata().unwrap();
    artifact.exports[0].kind = ProcMacroKind::AttributeLike;
    assert!(artifact.validate_metadata().is_err());
}

#[test]
fn macro_c_02_file_modules_preserve_visibility_and_attachment_resolution_scopes() {
    let mut cfg = config("< mod public\nmod private\n%derive public::Show\nstruct Root\n");
    cfg.source_providers.extend([
        SourceProvider::Virtual { path: "/macro-c02/public.rk".into(), text: "< macro derive Show = context, input -> input\nmacro derive Local = context, input -> input\n%derive Local\nstruct S\n".into() },
        SourceProvider::Virtual { path: "/macro-c02/private.rk".into(), text: "< macro derive Hidden = context, input -> input\n%derive Hidden\nenum E\n    A\n".into() },
    ]);
    let expanded = crate::discover_proc_macros(&cfg).unwrap();
    assert_eq!(expanded.proc_exports.len(), 1);
    assert_eq!(expanded.proc_exports[0].export.name, "public::Show");
    assert_eq!(expanded.proc_exports[0].export.kind, ProcMacroKind::DeriveLike);
    cfg.source_providers[0] = SourceProvider::Virtual { path: "/macro-c02/lib.rk".into(), text: "< mod public\n%derive public::Local\nstruct Root\n".into() };
    assert!(crate::discover_proc_macros(&cfg).is_err());
    let mut cfg = config("mod nested\n    < macro derive Show = context, input -> input\n    %derive Show\n    struct S\n");
    assert!(crate::discover_proc_macros(&cfg).unwrap().proc_exports.is_empty());
    cfg.target_role = TargetRole::Library;
    assert!(format!("{:?}", crate::discover_proc_macros(&cfg).unwrap_err()).contains("proc_macro = true"));
}

#[test]
fn macro_c_02_synthetic_source_kind_metadata_is_checked_before_discovery() {
    let cfg = config("macro derive Show = context, input -> input\n");
    let mut program = crate::parser::parse_string("macro derive Show = context, input -> input\n", &cfg).unwrap();
    let TopLevel::ProcMacroDecl(node) = &mut program.module.top_levels[0] else { panic!("proc macro") };
    node.kind = ProcMacroKind::AttributeLike;
    let errors = expand_macros_with_context(program, &MacroExpansionContext::new(&cfg)).unwrap_err();
    assert!(format!("{errors:?}").contains("unsupported source procedural macro kind"));
    for missing_input in [false, true] {
        let mut program = crate::parser::parse_string("%derive dep::Show\nstruct S\n", &cfg).unwrap();
        let TopLevel::DerivedDecl(node) = &mut program.module.top_levels[0] else { panic!("derived") };
        if missing_input {
            let crate::ast::DeriveTarget::Struct(target) = &mut node.target else { panic!("struct") };
            target.original = None;
        } else { node.derives.clear(); }
        let context = MacroExpansionContext::new(&cfg).with_named_proc_macro_artifact("dep".into(), descriptor(ProcMacroKind::DeriveLike));
        let errors = expand_macros_with_context(program, &context).unwrap_err();
        assert!(format!("{errors:?}").contains("no original declaration input"));
    }
}
