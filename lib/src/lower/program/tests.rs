use super::*;

use crate::ast::{Ident, Module};
use crate::hir::{HirBlock, HirEnum, HirFunction, HirStruct, HirTrait};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::lexer::Span;
use crate::source_loader::LoadedModule;
use crate::types::Type;

fn test_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: std::collections::HashMap::new().into(),
        params: Vec::new(),
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
        methods: std::collections::HashMap::new(),
        signatures: std::collections::HashMap::new(),
    }
}

fn loaded_module(qualified_name: &str, path: std::path::PathBuf, module: Module) -> LoadedModule {
    LoadedModule {
        id: crate::source_loader::ModuleId(0),
        qualified_name: qualified_name.to_string(),
        path: path.clone(),
        canonical_path: path,
        module,
    }
}

#[test]
fn source_module_resolver_requires_seeded_module_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_requires_seeded_module_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let root = temp_dir.join("main.rk");

    let mut lowerer = Lowerer::new_for_test();
    lowerer.modules.set_current_module_path(root);

    let error = SourceModuleResolver::new(&mut lowerer.modules)
        .load_local_module("util")
        .expect_err("module cache should not bypass graph-seeded module paths");

    assert!(error.contains("was not loaded by the source database"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn import_alias_does_not_remove_current_crate_function_with_same_short_name() {
    let mut lowerer = Lowerer::new_for_test();
    let local_id = DefId::new(CrateId(0), LocalDefId(1));
    let imported_id = DefId::new(CrateId(7), LocalDefId(1));
    lowerer.current_def_ids.insert(local_id);
    lowerer
        .resolver
        .item_paths
        .insert("answer".to_string(), local_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(local_id, "answer".to_string());
    lowerer
        .resolver
        .item_paths
        .insert("dep::answer".to_string(), imported_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(imported_id, "dep::answer".to_string());
    lowerer
        .items
        .insert_function(test_function(local_id, "answer"));
    lowerer
        .items
        .insert_function(test_function(imported_id, "answer"));

    lowerer.import_qualified_name("answer".to_string(), "dep::answer".to_string());

    assert_eq!(
        lowerer.items.function(local_id).map(|func| func.id),
        Some(local_id)
    );
    assert_eq!(
        lowerer.resolver.resolve_item_or_alias("answer"),
        Some(local_id)
    );
}

#[test]
fn import_struct_registers_resolver_alias_metadata() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = DefId::new(CrateId(7), LocalDefId(2));
    lowerer
        .items
        .insert_structure(test_struct(struct_id, "dep::Thing"));
    lowerer
        .resolver
        .item_paths
        .insert("dep::Thing".to_string(), struct_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "dep::Thing".to_string());

    lowerer.import_qualified_name("Thing".to_string(), "dep::Thing".to_string());

    assert_eq!(
        lowerer.resolver.import_aliases.get("Thing").copied(),
        Some(struct_id)
    );
    assert_eq!(
        LowerResolutionContext::new(&lowerer)
            .resolve_struct_type("Thing")
            .map(|structure| structure.id),
        Some(struct_id)
    );
}

#[test]
fn import_enum_and_trait_register_resolver_alias_metadata() {
    let mut lowerer = Lowerer::new_for_test();
    let enum_id = DefId::new(CrateId(7), LocalDefId(3));
    let trait_id = DefId::new(CrateId(7), LocalDefId(4));
    lowerer
        .items
        .insert_enumeration(test_enum(enum_id, "dep::Choice"));
    lowerer
        .items
        .insert_trait_def(test_trait(trait_id, "dep::Show"));
    lowerer
        .resolver
        .item_paths
        .insert("dep::Choice".to_string(), enum_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(enum_id, "dep::Choice".to_string());
    lowerer
        .resolver
        .item_paths
        .insert("dep::Show".to_string(), trait_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "dep::Show".to_string());

    lowerer.import_qualified_name("Choice".to_string(), "dep::Choice".to_string());
    lowerer.import_qualified_name("Show".to_string(), "dep::Show".to_string());

    assert_eq!(
        lowerer.resolver.import_aliases.get("Choice").copied(),
        Some(enum_id)
    );
    assert_eq!(
        lowerer.resolver.import_aliases.get("Show").copied(),
        Some(trait_id)
    );
    assert_eq!(
        LowerResolutionContext::new(&lowerer)
            .resolve_enum_type("Choice")
            .map(|enum_def| enum_def.id),
        Some(enum_id)
    );
    assert_eq!(
        LowerResolutionContext::new(&lowerer)
            .resolve_trait_type("Show")
            .map(|trait_def| trait_def.id),
        Some(trait_id)
    );
}

#[test]
fn source_module_resolver_prefers_current_crate_prefixed_graph_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_prefers_current_crate_graph_path_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let alias_io = temp_dir.join("alias_io.rk");
    let graph_io = temp_dir.join("graph_io.rk");

    let alias_module = Module {
        name: Some(Ident {
            name: "io".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![],
        is_inline: false,
        filepath: Some(alias_io.clone()),
    };
    let graph_module = Module {
        name: Some(Ident {
            name: "io".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![],
        is_inline: false,
        filepath: Some(graph_io.clone()),
    };
    let mut lowerer = Lowerer::new_for_test();
    lowerer.modules = crate::lower::services::LowerModuleService::from_source_modules(vec![
        loaded_module("math::io", alias_io.clone(), alias_module),
        loaded_module("test::math::io", graph_io.clone(), graph_module),
    ]);
    lowerer
        .modules
        .set_current_crate_name(Some("test".to_string()));
    lowerer
        .modules
        .set_current_qualified_module_prefix(Some("math".to_string()));

    let module = SourceModuleResolver::new(&mut lowerer.modules)
        .load_local_module("io")
        .expect("current-crate-prefixed graph path should resolve");

    assert_eq!(module.filepath.as_ref(), Some(&graph_io));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn source_module_resolver_requires_seeded_external_child_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_requires_seeded_external_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let dep_root = temp_dir.join("lib.rk");
    let util = temp_dir.join("util.rk");

    let module = Module {
        name: Some(Ident {
            name: "util".to_string(),
            span: Span::test(),
        }),
        top_levels: vec![],
        is_inline: false,
        filepath: Some(util.clone()),
    };
    let mut lowerer = Lowerer::new_for_test();
    lowerer.modules =
        crate::lower::services::LowerModuleService::from_source_modules(vec![loaded_module(
            "dep", dep_root, module,
        )]);

    let error = SourceModuleResolver::new(&mut lowerer.modules)
        .load_external_crate_module("util", "dep")
        .expect_err("external module cache should not bypass graph-seeded module paths");

    assert!(error.contains("was not loaded by the source database"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn public_lower_entrypoint_uses_indexed_current_def_ids() {
    let parsed = crate::parser::parse_string(
        "trait Show\n    @show: I64\n\nimpl Show for I64\n    @show = -> 1\n\nmain = -> 0\n",
        &crate::Config::default(),
    )
    .expect("program should parse");

    let partial = lower(&parsed).expect("public lower entrypoint should lower through collect");
    let show_id = partial.resolver.item_paths["Show"];
    let trait_sig_id = partial
        .traits
        .get(&show_id)
        .and_then(|trait_def| trait_def.signatures.get("show"))
        .map(|sig| sig.id)
        .expect("trait signature should have a DefId");
    let impl_method_id = partial
        .impls
        .values()
        .find_map(|imp| imp.methods.get("show"))
        .map(|method| method.id)
        .expect("impl method should have a DefId");

    assert!(partial.current_def_ids.contains(&trait_sig_id));
    assert!(partial.current_def_ids.contains(&impl_method_id));
}
