use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::ArtifactExport;
use crate::hir::{
    HirBlock, HirEnum, HirExtern, HirFunction, HirImpl, HirImplOwner, HirImplReceiverPattern,
    HirParam, HirStruct, HirTrait, HirVarTarget,
};
use crate::ids::{CrateId, DefId, HirLocalId, LocalDefId};
use crate::lower::prelude::PreludeImports;
use crate::lower::resolution::{LowerImportTarget, LowerResolutionContext};
use crate::lower::Lowerer;
use crate::types::Type;

use std::collections::HashMap;

fn def_id(crate_id: u32, local_id: u32) -> DefId {
    DefId::new(CrateId(crate_id), LocalDefId(local_id))
}

fn test_struct(id: DefId, name: &str) -> HirStruct {
    HirStruct {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    }
}

fn test_enum(id: DefId, name: &str) -> HirEnum {
    HirEnum {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        variants: Vec::new(),
    }
}

fn test_trait(id: DefId, name: &str) -> HirTrait {
    HirTrait {
        target: None,
        predicates: Vec::new(),
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    }
}

fn test_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::<HirParam>::new(),
        ret_type: Type::I64,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

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
fn resolution_context_resolves_nominals_by_module_alias_first() {
    let root_id = def_id(0, 40);
    let module_id = def_id(0, 41);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(root_id, "Thing"));
    lowerer
        .items
        .insert_structure(test_struct(module_id, "demo::helper::Thing"));
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
    let resolved = resolution.resolve_struct_type("Thing").unwrap();

    assert_eq!(resolved.id, module_id);
}

#[test]
fn resolution_context_resolves_enum_and_trait_by_id() {
    let enum_id = def_id(0, 50);
    let trait_id = def_id(0, 51);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_enumeration(test_enum(enum_id, "demo::Choice"));
    lowerer
        .items
        .insert_trait_def(test_trait(trait_id, "demo::Show"));
    lowerer
        .resolver
        .item_paths
        .insert("Choice".to_string(), enum_id);
    lowerer
        .resolver
        .item_paths
        .insert("Show".to_string(), trait_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(enum_id, "demo::Choice".to_string());
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "demo::Show".to_string());

    let resolution = LowerResolutionContext::new(&lowerer);

    assert_eq!(resolution.resolve_enum_type("Choice").unwrap().id, enum_id);
    assert_eq!(resolution.resolve_trait_type("Show").unwrap().id, trait_id);
}

#[test]
fn resolution_context_requires_resolver_ids_for_nominal_types() {
    let struct_id = def_id(0, 52);
    let enum_id = def_id(0, 53);
    let trait_id = def_id(0, 54);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(struct_id, "Widget"));
    lowerer
        .items
        .insert_enumeration(test_enum(enum_id, "Choice"));
    lowerer.items.insert_trait_def(test_trait(trait_id, "Show"));

    let resolution = LowerResolutionContext::new(&lowerer);

    assert!(resolution.resolve_struct_type("Widget").is_none());
    assert!(resolution.resolve_enum_type("Choice").is_none());
    assert!(resolution.resolve_trait_type("Show").is_none());
}

#[test]
fn resolution_context_requires_canonical_names_for_nominal_types() {
    let struct_id = def_id(0, 55);
    let enum_id = def_id(0, 56);
    let trait_id = def_id(0, 57);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(struct_id, "pkg::Widget"));
    lowerer
        .items
        .insert_enumeration(test_enum(enum_id, "pkg::Choice"));
    lowerer
        .items
        .insert_trait_def(test_trait(trait_id, "pkg::Show"));
    lowerer
        .resolver
        .item_paths
        .insert("Widget".to_string(), struct_id);
    lowerer
        .resolver
        .item_paths
        .insert("Choice".to_string(), enum_id);
    lowerer
        .resolver
        .item_paths
        .insert("Show".to_string(), trait_id);

    let resolution = LowerResolutionContext::new(&lowerer);

    assert!(resolution.resolve_struct_type("Widget").is_none());
    assert!(resolution.resolve_enum_type("Choice").is_none());
    assert!(resolution.resolve_trait_type("Show").is_none());
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
fn resolution_context_resolves_static_method_target_by_canonical_owner() {
    let struct_id = def_id(0, 74);
    let method_id = def_id(0, 75);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(struct_id, "String"));
    lowerer.resolver.insert_import_alias_with_name(
        "String".to_string(),
        "std::string::String".to_string(),
        struct_id,
    );
    let impl_id = def_id(0, 76);
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("std::string::String".to_string()),
            type_name: "std::string::String".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("from".to_string(), test_function(method_id, "from"))]),
        })
        .unwrap();
    let resolution = LowerResolutionContext::new(&lowerer);
    let resolved = resolution
        .resolve_static_method_path(&["String".to_string(), "from".to_string()])
        .unwrap()
        .unwrap();

    assert_eq!(resolved.value.name, "std::string::String::from");
    assert_eq!(
        resolved.value.target,
        Some(HirVarTarget::Function(method_id))
    );
    assert_eq!(resolved.target.impl_id(), Some(impl_id));
    assert_eq!(resolved.target.method_id(), Some(method_id));
}

#[test]
fn resolution_context_rejects_receiver_method_without_explicit_receiver_mode_as_static() {
    let struct_id = def_id(0, 174);
    let method_id = def_id(0, 175);
    let impl_id = def_id(0, 176);
    let mut method = test_function(method_id, "value");
    method.is_method = true;
    method.self_receiver = None;
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(struct_id, "Box"));
    lowerer
        .resolver
        .item_paths
        .insert("Box".to_string(), struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "Box".to_string());
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        })
        .unwrap();

    assert!(LowerResolutionContext::new(&lowerer)
        .resolve_static_method_path(&["Box".to_string(), "value".to_string()])
        .unwrap()
        .is_none());
}

#[test]
fn resolution_context_resolves_struct_literal_target_by_module_alias() {
    let root_id = def_id(0, 76);
    let module_id = def_id(0, 77);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(root_id, "Thing"));
    lowerer
        .items
        .insert_structure(test_struct(module_id, "demo::helper::Thing"));
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
    let resolved = resolution.resolve_struct_literal_path(&["Thing".to_string()]);

    assert_eq!(resolved.name, "demo::helper::Thing");
    assert_eq!(resolved.structure.unwrap().id, module_id);
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
fn resolution_context_resolves_owner_path_by_id() {
    let owner_id = def_id(0, 81);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .resolver
        .item_paths
        .insert("demo::Box".to_string(), owner_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(owner_id, "demo::Box".to_string());

    let resolution = LowerResolutionContext::new(&lowerer);

    assert_eq!(
        resolution.try_canonical_owner_path("demo::Box"),
        Some("demo::Box".to_string())
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

#[test]
fn resolution_context_does_not_mark_import_alias_struct_unexported() {
    let string_id = def_id(0, 94);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(string_id, "std::string::String"));
    lowerer.resolver.insert_import_alias_with_name(
        "String".to_string(),
        "std::string::String".to_string(),
        string_id,
    );

    let resolution = LowerResolutionContext::new(&lowerer);

    assert!(!resolution.struct_segment_looks_unexported("String"));
}

#[test]
fn resolution_context_marks_qualified_only_struct_unexported() {
    let hidden_id = def_id(0, 95);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_structure(test_struct(hidden_id, "pkg::Hidden"));
    lowerer
        .resolver
        .item_names_by_id
        .insert(hidden_id, "pkg::Hidden".to_string());

    let resolution = LowerResolutionContext::new(&lowerer);

    assert!(resolution.struct_segment_looks_unexported("Hidden"));
}
