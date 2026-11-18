use crate::{analyze, compile_with_products, Config, CrateKind, SourceProvider};
use crate::lint::LintLevel;

fn config(source: &str, kind: CrateKind) -> Config {
    let entry_file = std::path::PathBuf::from("/virtual/dead_code/main.rk");
    Config {
        entry_file: entry_file.clone(),
        crate_kind: kind,
        no_std: true,
        no_prelude: true,
        no_link: true,
        source_providers: vec![SourceProvider::Virtual {
            path: entry_file,
            text: source.to_string(),
        }],
        ..Config::default()
    }
}

fn messages(source: &str, kind: CrateKind) -> Vec<String> {
    analyze(&config(source, kind)).unwrap().warnings.0.into_iter()
        .filter(|diagnostic| diagnostic.message.contains("never used"))
        .map(|diagnostic| diagnostic.message).collect()
}

#[test]
fn binary_exports_are_not_roots_and_dead_cycles_are_dead() {
    let warnings = messages(
        "< unused = -> 1\na = -> b!\nb = -> a!\nmain = !-> ()\n",
        CrateKind::Bin,
    );
    assert!(warnings.iter().any(|message| message.contains("`unused`")));
    assert!(warnings.iter().any(|message| message.contains("`a`")));
    assert!(warnings.iter().any(|message| message.contains("`b`")));
    assert!(!warnings.iter().any(|message| message.contains("`main`")));
}

#[test]
fn library_export_keeps_transitive_helpers_but_not_private_items() {
    let warnings = messages(
        "< exported = -> helper!\nhelper = -> 1\nprivate = -> 2\n",
        CrateKind::Lib,
    );
    assert_eq!(warnings, ["function `private` is never used"]);
}

#[test]
fn functions_used_as_values_are_live() {
    let warnings = messages(
        "identity = x -> x\nmain = ->\n    f = identity\n    f 1\n",
        CrateKind::Bin,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn used_type_alias_is_not_reported_dead() {
    let analysis = analyze(&config(
        "type Count = I64\nanswer: () -> Count\nanswer = -> 1\nmain = -> answer!\n",
        CrateKind::Bin,
    )).unwrap();
    assert!(!analysis.warnings.contains("never used"), "{:?}", analysis.warnings);
}

#[test]
fn inherent_method_reachability_does_not_keep_unused_siblings() {
    let warnings = messages(
        "struct Box\n    < value: I64\nimpl Box\n    @get = -> @value\n    @unused = -> 2\nmain = ->\n    b = Box\n        value: 1\n    b.get!\n",
        CrateKind::Bin,
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("unused"), "{warnings:?}");
}

#[test]
fn private_module_export_is_dead_unless_reexported() {
    for (declaration, expected_dead) in [
        ("mod inner", true),
        ("< mod inner", false),
        ("mod inner\n< inner", false),
        ("mod inner\n< inner::answer", false),
        ("mod inner\n< inner::*", false),
    ] {
        let mut config = config(&format!("{declaration}\n"), CrateKind::Lib);
        config.source_providers.push(SourceProvider::Virtual {
            path: "/virtual/dead_code/inner.rk".into(),
            text: "< answer = -> 42\n".to_string(),
        });
        let analysis = analyze(&config).unwrap();
        assert_eq!(analysis.warnings.contains("never used"), expected_dead,
            "{declaration}: {:?}; {:?}", analysis.warnings, analysis.hir.resolver.export_aliases);
    }
}

#[test]
fn regression_pruned_variant_exports_do_not_break_library_products() {
    let mut config = config("mod inner\n< Choice::Public\n< enum Choice\n    Public\n", CrateKind::Lib);
    config.current_crate_name = Some("review".into());
    config.source_providers.push(SourceProvider::Virtual {
        path: "/virtual/dead_code/inner.rk".into(),
        text: "enum Private\n    Hidden\n< Private::Hidden\n".into(),
    });
    let dir = std::env::temp_dir().join(format!("rock_variant_exports_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    config.output_dir = dir.clone();
    let result = compile_with_products(&config);
    std::fs::remove_dir_all(dir).unwrap();
    let products = result.unwrap().products.unwrap();
    let exports = &products.identity_table.variant_export_names;
    assert!(exports.values().any(|(_, variant)| variant == "Public"), "{exports:?}");
    assert!(!exports.values().any(|(_, variant)| variant == "Hidden"));
}

#[test]
fn variant_patterns_do_not_count_as_construction() {
    let analysis = analyze(&config(
        "enum Choice\n    First\n    Second\nmain = ->\n    value = Choice::First\n    match value\n        Choice::First => 0\n        Choice::Second => 1\n",
        CrateKind::Bin,
    )).unwrap();
    assert!(analysis.warnings.contains("Choice::Second"));
    assert!(!analysis.warnings.contains("variant `Choice::First`"));
}

#[test]
fn denied_dead_code_stops_before_object_emission() {
    let mut config = config("unused = -> 1\nmain = !-> ()\n", CrateKind::Bin);
    config.lint_levels.set("dead_code", LintLevel::Deny).unwrap();
    let error = compile_with_products(&config).unwrap_err();
    assert!(error.has_errors());
    assert!(error.contains("function `unused` is never used"));
}

#[test]
fn filtered_type_locations_match_full_recollection() {
    let config = config(
        "extern dead: I64 -> I64\nextern abs: I64 -> I64\nunused = -> 1\nmain = -> abs 0\n",
        CrateKind::Bin,
    );
    let mut analysis = analyze(&config).unwrap();
    let modules = crate::lint::SourceModules {
        root: &analysis.ast.module,
        files: Default::default(),
        current_crate_name: None,
    };
    let reachable = super::Reachability::compute(&analysis.hir, &modules, &config, []);
    reachable.prune(&mut analysis.hir);
    let expected = crate::hir::collect_hir_type_ids(
        analysis.hir.program.program(), &mut analysis.hir.type_context,
    );
    assert_eq!(analysis.hir.type_ids.len(), expected.len());
    for (location, id) in expected.iter() {
        assert_eq!(analysis.hir.type_ids.get(location), Some(id));
    }
    let mut rebuilt = analysis.hir.program.program().clone();
    rebuilt.rebuild_indexes_with_canonical_names(&analysis.hir.resolver.item_names_by_id);
    let actual = &analysis.hir.program.indexes;
    let expected = &rebuilt.indexes;
    macro_rules! same {
        ($($field:ident),+ $(,)?) => { $(assert_eq!(actual.$field, expected.$field);)+ };
    }
    same!(functions_by_id, structs_by_id, enums_by_id, traits_by_id, type_aliases_by_id,
        impls_by_id, externs_by_id, impl_owner_ids_by_id, methods_by_id,
        effective_trait_methods, fields_by_id, variants_by_id,
        associated_type_decls_by_id, associated_type_defs_by_id);
}

#[test]
fn dead_code_still_has_type_errors() {
    let error = analyze(&config(
        "bad: () -> I64\nbad = -> true\nmain = !-> ()\n", CrateKind::Bin,
    )).unwrap_err();
    assert!(error.has_errors());
}

#[test]
fn suppression_does_not_prevent_pruning_and_analysis_keeps_source_items() {
    let source = "struct DeadType\n    < value: I64\nenum DeadEnum\n    First\n    Second\nunused = -> 1\n_helper = -> 2\nmain = !-> ()\n";
    let mut config = config(source, CrateKind::Bin);
    config.lint_levels.set("dead_code", LintLevel::Allow).unwrap();
    let analysis = analyze(&config).unwrap();
    assert_eq!(analysis.hir.program.functions.len(), 3);
    let dir = std::env::temp_dir().join(format!("rock_dead_code_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    config.output_dir = dir.clone();
    config.emit_llvm = true;
    let output = compile_with_products(&config).unwrap();
    let products = output.products.unwrap();
    assert!(products.identity_table.display_names.values().any(|name| name == "main"));
    assert!(!products.identity_table.display_names.values().any(|name| name == "unused" || name == "_helper"));
    assert!(products.interface.structs.is_empty());
    assert!(products.interface.enums.is_empty());
    let ir = std::fs::read_to_string(dir.join("main.ll")).unwrap();
    assert!(!ir.contains("unused"));
    assert!(!ir.contains("_helper"));
    std::fs::remove_dir_all(dir).unwrap();
}
