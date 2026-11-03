use super::*;

fn test_extern(id: DefId, name: &str) -> HirExtern {
    HirExtern {
        id,
        name: name.to_string(),
        params: Vec::new(),
        ret: Type::I64,
        variadic: false,
        is_unsafe: false,
    }
}

#[test]
fn resolution_context_resolves_current_and_dependency_items() {
    let current_id = def_id(0, 10);
    let dep_id = def_id(2, 20);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .resolver
        .item_paths
        .insert("demo::local".to_string(), current_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(current_id, "demo::local".to_string());

    let mut dep = ResolverTables::default();
    dep.item_paths.insert("dep::value".to_string(), dep_id);
    dep.item_names_by_id
        .insert(dep_id, "dep::value".to_string());
    lowerer.resolver.merge_global_inputs(&dep);
    lowerer.dependency_resolvers.insert("dep".to_string(), dep);

    let resolution = LowerResolutionContext::new(&lowerer);

    assert_eq!(resolution.resolve_item_id("demo::local"), Some(current_id));
    assert_eq!(resolution.resolve_item_id("dep::value"), Some(dep_id));
    assert_eq!(resolution.canonical_name(current_id), Some("demo::local"));
    assert_eq!(resolution.canonical_name(dep_id), Some("dep::value"));
}

#[test]
fn resolution_context_prefers_module_alias_before_root_item_when_requested() {
    let root_id = def_id(0, 30);
    let module_id = def_id(0, 31);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .resolver
        .item_paths
        .insert("Thing".to_string(), root_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_id, "Thing".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "Thing".to_string(),
        "demo::helper::Thing".to_string(),
        module_id,
    );

    let resolution = LowerResolutionContext::new(&lowerer);

    assert_eq!(resolution.resolve_item_id("Thing"), Some(root_id));
    assert_eq!(
        resolution.resolve_module_alias_or_item_id("Thing"),
        Some(module_id)
    );
    assert_eq!(
        resolution.canonical_name_for_module_alias_or_item("Thing"),
        Some("demo::helper::Thing".to_string())
    );
}

#[test]
fn resolution_context_resolves_local_before_top_level_alias() {
    let function_id = def_id(0, 60);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_function(test_function(function_id, "value"));
    lowerer
        .resolver
        .item_paths
        .insert("value".to_string(), function_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(function_id, "value".to_string());
    lowerer
        .scope
        .define_local("value".to_string(), Type::Bool, true, HirLocalId(9));

    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution.resolve_identifier_value("value").unwrap();

    assert_eq!(resolved.target, Some(HirVarTarget::Local(HirLocalId(9))));
    assert_eq!(resolved.name, "value");
    assert_eq!(resolved.ty, Type::Bool);
}

#[test]
fn resolution_context_keeps_nested_local_function_value_shadow_local() {
    let function_id = def_id(0, 98);
    let local_id = HirLocalId(10);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_function(test_function(function_id, "value"));
    lowerer
        .resolver
        .item_paths
        .insert("value".to_string(), function_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(function_id, "value".to_string());
    lowerer.scope.define(
        "value".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );
    lowerer.scope.push();
    lowerer.scope.define_local(
        "value".to_string(),
        Type::function(Vec::new(), Type::Bool),
        false,
        local_id,
    );

    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution.resolve_identifier_value("value").unwrap();

    assert_eq!(resolved.target, Some(HirVarTarget::Local(local_id)));
    assert_eq!(resolved.ty, Type::function(Vec::new(), Type::Bool));
}

#[test]
fn resolution_context_resolves_alias_to_top_level_target() {
    let function_id = def_id(0, 61);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_function(test_function(function_id, "demo::helper::value"));
    lowerer.resolver.insert_module_alias_with_name(
        "value".to_string(),
        "demo::helper::value".to_string(),
        function_id,
    );
    lowerer.scope.define_alias(
        "value".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );

    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution.resolve_identifier_value("value").unwrap();

    assert_eq!(resolved.target, Some(HirVarTarget::Function(function_id)));
    assert_eq!(resolved.name, "demo::helper::value");
}

#[test]
fn resolution_context_resolves_scoped_function_alias_through_resolver_metadata() {
    let function_id = def_id(0, 92);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_function(test_function(function_id, "answer"));
    lowerer
        .resolver
        .item_paths
        .insert("answer".to_string(), function_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(function_id, "answer".to_string());
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );

    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution.resolve_identifier_value("answer").unwrap();

    assert_eq!(resolved.target, Some(HirVarTarget::Function(function_id)));
    assert_eq!(resolved.name, "answer");
}

#[test]
fn resolution_context_requires_resolver_ids_for_scoped_alias_targets() {
    let function_id = def_id(0, 97);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_function(test_function(function_id, "answer"));
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );

    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution.resolve_identifier_value("answer").unwrap();

    assert_eq!(resolved.target, None);
    assert_eq!(resolved.name, "answer");
}

#[test]
fn resolution_context_resolves_scoped_extern_alias_through_resolver_metadata() {
    let extern_id = def_id(0, 93);
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_extern(test_extern(extern_id, "puts"));
    lowerer
        .resolver
        .item_paths
        .insert("puts".to_string(), extern_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(extern_id, "puts".to_string());
    lowerer.scope.define_alias(
        "puts".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );

    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution.resolve_identifier_value("puts").unwrap();

    assert_eq!(resolved.target, Some(HirVarTarget::Extern(extern_id)));
    assert_eq!(resolved.name, "puts");
}

#[test]
fn resolution_context_resolves_qualified_function_after_first_segment_alias() {
    let function_id = def_id(0, 70);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_function(test_function(function_id, "demo::helper::answer"));
    lowerer
        .resolver
        .item_paths
        .insert("demo::helper::answer".to_string(), function_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(function_id, "demo::helper::answer".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "Helper".to_string(),
        "demo::helper".to_string(),
        def_id(0, 71),
    );

    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution
        .resolve_qualified_value_path(&["Helper".to_string(), "answer".to_string()])
        .unwrap();

    assert_eq!(resolved.target, Some(HirVarTarget::Function(function_id)));
    assert_eq!(resolved.name, "demo::helper::answer");
}

#[test]
fn resolution_context_resolves_qualified_function_through_resolver_metadata() {
    let function_id = def_id(0, 72);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_function(test_function(function_id, "pkg::answer"));
    lowerer
        .resolver
        .item_paths
        .insert("pkg::answer".to_string(), function_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(function_id, "pkg::answer".to_string());

    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution
        .resolve_qualified_value_path(&["pkg".to_string(), "answer".to_string()])
        .unwrap();

    assert_eq!(resolved.target, Some(HirVarTarget::Function(function_id)));
    assert_eq!(resolved.name, "pkg::answer");
}

#[test]
fn resolution_context_requires_resolver_ids_for_qualified_values() {
    let function_id = def_id(0, 96);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_function(test_function(function_id, "pkg::answer"));

    let resolution = LowerResolutionContext::new(&lowerer);

    assert!(resolution
        .resolve_qualified_value_path(&["pkg".to_string(), "answer".to_string()])
        .is_none());
}

#[test]
fn resolution_context_resolves_scoped_qualified_function_through_resolver_metadata() {
    let function_id = def_id(0, 73);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_function(test_function(function_id, "pkg::answer"));
    lowerer
        .resolver
        .item_paths
        .insert("pkg::answer".to_string(), function_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(function_id, "pkg::answer".to_string());
    lowerer.scope.define_alias(
        "pkg::answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );

    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution
        .resolve_qualified_value_path(&["pkg".to_string(), "answer".to_string()])
        .unwrap();

    assert_eq!(resolved.target, Some(HirVarTarget::Function(function_id)));
    assert_eq!(resolved.name, "pkg::answer");
}

#[test]
fn resolution_context_resolves_artifact_root_glob_targets_from_dependency_resolver_exports() {
    let export_id = def_id(3, 5);
    let mut lowerer = Lowerer::new_for_test();
    let mut dep = ResolverTables::default();
    dep.insert_export_alias_with_name("answer".to_string(), "dep::answer".to_string(), export_id);
    lowerer.dependency_resolvers.insert("dep".to_string(), dep);

    let resolution = LowerResolutionContext::new(&lowerer);
    let targets = resolution
        .resolve_glob_import_targets(&["dep".to_string()])
        .unwrap();

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].short_name, "answer");
    assert_eq!(targets[0].source, "dep::answer");
    assert_eq!(targets[0].id, Some(export_id));
}

#[test]
fn resolution_context_preserves_namespaced_dependency_export_aliases_for_root_globs() {
    let show_println_id = def_id(3, 6);
    let debug_fmt_id = def_id(3, 7);
    let mut lowerer = Lowerer::new_for_test();
    let mut dep = ResolverTables::default();
    dep.insert_export_alias_with_name(
        "Show::println".to_string(),
        "dep::Show::println".to_string(),
        show_println_id,
    );
    dep.insert_export_alias_with_name(
        "dep::Debug::fmt".to_string(),
        "dep::Debug::fmt".to_string(),
        debug_fmt_id,
    );
    lowerer.dependency_resolvers.insert("dep".to_string(), dep);

    let resolution = LowerResolutionContext::new(&lowerer);
    let targets = resolution
        .resolve_glob_import_targets(&["dep".to_string()])
        .unwrap();

    assert_eq!(
        targets,
        vec![
            LowerImportTarget {
                short_name: "Debug::fmt".to_string(),
                source: "dep::Debug::fmt".to_string(),
                id: Some(debug_fmt_id),
            },
            LowerImportTarget {
                short_name: "Show::println".to_string(),
                source: "dep::Show::println".to_string(),
                id: Some(show_println_id),
            },
        ]
    );
}

#[test]
fn resolution_context_does_not_resolve_qualified_missing_path_through_prelude_suffix() {
    let drop_id = def_id(7, 42);
    let mut lowerer = Lowerer::new_for_test();
    *lowerer.prelude = PreludeImports::with_exports(
        true,
        [(
            "Drop".to_string(),
            ArtifactExport {
                source: "std::drop::Drop".to_string(),
                id: drop_id,
            },
        )]
        .into_iter()
        .collect(),
    );
    lowerer.resolver.insert_import_alias_with_name(
        "Drop".to_string(),
        "std::drop::Drop".to_string(),
        drop_id,
    );

    let resolution = LowerResolutionContext::new(&lowerer);

    assert_eq!(resolution.resolve_item_id("Drop"), Some(drop_id));
    assert_eq!(resolution.resolve_item_id("missing::Drop"), None);
}

#[test]
fn prelude_imports_preserves_prelude_sync_same_export_alias() {
    let export_id = def_id(0, 90);
    let other_id = def_id(0, 91);
    let mut resolver = ResolverTables::default();

    assert!(
        !crate::lower::prelude::PreludeImports::export_short_name_is_user_owned(
            &resolver, "map", export_id
        )
    );

    resolver.insert_import_alias_with_name(
        "map".to_string(),
        "std::prelude::map".to_string(),
        export_id,
    );
    assert!(
        !crate::lower::prelude::PreludeImports::export_short_name_is_user_owned(
            &resolver, "map", export_id
        )
    );

    resolver.insert_import_alias_with_name(
        "filter".to_string(),
        "demo::filter".to_string(),
        other_id,
    );
    assert!(
        crate::lower::prelude::PreludeImports::export_short_name_is_user_owned(
            &resolver, "filter", export_id
        )
    );

    resolver.item_paths.insert("fold".to_string(), other_id);
    assert!(
        crate::lower::prelude::PreludeImports::export_short_name_is_user_owned(
            &resolver, "fold", export_id
        )
    );
}
