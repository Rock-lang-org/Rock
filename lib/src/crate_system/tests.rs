use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::ast::Module;
use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::{ArtifactCrateInterface, ArtifactCrossCrateHir, ArtifactExport};
use crate::hir::{
    AcceptedHirBlock as HirBlock, AcceptedHirFunction as HirFunction, AcceptedHirImpl as HirImpl,
    AcceptedHirTrait as HirTrait, HirImplOwner,
};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::types::{GenericParamDecl, GenericParamId, Type};
use crate::SourceProvider;

use super::*;

fn empty_manifest(name: &str) -> CrateManifest {
    CrateManifest {
        crate_: CrateConfig {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            no_std: false,
        },
        targets: vec![CrateTarget::Lib {
            path: "lib.rk".to_string(),
        }],
        dependencies: None,
    }
}

fn empty_module() -> Module {
    Module {
        name: None,
        top_levels: Vec::new(),
        is_inline: false,
        filepath: None,
    }
}

fn hir_function_with_id(
    id: DefId,
    name: &str,
    _qualified_name: Option<&str>,
    generic_params: &[&str],
) -> HirFunction {
    let generic_params = generic_params
        .iter()
        .enumerate()
        .map(|(index, name)| {
            GenericParamDecl::type_param(
                GenericParamId {
                    owner: id,
                    index: index as u32,
                },
                *name,
            )
        })
        .collect();

    HirFunction {
        id,
        name: name.to_string(),
        generic_params,
        generic_bounds: HashMap::new().into(),
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

#[test]
fn test_crate_context_parse_rock_toml_delegates_to_shared_parser() {
    let content = r#"
[crate]
name = "test_crate"
version = "0.1.0"

[lib]
path = "src/lib.rk"
"#;

    let manifest = crate::crate_system::CrateContext::parse_rock_toml(content).unwrap();
    assert_eq!(manifest.crate_.name, "test_crate");
    assert_eq!(manifest.primary_target().path(), "src/lib.rk");
}

#[test]
fn test_crate_context_new() {
    let ctx = CrateContext::new();
    assert_eq!(ctx.crate_count(), 0);
}

#[test]
fn test_crate_context_register() {
    let mut ctx = CrateContext::new();

    let manifest = CrateManifest {
        crate_: CrateConfig {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            no_std: false,
        },
        targets: vec![CrateTarget::Lib {
            path: "lib.rk".to_string(),
        }],
        dependencies: None,
    };
    let module = Module {
        name: None,
        top_levels: vec![],
        is_inline: false,
        filepath: None,
    };

    ctx.register_crate(manifest, PathBuf::from("/test"), module);
    assert!(ctx.has_crate("test"));
}

#[test]
fn crate_context_registers_source_crates_outside_extern_store() {
    let mut ctx = CrateContext::new();
    ctx.register_crate(empty_manifest("dep"), PathBuf::from("/dep"), empty_module());

    assert!(ctx.has_crate("dep"));
    assert!(ctx.has_source_crate("dep"));
    assert!(!ctx.has_extern_crate("dep"));
    assert!(ctx.source_crate("dep").is_some());
    assert!(ctx.extern_crate("dep").is_none());
}

#[test]
fn load_crate_from_dir_seeds_source_module_cache() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_crate_context_source_cache_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(temp_dir.join("src")).unwrap();
    std::fs::write(
        temp_dir.join("rock.toml"),
        r#"[crate]
name = "dep"
version = "0.1.0"

[lib]
path = "src/lib.rk"
"#,
    )
    .unwrap();
    std::fs::write(temp_dir.join("src/lib.rk"), "mod util\n< util::*\n").unwrap();
    let util_path = temp_dir.join("src/util.rk");
    std::fs::write(&util_path, "answer = -> 42\n< answer\n").unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_crate_from_dir(temp_dir.clone())
        .expect("source crate with module should load");

    let source = ctx
        .source_crate("dep")
        .expect("dep source crate should load");
    assert!(source.file_cache.contains_key(&util_path));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn load_crate_from_dir_uses_source_database_module_graph() {
    let dir = std::env::temp_dir().join(format!("rock_crate_source_graph_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("rock.toml"),
        "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rk"), "mod util\n< util::*\n").unwrap();
    std::fs::write(
        dir.join("src").join("util.rk"),
        "answer = -> 42\n< answer\n",
    )
    .unwrap();

    let mut ctx = crate::crate_system::CrateContext::new();
    ctx.load_crate_from_dir(dir.clone())
        .expect("source crate should load");
    let source = ctx.source_crate("dep").expect("dep should be registered");

    assert!(source.file_cache.values().any(|module| {
        module
            .filepath
            .as_ref()
            .is_some_and(|path| path.ends_with("util.rk"))
    }));
    let tree = source
        .build_module_tree()
        .expect("module tree should build");
    assert!(tree.modules.contains_key("dep::util"));
    assert_eq!(tree.modules["dep"].submodules, vec!["util".to_string()]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_crate_from_dir_with_source_providers_loads_virtual_modules() {
    let dir = std::env::temp_dir().join(format!(
        "rock_crate_virtual_source_provider_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("rock.toml"),
        "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rk"), "mod util\n< util::*\n").unwrap();
    let util_path = dir.join("src").join("util.rk");

    let mut ctx = crate::crate_system::CrateContext::new();
    ctx.load_crate_from_dir_with_source_providers(
        dir.clone(),
        vec![SourceProvider::Virtual {
            path: util_path.clone(),
            text: "answer = -> 42\n< answer\n".to_string(),
        }],
    )
    .expect("source crate should load virtual child module through SourceDatabase");

    let source = ctx.source_crate("dep").expect("dep should be registered");
    assert!(source.file_cache.contains_key(&util_path));
    let tree = source
        .build_module_tree()
        .expect("module tree should include virtual child");
    assert!(tree.modules.contains_key("dep::util"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_crate_from_dir_with_source_providers_loads_artifact_modules() {
    let dir = std::env::temp_dir().join(format!(
        "rock_crate_artifact_source_provider_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("rock.toml"),
        "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rk"), "mod util\n< util::*\n").unwrap();
    let util_path = dir.join("src").join("util.rk");
    let artifact_path = dir.join("dep.rkca");

    let mut ctx = crate::crate_system::CrateContext::new();
    ctx.load_crate_from_dir_with_source_providers(
        dir.clone(),
        vec![SourceProvider::Artifact {
            path: util_path.clone(),
            artifact_path,
            text: "answer = -> 42\n< answer\n".to_string(),
        }],
    )
    .expect("source crate should load artifact-backed child module through SourceDatabase");

    let source = ctx.source_crate("dep").expect("dep should be registered");
    assert!(source.file_cache.contains_key(&util_path));
    let tree = source
        .build_module_tree()
        .expect("module tree should include artifact-backed child");
    assert!(tree.modules.contains_key("dep::util"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_crate_with_dependencies_with_source_providers_loads_nested_virtual_modules() {
    let root = std::env::temp_dir().join(format!(
        "rock_crate_dependency_virtual_provider_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let app = root.join("app");
    let dep = root.join("dep");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::create_dir_all(dep.join("src")).unwrap();
    std::fs::write(
        app.join("rock.toml"),
        "[crate]\nname = \"app\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n\n[dependencies.dep]\npath = \"../dep\"\n",
    )
    .unwrap();
    std::fs::write(app.join("src").join("lib.rk"), "main = -> 0\n").unwrap();
    std::fs::write(
        dep.join("rock.toml"),
        "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
    )
    .unwrap();
    std::fs::write(dep.join("src").join("lib.rk"), "mod util\n< util::*\n").unwrap();
    let util_path = dep.join("src").join("util.rk");

    let mut ctx = crate::crate_system::CrateContext::new();
    let loaded = ctx
        .load_crate_with_dependencies_with_source_providers(
            app.clone(),
            &mut Vec::new(),
            vec![SourceProvider::Virtual {
                path: util_path.clone(),
                text: "answer = -> 42\n< answer\n".to_string(),
            }],
        )
        .expect("recursive dependency load should use configured source providers");

    assert_eq!(loaded, "app");
    let dep_source = ctx.source_crate("dep").expect("dep should be registered");
    assert!(dep_source.file_cache.contains_key(&util_path));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn load_crate_from_dir_reports_missing_lib_through_source_database() {
    let dir = std::env::temp_dir().join(format!("rock_crate_missing_lib_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("rock.toml"),
        "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/missing.rk\"\n",
    )
    .unwrap();

    let mut ctx = crate::crate_system::CrateContext::new();
    let error = ctx
        .load_crate_from_dir(dir.clone())
        .expect_err("missing source crate lib should fail through source loading");

    assert!(error.contains("Failed to read source file"), "{error}");
    assert!(error.contains("src/missing.rk"), "{error}");
    assert!(!error.contains("Lib path"), "{error}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn crate_context_adds_artifact_records_to_extern_store() {
    let mut ctx = CrateContext::new();
    let crate_id = CrateId(5);
    ctx.add_extern_crate(ExternCrateRecord::new(
        crate_id,
        "dep".to_string(),
        ExternCrateMetadata::empty_for_test(),
        ExternCrateBodies::default(),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    ))
    .expect("extern crate should insert");

    assert!(ctx.has_crate("dep"));
    assert!(!ctx.has_source_crate("dep"));
    assert!(ctx.has_extern_crate("dep"));
    assert_eq!(ctx.extern_crate("dep").unwrap().crate_id(), crate_id);
    assert_eq!(ctx.extern_crates().count(), 1);
}

#[test]
fn extern_crate_store_records_artifact_capabilities_by_session_crate_id() {
    let crate_id = CrateId(42);
    let answer_id = DefId::new(crate_id, LocalDefId(7));
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function(
        "dep::answer".to_string(),
        hir_function_with_id(answer_id, "answer", Some("dep::answer"), &[]),
    );
    interface.root_export_ids.insert(
        "answer".to_string(),
        ArtifactExport {
            source: "dep::answer".to_string(),
            id: answer_id,
        },
    );

    let mut resolver = ResolverTables::default();
    resolver
        .item_paths
        .insert("dep::answer".to_string(), answer_id);
    resolver
        .item_names_by_id
        .insert(answer_id, "dep::answer".to_string());

    let record = ExternCrateRecord::new(
        crate_id,
        "dep".to_string(),
        ExternCrateMetadata::new(
            interface,
            resolver,
            BTreeMap::from([(
                "pipe".to_string(),
                ArtifactExport {
                    source: "dep::prelude::pipe".to_string(),
                    id: answer_id,
                },
            )]),
        ),
        ExternCrateBodies::default(),
        ExternCrateLink::object(
            PathBuf::from("/dep/dep.o"),
            BTreeMap::from([(answer_id, "dep_answer".to_string())]),
        ),
    );
    let mut store = ExternCrateStore::default();
    store.insert(record).expect("extern record should insert");

    let dep = store.by_name("dep").expect("dep extern crate should exist");
    assert_eq!(dep.crate_id(), crate_id);
    assert_eq!(
        dep.metadata().interface().canonical_name(answer_id),
        Some("dep::answer")
    );
    assert_eq!(
        dep.metadata()
            .prelude_export_ids()
            .get("pipe")
            .map(|export| export.source.as_str()),
        Some("dep::prelude::pipe")
    );
    assert_eq!(dep.link().object_path(), Some(&PathBuf::from("/dep/dep.o")));
    assert_eq!(dep.link().backend_symbol(answer_id), Some("dep_answer"));
}

#[test]
fn extern_crate_store_rejects_duplicate_names_and_ids() {
    let mut store = ExternCrateStore::default();
    let first = ExternCrateRecord::new(
        CrateId(1),
        "dep".to_string(),
        ExternCrateMetadata::empty_for_test(),
        ExternCrateBodies::default(),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    );
    store
        .insert(first)
        .expect("first extern crate should insert");

    let duplicate_name = ExternCrateRecord::new(
        CrateId(2),
        "dep".to_string(),
        ExternCrateMetadata::empty_for_test(),
        ExternCrateBodies::default(),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    );
    let name_error = store
        .insert(duplicate_name)
        .expect_err("duplicate extern crate name should fail");
    assert!(name_error.contains("duplicate external crate name 'dep'"));

    let duplicate_id = ExternCrateRecord::new(
        CrateId(1),
        "other".to_string(),
        ExternCrateMetadata::empty_for_test(),
        ExternCrateBodies::default(),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    );
    let id_error = store
        .insert(duplicate_id)
        .expect_err("duplicate extern crate id should fail");
    assert!(id_error.contains("duplicate external crate id 1"));
}

#[test]
fn extern_crate_store_indexes_and_collects_object_link_inputs() {
    let metadata_id = CrateId(4);
    let object_id = CrateId(5);
    let mut store = ExternCrateStore::default();
    store
        .insert(ExternCrateRecord::new(
            metadata_id,
            "meta".to_string(),
            ExternCrateMetadata::empty_for_test(),
            ExternCrateBodies::default(),
            ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .expect("metadata-only extern crate should insert");
    store
        .insert(ExternCrateRecord::new(
            object_id,
            "obj".to_string(),
            ExternCrateMetadata::empty_for_test(),
            ExternCrateBodies::default(),
            ExternCrateLink::object(PathBuf::from("/dep/obj.o"), BTreeMap::new()),
        ))
        .expect("object-backed extern crate should insert");

    assert!(store.contains_name("meta"));
    assert!(store.contains_name("obj"));
    assert_eq!(store.by_crate_id(metadata_id).unwrap().name(), "meta");
    assert_eq!(store.by_crate_id(object_id).unwrap().name(), "obj");
    assert_eq!(
        store.iter().map(|dep| dep.name()).collect::<Vec<_>>(),
        vec!["meta", "obj"]
    );

    let inputs = store.link_inputs();
    assert_eq!(inputs.object_crate_names, vec!["obj".to_string()]);
    assert_eq!(inputs.object_paths, vec![PathBuf::from("/dep/obj.o")]);
}

#[test]
fn extern_crate_bodies_expose_categories_without_raw_bundle_access() {
    let id = DefId::new(CrateId(3), LocalDefId(1));
    let trait_id = DefId::new(CrateId(3), LocalDefId(2));
    let impl_id = DefId::new(CrateId(3), LocalDefId(3));
    let mut bundle = ArtifactCrossCrateHir {
        generic_functions: BTreeMap::new(),
        traits_with_defaults: BTreeMap::new(),
        generic_impls: Vec::new(),
    };
    bundle
        .generic_functions
        .insert(id, hir_function_with_id(id, "id", Some("dep::id"), &["T"]));
    bundle.traits_with_defaults.insert(
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Trait".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    bundle.generic_impls.push(HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::new(),
    });

    let bodies = ExternCrateBodies::from_cross_crate_hir(bundle);

    assert_eq!(
        bodies
            .providers()
            .generic_function(id)
            .expect("generic body provider")
            .name,
        "id"
    );
    assert!(bodies.providers().trait_with_defaults(trait_id).is_some());
    assert!(bodies.providers().generic_impl(impl_id).is_some());
}

#[test]
fn crate_context_collects_dependency_link_inputs_from_provider_capabilities() {
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(ExternCrateRecord::new(
        CrateId(9),
        "dep".to_string(),
        ExternCrateMetadata::empty_for_test(),
        ExternCrateBodies::default(),
        ExternCrateLink::object(PathBuf::from("/dep/dep.o"), BTreeMap::new()),
    ))
    .expect("extern crate should insert");

    let inputs = ctx.dependency_link_inputs();

    assert_eq!(inputs.object_crate_names, vec!["dep".to_string()]);
    assert_eq!(inputs.object_paths, vec![PathBuf::from("/dep/dep.o")]);
}

#[test]
fn extern_crate_link_makes_object_path_required_for_object_backed_dependencies() {
    let link = ExternCrateLink::object(PathBuf::from("/dep/dep.o"), BTreeMap::new());

    assert!(link.is_object_backed());
    assert_eq!(link.object_path(), Some(&PathBuf::from("/dep/dep.o")));
}

#[test]
fn extern_crate_ref_imported_function_symbol_requires_link_record() {
    let function_id = DefId::new(CrateId(9), LocalDefId(1));
    let function = hir_function_with_id(function_id, "answer", Some("dep::answer"), &[]);
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function("dep::answer".to_string(), function.clone());
    let metadata = ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new());
    let record = ExternCrateRecord::new(
        CrateId(9),
        "dep".to_string(),
        metadata,
        ExternCrateBodies::default(),
        ExternCrateLink::object(PathBuf::from("/dep/dep.o"), BTreeMap::new()),
    );
    let mut store = ExternCrateStore::default();
    store.insert(record).unwrap();
    let dep = store.by_name("dep").unwrap();

    assert_eq!(dep.imported_function_symbol("dep::answer", &function), None);
}

#[test]
fn extern_crate_ref_imported_impl_method_symbol_requires_link_record() {
    let impl_id = DefId::new(CrateId(9), LocalDefId(2));
    let method_id = DefId::new(CrateId(9), LocalDefId(3));
    let mut method = hir_function_with_id(method_id, "make", Some("dep::Box::make"), &[]);
    method.is_method = true;
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("make".to_string(), method.clone())]),
    };
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_impl(imp.clone());
    let metadata = ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new());
    let record = ExternCrateRecord::new(
        CrateId(9),
        "dep".to_string(),
        metadata,
        ExternCrateBodies::default(),
        ExternCrateLink::object(PathBuf::from("/dep/dep.o"), BTreeMap::new()),
    );
    let mut store = ExternCrateStore::default();
    store.insert(record).unwrap();
    let dep = store.by_name("dep").unwrap();

    assert!(!dep.provides_impl_method_body(&imp, &method));
    assert_eq!(dep.imported_impl_method_symbol(&imp, "make", &method), None);
}

#[test]
fn extern_crate_ref_imported_impl_method_allows_concrete_trait_args() {
    let impl_id = DefId::new(CrateId(9), LocalDefId(4));
    let method_id = DefId::new(CrateId(9), LocalDefId(5));
    let mut method = hir_function_with_id(method_id, "+", Some("dep::I64_+"), &[]);
    method.is_method = true;
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("I64".to_string()),
        type_name: "I64".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: Some("Add".to_string()),
        trait_id: Some(DefId::new(CrateId(9), LocalDefId(6))),
        trait_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: DefId::new(CrateId(9), LocalDefId(6)),
                index: 0,
            },
            "Rhs",
        )],
        trait_arg_types: vec![Type::I64],
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("+".to_string(), method.clone())]),
    };
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_impl(imp.clone());
    let metadata = ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new());
    let record = ExternCrateRecord::new(
        CrateId(9),
        "dep".to_string(),
        metadata,
        ExternCrateBodies::default(),
        ExternCrateLink::object(
            PathBuf::from("/dep/dep.o"),
            BTreeMap::from([(method_id, "dep_I64_add".to_string())]),
        ),
    );
    let mut store = ExternCrateStore::default();
    store.insert(record).unwrap();
    let dep = store.by_name("dep").unwrap();

    assert!(dep.provides_impl_method_body(&imp, &method));
    assert_eq!(
        dep.imported_impl_method_symbol(&imp, "+", &method),
        Some("dep_I64_add".to_string())
    );
}

#[test]
fn extern_crate_ref_concrete_impl_body_rejects_generic_methods_with_link_records() {
    let impl_id = DefId::new(CrateId(9), LocalDefId(7));
    let method_id = DefId::new(CrateId(9), LocalDefId(8));
    let mut method = hir_function_with_id(method_id, "make", Some("dep::Box::make"), &["T"]);
    method.is_method = true;
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("make".to_string(), method.clone())]),
    };
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_impl(imp.clone());
    let metadata = ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new());
    let record = ExternCrateRecord::new(
        CrateId(9),
        "dep".to_string(),
        metadata,
        ExternCrateBodies::default(),
        ExternCrateLink::object(
            PathBuf::from("/dep/dep.o"),
            BTreeMap::from([(method_id, "dep_Box_make".to_string())]),
        ),
    );
    let mut store = ExternCrateStore::default();
    store.insert(record).unwrap();
    let dep = store.by_name("dep").unwrap();

    assert!(!dep.provides_impl_body(&imp));
    assert!(!dep.provides_impl_method_body(&imp, &method));
    assert_eq!(dep.imported_impl_method_symbol(&imp, "make", &method), None);
}

#[test]
fn extern_crate_ref_imported_symbols_use_explicit_link_records() {
    let function_id = DefId::new(CrateId(9), LocalDefId(1));
    let function = hir_function_with_id(function_id, "answer", Some("dep::answer"), &[]);
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function("dep::answer".to_string(), function.clone());
    let metadata = ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new());
    let record = ExternCrateRecord::new(
        CrateId(9),
        "dep".to_string(),
        metadata,
        ExternCrateBodies::default(),
        ExternCrateLink::object(
            PathBuf::from("/dep/dep.o"),
            BTreeMap::from([(function_id, "dep_answer".to_string())]),
        ),
    );
    let mut store = ExternCrateStore::default();
    store.insert(record).unwrap();
    let dep = store.by_name("dep").unwrap();

    assert_eq!(
        dep.imported_function_symbol("dep::answer", &function),
        Some("dep_answer".to_string())
    );
}

#[test]
fn test_compilation_order_no_deps() {
    let mut ctx = CrateContext::new();

    let manifest = CrateManifest {
        crate_: CrateConfig {
            name: "crate_a".to_string(),
            version: "0.1.0".to_string(),
            no_std: false,
        },
        targets: vec![CrateTarget::Lib {
            path: "lib.rk".to_string(),
        }],
        dependencies: None,
    };

    let module = Module {
        name: None,
        top_levels: vec![],
        is_inline: false,
        filepath: None,
    };

    ctx.register_crate(manifest, PathBuf::from("/a"), module);
    let order = ctx.compilation_order().unwrap();
    assert_eq!(order, vec!["crate_a".to_string()]);
}

#[test]
fn test_compilation_order_with_deps() {
    let mut ctx = CrateContext::new();

    let manifest_a = CrateManifest {
        crate_: CrateConfig {
            name: "crate_a".to_string(),
            version: "0.1.0".to_string(),
            no_std: false,
        },
        targets: vec![CrateTarget::Lib {
            path: "lib.rk".to_string(),
        }],
        dependencies: None,
    };
    let module_a = Module {
        name: None,
        top_levels: vec![],
        is_inline: false,
        filepath: None,
    };
    ctx.register_crate(manifest_a, PathBuf::from("/a"), module_a);

    let manifest_b = CrateManifest {
        crate_: CrateConfig {
            name: "crate_b".to_string(),
            version: "0.1.0".to_string(),
            no_std: false,
        },
        targets: vec![CrateTarget::Lib {
            path: "lib.rk".to_string(),
        }],
        dependencies: {
            let mut deps = BTreeMap::new();
            deps.insert(
                "crate_a".to_string(),
                Dependency {
                    path: Some("../crate_a".to_string()),
                    version: None,
                },
            );
            Some(deps)
        },
    };
    let module_b = Module {
        name: None,
        top_levels: vec![],
        is_inline: false,
        filepath: None,
    };
    ctx.register_crate(manifest_b, PathBuf::from("/b"), module_b);

    let order = ctx.compilation_order().unwrap();
    assert_eq!(order, vec!["crate_a".to_string(), "crate_b".to_string()]);
}
