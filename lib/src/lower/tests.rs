use super::*;

use crate::ast::{Ident, IdentOrType, IdentifierPath, Program};
use crate::collect::item_index::{IndexingIds, ItemIndex};
use crate::collect::{DeclarationItems, DeclarationTypeVars, Declarations};
use crate::crate_artifact::ArtifactExport;
use crate::crate_system::CrateContext;
use crate::ids::{DefId, LocalDefId, ModuleId};

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: crate::lexer::Span::test(),
    }
}

fn test_function(id: DefId, name: &str, stmts: Vec<HirStmt>) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::I64,
        body: HirBlock {
            stmts,
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn declaration_items(
    functions: HashMap<DefId, HirFunction>,
    externs: HashMap<DefId, HirExtern>,
) -> DeclarationItems {
    DeclarationItems::from_id_maps(
        functions,
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        externs,
    )
    .unwrap()
}

fn declarations_with_items(resolver: ResolverTables, items: DeclarationItems) -> Declarations {
    Declarations {
        indexing_ids: IndexingIds::new_root(),
        item_index: ItemIndex::new(),
        resolver,
        current_def_ids: BTreeSet::new(),
        items,
        function_type_vars: HashMap::new(),
        infix_precedence: HashMap::new(),
        loaded_module_paths: Vec::new(),
        type_vars: DeclarationTypeVars::new(),
        inject_prelude: false,
        loaded_prelude_export_ids: HashMap::new(),
        module_file_cache: HashMap::new(),
        source_modules: crate::source_loader::SourceModuleSet::default(),
        dependency_root_export_ids: HashMap::new(),
        source_map: Default::default(),
        language_items: Default::default(),
    }
}

#[test]
fn lowerer_from_declarations_rejects_missing_canonical_item_name() {
    let function_id = DefId::new(CrateId(0), LocalDefId(1));
    let decls = declarations_with_items(
        ResolverTables::default(),
        declaration_items(
            HashMap::from([(
                function_id,
                test_function(function_id, "answer", Vec::new()),
            )]),
            HashMap::new(),
        ),
    );

    let errors = Lowerer::from_declarations(decls)
        .err()
        .expect("missing canonical names should be reported");

    assert!(errors[0]
        .message
        .contains("missing canonical function declaration name"));
}

#[test]
fn lowerer_from_declarations_reports_smallest_missing_canonical_id_regardless_of_insertion_order() {
    let first = DefId::new(CrateId(0), LocalDefId(1));
    let second = DefId::new(CrateId(0), LocalDefId(2));

    for ids in [[second, first], [first, second]] {
        let functions = ids
            .into_iter()
            .map(|id| (id, test_function(id, "missing", Vec::new())))
            .collect();
        let errors = Lowerer::from_declarations(declarations_with_items(
            ResolverTables::default(),
            declaration_items(functions, HashMap::new()),
        ))
        .err()
        .expect("missing canonical function names should be reported");

        assert_eq!(
            errors[0].message,
            format!("missing canonical function declaration name for DefId {first:?}")
        );
    }

    for ids in [[second, first], [first, second]] {
        let externs = ids.into_iter().map(|id| (id, test_extern(id))).collect();
        let errors = Lowerer::from_declarations(declarations_with_items(
            ResolverTables::default(),
            declaration_items(HashMap::new(), externs),
        ))
        .err()
        .expect("missing canonical extern names should be reported");

        assert_eq!(
            errors[0].message,
            format!("missing canonical extern declaration name for DefId {first:?}")
        );
    }
}

fn test_impl(id: DefId) -> HirImpl {
    HirImpl {
        id,
        owner: crate::hir::HirImplOwner::Named(format!("Type{}", id.local.0)),
        type_name: format!("Type{}", id.local.0),
        type_generics: vec![],
        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
        trait_name: None,
        trait_id: None,
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: HashMap::new().into(),
        methods: HashMap::new(),
    }
}

fn test_extern(id: DefId) -> HirExtern {
    HirExtern {
        id,
        name: format!("extern{}", id.local.0),
        params: vec![],
        ret: Type::Unit,
        variadic: false,
        is_unsafe: false,
    }
}

#[test]
fn lowerer_from_declarations_orders_impls_and_externs_by_def_id() {
    let first = DefId::new(CrateId(0), LocalDefId(1));
    let second = DefId::new(CrateId(0), LocalDefId(2));
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(first, "extern1".to_string());
    resolver
        .item_names_by_id
        .insert(second, "extern2".to_string());
    let items = DeclarationItems::from_id_maps(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(second, test_impl(second)), (first, test_impl(first))]),
        HashMap::from([(second, test_extern(second)), (first, test_extern(first))]),
    )
    .unwrap();

    let lowerer = Lowerer::from_declarations(declarations_with_items(resolver, items)).unwrap();

    assert_eq!(
        lowerer
            .items
            .impl_defs_in_order()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        vec![first, second]
    );
    assert_eq!(lowerer.items.extern_def(first).unwrap().id, first);
    assert_eq!(lowerer.items.extern_def(second).unwrap().id, second);
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

fn int_return(value: i64) -> HirStmt {
    HirStmt::Return(Some(HirExpr {
        kind: HirExprKind::IntLiteral(value),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    }))
}

fn identifier_path(names: &[&str]) -> IdentifierPath {
    IdentifierPath {
        path: names
            .iter()
            .map(|name| IdentOrType::Ident(ident(name)))
            .collect(),
    }
}

#[test]
fn prelude_imports_service_owns_enabled_state_and_captured_exports() {
    let print_id = DefId::new(CrateId(7), LocalDefId(3));
    let mut prelude = crate::lower::prelude::PreludeImports::new(false);

    assert!(!prelude.is_enabled());

    prelude.capture_loaded_prelude_exports(
        "std",
        &std::collections::BTreeMap::from([(
            "print".to_string(),
            ArtifactExport {
                source: "std::prelude::print".to_string(),
                id: print_id,
            },
        )]),
    );

    assert_eq!(prelude.export_source("print"), Some("std::prelude::print"));
    assert!(!Lowerer::with_options(false).prelude.is_enabled());
}

#[test]
fn lowerer_resolves_current_aliases_and_premerged_dependency_paths_through_facade() {
    let current_id = DefId::new(CrateId(0), LocalDefId(1));
    let dependency_item_id = DefId::new(CrateId(7), LocalDefId(2));
    let dependency_export_id = DefId::new(CrateId(7), LocalDefId(3));
    let mut lowerer = Lowerer::new_for_test();
    lowerer.resolver.insert_import_alias_with_name(
        "local_alias".to_string(),
        "demo::answer".to_string(),
        current_id,
    );

    let mut dependency = ResolverTables::default();
    dependency
        .item_paths
        .insert("dep::item".to_string(), dependency_item_id);
    dependency
        .item_names_by_id
        .insert(dependency_item_id, "dep::item".to_string());
    dependency.insert_export_alias_with_name(
        "dep::public".to_string(),
        "dep::internal::answer".to_string(),
        dependency_export_id,
    );
    lowerer.resolver.merge_global_inputs(&dependency);
    lowerer
        .dependency_resolvers
        .insert("dep".to_string(), dependency);

    assert_eq!(lowerer.resolve_item_def_id("local_alias"), Some(current_id));
    assert_eq!(
        lowerer.resolve_item_def_id("dep::item"),
        Some(dependency_item_id)
    );
    assert_eq!(
        lowerer
            .canonical_name_for_alias_or_item("dep::item")
            .as_deref(),
        Some("dep::item")
    );
    assert_eq!(
        lowerer.resolve_item_def_id("dep::public"),
        Some(dependency_export_id)
    );
    assert_eq!(
        lowerer
            .canonical_name_for_alias_or_item("dep::public")
            .as_deref(),
        Some("dep::internal::answer")
    );
}

#[test]
fn trait_by_name_prefers_resolver_alias_id() {
    let canonical_id = DefId::new(CrateId(0), LocalDefId(30));
    let collision_id = DefId::new(CrateId(0), LocalDefId(31));
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_trait_def(test_trait(canonical_id, "dep::Show"));
    lowerer
        .items
        .insert_trait_def(test_trait(collision_id, "other::Show"));
    lowerer.resolver.insert_import_alias_with_name(
        "Show".to_string(),
        "dep::Show".to_string(),
        canonical_id,
    );

    let trait_def = lowerer.trait_by_name("Show").unwrap();

    assert_eq!(trait_def.id, canonical_id);
}

#[test]
fn lower_uses_canonical_current_crate_import_aliases() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_canonical_import_alias_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let root_path = temp_dir.join("main.rk");
    let io_path = temp_dir.join("io.rk");
    std::fs::write(
        &io_path,
        "writer_name: I64\nwriter_name = -> 0\n< writer_name\n",
    )
    .unwrap();
    std::fs::write(&root_path, "mod io\n> io::writer_name\n").unwrap();

    let config = crate::Config {
        entry_file: root_path.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("test".to_string()),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(root_path, &config).unwrap();
    let program = Program {
        module: graph.root_module().clone(),
    };

    let decls = crate::collect::collect_with_source_graph(
        &program,
        &graph,
        &CrateContext::new(),
        false,
        Some("test"),
    )
    .expect("collect should resolve canonical import aliases");

    let lowerer = Lowerer::from_declarations(decls).unwrap();

    assert_eq!(
        lowerer
            .canonical_name_for_alias_or_item("writer_name")
            .as_deref(),
        Some("test::io::writer_name")
    );
    assert!(lowerer.scope.lookup("writer_name").is_some());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn lower_from_declarations_uses_preloaded_module_cache() {
    let temp_dir =
        std::env::temp_dir().join(format!("rock_lower_source_graph_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let entry = temp_dir.join("main.rk");
    std::fs::write(&entry, "mod util\n> util::answer\nmain = -> answer!\n").unwrap();
    std::fs::write(temp_dir.join("util.rk"), "answer = -> 42\n< answer\n").unwrap();

    let config = crate::Config {
        entry_file: entry.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("demo".to_string()),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(entry.clone(), &config).unwrap();
    let program = crate::ast::Program {
        module: graph.root_module().clone(),
    };
    let decls = crate::collect::collect_with_source_graph(
        &program,
        &graph,
        &crate::crate_system::CrateContext::new(),
        false,
        Some("demo"),
    )
    .unwrap();
    std::fs::remove_file(temp_dir.join("util.rk")).unwrap();

    let lowered = crate::lower::program::lower_from_declarations(
        &program,
        decls,
        &crate::crate_system::CrateContext::new(),
        Some("demo"),
    );

    assert!(lowered.is_ok(), "lowering should not re-read util.rk");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn lower_from_declarations_uses_loaded_path_for_directory_module() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_source_graph_dir_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(temp_dir.join("util")).unwrap();
    let entry = temp_dir.join("main.rk");
    std::fs::write(&entry, "mod util\n> util::answer\nmain = -> answer!\n").unwrap();
    std::fs::write(
        temp_dir.join("util").join("mod.rk"),
        "answer = -> 42\n< answer\n",
    )
    .unwrap();

    let config = crate::Config {
        entry_file: entry.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("demo".to_string()),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(entry.clone(), &config).unwrap();
    let program = crate::ast::Program {
        module: graph.root_module().clone(),
    };
    let decls = crate::collect::collect_with_source_graph(
        &program,
        &graph,
        &crate::crate_system::CrateContext::new(),
        false,
        Some("demo"),
    )
    .unwrap();
    std::fs::remove_dir_all(temp_dir.join("util")).unwrap();

    let lowered = crate::lower::program::lower_from_declarations(
        &program,
        decls,
        &crate::crate_system::CrateContext::new(),
        Some("demo"),
    )
    .expect("lowering should use cached directory module");

    assert!(
        lowered.functions[&lowered.resolver.item_paths["demo::util::answer"]]
            .body
            .stmts
            .len()
            > 0,
        "directory module body should be lowered from cache"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn lower_from_declarations_accepts_graph_only_current_crate_prefixed_nested_inline_module_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_current_crate_nested_inline_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let entry = temp_dir.join("main.rk");
    let math_path = temp_dir.join("math.rk");
    let io_path = temp_dir.join("io.rk");
    std::fs::write(&entry, "mod math\nmain = -> 0\n").unwrap();
    std::fs::write(&math_path, "mod io\n").unwrap();
    std::fs::write(&io_path, "answer: I64\nanswer = -> 42\n< answer\n").unwrap();

    let config = crate::Config {
        entry_file: entry.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("test".to_string()),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(entry.clone(), &config).unwrap();
    assert!(graph
        .loaded_module_paths()
        .iter()
        .any(|(name, path)| name == "test::math::io" && path == &io_path));

    let program = Program {
        module: crate::ast::Module {
            name: None,
            top_levels: vec![crate::ast::TopLevel::Module(crate::ast::ModuleDecl(
                crate::ast::Module {
                    name: Some(ident("math")),
                    top_levels: vec![crate::ast::TopLevel::Mod(ident("io"), false)],
                    is_inline: true,
                    filepath: None,
                },
            ))],
            is_inline: false,
            filepath: Some(entry),
        },
    };
    let mut decls = crate::collect::collect_with_source_graph(
        &program,
        &graph,
        &crate::crate_system::CrateContext::new(),
        false,
        Some("test"),
    )
    .expect("collect should resolve nested inline source-backed module");
    decls.loaded_module_paths = graph.loaded_module_paths();
    assert!(!decls
        .loaded_module_paths
        .iter()
        .any(|(name, _)| name == "math::io"));

    let lowered = crate::lower::program::lower_from_declarations(
        &program,
        decls,
        &crate::crate_system::CrateContext::new(),
        Some("test"),
    )
    .expect("lowering should use the current-crate-prefixed source graph path");

    assert!(
        lowered.functions[&lowered.resolver.item_paths["math::io::answer"]]
            .body
            .stmts
            .len()
            > 0,
        "nested source-backed module body should be lowered"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn lower_loaded_module_bodies_preserves_paths_for_directory_glob_imports() {
    let temp_dir =
        std::env::temp_dir().join(format!("rock_lower_loaded_dir_glob_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(temp_dir.join("helper")).unwrap();
    let entry = temp_dir.join("main.rk");
    std::fs::write(&entry, "mod parent\nmain = -> parent::use_answer!\n").unwrap();
    std::fs::write(
        temp_dir.join("parent.rk"),
        "< mod helper\n> helper::*\nuse_answer: I64\nuse_answer = -> answer!\n< use_answer\n",
    )
    .unwrap();
    std::fs::write(
        temp_dir.join("helper").join("mod.rk"),
        "answer: I64\nanswer = -> 42\n< answer\n",
    )
    .unwrap();

    let config = crate::Config {
        entry_file: entry.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("demo".to_string()),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(entry.clone(), &config).unwrap();
    let program = crate::ast::Program {
        module: graph.root_module().clone(),
    };
    let decls = crate::collect::collect_with_source_graph(
        &program,
        &graph,
        &crate::crate_system::CrateContext::new(),
        false,
        Some("demo"),
    )
    .unwrap();
    assert!(
        decls.items.functions().values().any(|function| {
            decls
                .resolver
                .item_names_by_id
                .get(&function.id)
                .map(String::as_str)
                == Some("demo::parent::helper::answer")
        }),
        "functions: {:?}",
        decls.resolver.item_names_by_id.values().collect::<Vec<_>>()
    );

    let lowered = crate::lower::program::lower_from_declarations(
        &program,
        decls,
        &crate::crate_system::CrateContext::new(),
        Some("demo"),
    )
    .expect("loaded module body should resolve directory-backed glob import");

    assert!(
        lowered.functions[&lowered.resolver.item_paths["demo::parent::use_answer"]]
            .body
            .stmts
            .len()
            > 0,
        "loaded module body should be lowered"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn lower_loaded_module_trait_defaults_does_not_skip_local_lib_module() {
    let temp_dir =
        std::env::temp_dir().join(format!("rock_lower_local_lib_mod_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let entry = temp_dir.join("main.rk");
    std::fs::write(&entry, "mod lib\n> lib::Animal\nmain = -> 0\n").unwrap();
    std::fs::write(
        temp_dir.join("lib.rk"),
        "trait Animal\n    @speak = -> 4\n< Animal\n",
    )
    .unwrap();

    let config = crate::Config {
        entry_file: entry.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("demo".to_string()),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(entry.clone(), &config).unwrap();
    let program = crate::ast::Program {
        module: graph.root_module().clone(),
    };
    let decls = crate::collect::collect_with_source_graph(
        &program,
        &graph,
        &crate::crate_system::CrateContext::new(),
        false,
        Some("demo"),
    )
    .unwrap();

    let lowered = crate::lower::program::lower_from_declarations(
        &program,
        decls,
        &crate::crate_system::CrateContext::new(),
        Some("demo"),
    )
    .expect("local lib module should lower trait defaults");

    assert!(
        lowered.traits[&lowered.resolver.item_paths["demo::lib::Animal"]].methods["speak"]
            .body
            .stmts
            .len()
            > 0,
        "trait default body in local lib module should be lowered"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn lower_loaded_modules_attach_same_named_trait_defaults_to_exact_ids() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_loaded_same_trait_names_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let entry = temp_dir.join("main.rk");
    std::fs::write(&entry, "mod left\nmod right\nmain = -> 0\n").unwrap();
    std::fs::write(
        temp_dir.join("left.rk"),
        "trait Shared\n    @value = -> 1\n",
    )
    .unwrap();
    std::fs::write(
        temp_dir.join("right.rk"),
        "trait Shared\n    @value = -> true\n",
    )
    .unwrap();

    let config = crate::Config {
        entry_file: entry.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("demo".to_string()),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(entry.clone(), &config).unwrap();
    let program = crate::ast::Program {
        module: graph.root_module().clone(),
    };
    let crate_ctx = crate::crate_system::CrateContext::new();
    let decls = crate::collect::collect_with_source_graph(
        &program,
        &graph,
        &crate_ctx,
        false,
        Some("demo"),
    )
    .unwrap();

    let lowered =
        crate::lower::program::lower_from_declarations(&program, decls, &crate_ctx, Some("demo"))
            .expect("loaded modules with same-named traits should lower");
    let left_trait_id = lowered.resolver.item_paths["demo::left::Shared"];
    let right_trait_id = lowered.resolver.item_paths["demo::right::Shared"];

    assert_ne!(left_trait_id, right_trait_id);
    assert_eq!(
        lowered.traits[&left_trait_id].methods["value"].body.ty,
        Type::I64
    );
    assert_eq!(
        lowered.traits[&right_trait_id].methods["value"].body.ty,
        Type::Bool
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn glob_import_targets_uses_cached_nested_source_module() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_nested_glob_cache_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let entry = temp_dir.join("main.rk");
    std::fs::write(&entry, "mod foo\nmain = -> 0\n").unwrap();
    std::fs::write(temp_dir.join("foo.rk"), "mod bar\n").unwrap();
    std::fs::write(temp_dir.join("bar.rk"), "answer = -> 42\n< answer\n").unwrap();

    let config = crate::Config {
        entry_file: entry.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("demo".to_string()),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(entry.clone(), &config).unwrap();
    let mut lowerer = Lowerer::new_for_test();
    lowerer.modules =
        crate::lower::services::LowerModuleService::from_source_modules(graph.source_modules());
    lowerer
        .modules
        .configure_current_crate(Some("demo"), Some(&entry));

    let targets = lowerer
        .glob_import_targets(&["foo".to_string(), "bar".to_string()])
        .expect("nested file-backed module glob should resolve from cache");

    assert_eq!(
        targets,
        vec![("answer".to_string(), "demo::foo::bar::answer".to_string())]
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn lower_from_declarations_reports_missing_source_module() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_missing_source_backed_module_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let root_path = temp_dir.join("main.rk");
    let helper_path = temp_dir.join("helper.rk");
    std::fs::write(&root_path, "mod helper\nmain = -> 0\n").unwrap();
    std::fs::write(&helper_path, "answer = -> 7\n").unwrap();

    let config = crate::Config {
        entry_file: root_path.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("demo".to_string()),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(root_path.clone(), &config).unwrap();
    let program = Program {
        module: graph.root_module().clone(),
    };
    let helper_span = match &program.module.top_levels[0] {
        crate::ast::TopLevel::Mod(ident, _) => ident.span.clone(),
        _ => panic!("root module should retain the helper declaration"),
    };
    let mut decls = crate::collect::collect_with_source_graph(
        &program,
        &graph,
        &crate::crate_system::CrateContext::new(),
        false,
        Some("demo"),
    )
    .expect("collection should index the root and helper modules");
    assert!(decls
        .item_index
        .child_module_id(ModuleId(0), "helper")
        .is_some());
    decls.module_file_cache.clear();
    decls.source_modules = crate::source_loader::SourceModuleSet::default();
    decls.loaded_module_paths.clear();

    let result = crate::lower::program::lower_from_declarations(
        &program,
        decls,
        &crate::crate_system::CrateContext::new(),
        Some("demo"),
    );
    let errors = match result {
        Ok(_) => panic!("missing cached module should produce a resolve error"),
        Err(errors) => errors,
    };

    let error = errors
        .iter()
        .find(|error| {
            error
                .message
                .contains("Module 'demo::helper' was not loaded by the source database")
        })
        .expect("missing source-backed module should report its declaration");
    let span = error
        .span()
        .expect("missing source-backed module error should be span-aware");
    assert_eq!(span.file_path, helper_span.file_path);
    assert_eq!(span.start, helper_span.start);
    assert_eq!(span.end, helper_span.end);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn lowerer_from_declarations_preserves_dependency_import_alias_ids() {
    let dependency_id = DefId::new(CrateId(7), LocalDefId(3));
    let mut resolver = ResolverTables::default();
    resolver
        .import_aliases
        .insert("answer".to_string(), dependency_id);
    resolver
        .item_names_by_id
        .insert(dependency_id, "dep::answer".to_string());

    let decls = Declarations {
        indexing_ids: IndexingIds::new_root(),
        item_index: ItemIndex::new(),
        resolver,
        current_def_ids: BTreeSet::new(),
        items: declaration_items(HashMap::new(), HashMap::new()),
        function_type_vars: HashMap::new(),
        infix_precedence: HashMap::new(),
        loaded_module_paths: Vec::new(),
        type_vars: DeclarationTypeVars::new(),
        inject_prelude: false,
        loaded_prelude_export_ids: HashMap::new(),
        module_file_cache: HashMap::new(),
        source_modules: crate::source_loader::SourceModuleSet::default(),
        dependency_root_export_ids: HashMap::new(),
        source_map: Default::default(),
        language_items: Default::default(),
    };

    let lowerer = Lowerer::from_declarations(decls).unwrap();

    assert_eq!(
        lowerer
            .canonical_name_for_alias_or_item("answer")
            .as_deref(),
        Some("dep::answer")
    );
    assert_eq!(
        lowerer.resolver.import_aliases.get("answer").copied(),
        Some(dependency_id)
    );
}

#[test]
fn lowerer_from_declarations_import_alias_lowers_to_resolved_function() {
    let dependency_id = DefId::new(CrateId(7), LocalDefId(3));
    let mut resolver = ResolverTables::default();
    resolver
        .import_aliases
        .insert("answer".to_string(), dependency_id);
    resolver
        .item_names_by_id
        .insert(dependency_id, "dep::answer".to_string());

    let decls = Declarations {
        indexing_ids: IndexingIds::new_root(),
        item_index: ItemIndex::new(),
        resolver,
        current_def_ids: BTreeSet::new(),
        items: declaration_items(
            HashMap::from([(
                dependency_id,
                test_function(dependency_id, "dep::answer", Vec::new()),
            )]),
            HashMap::new(),
        ),
        function_type_vars: HashMap::new(),
        infix_precedence: HashMap::new(),
        loaded_module_paths: Vec::new(),
        type_vars: DeclarationTypeVars::new(),
        inject_prelude: false,
        loaded_prelude_export_ids: HashMap::new(),
        module_file_cache: HashMap::new(),
        source_modules: crate::source_loader::SourceModuleSet::default(),
        dependency_root_export_ids: HashMap::new(),
        source_map: Default::default(),
        language_items: Default::default(),
    };
    let mut lowerer = Lowerer::from_declarations(decls).unwrap();

    let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

    match expr.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "dep::answer");
            assert_eq!(reference.target, HirVarTarget::Function(dependency_id));
        }
        other => panic!("expected resolved import alias, got {other:?}"),
    }
}

#[test]
fn lowerer_from_declarations_extern_import_alias_lowers_to_resolved_extern() {
    let dependency_id = DefId::new(CrateId(7), LocalDefId(4));
    let mut resolver = ResolverTables::default();
    resolver
        .import_aliases
        .insert("puts".to_string(), dependency_id);
    resolver
        .item_names_by_id
        .insert(dependency_id, "dep::puts".to_string());

    let decls = Declarations {
        indexing_ids: IndexingIds::new_root(),
        item_index: ItemIndex::new(),
        resolver,
        current_def_ids: BTreeSet::new(),
        items: declaration_items(
            HashMap::new(),
            HashMap::from([(
                dependency_id,
                HirExtern {
                    id: dependency_id,
                    name: "dep::puts".to_string(),
                    params: vec![Type::I32],
                    ret: Type::I32,
                    variadic: false,
                    is_unsafe: false,
                },
            )]),
        ),
        function_type_vars: HashMap::new(),
        infix_precedence: HashMap::new(),
        loaded_module_paths: Vec::new(),
        type_vars: DeclarationTypeVars::new(),
        inject_prelude: false,
        loaded_prelude_export_ids: HashMap::new(),
        module_file_cache: HashMap::new(),
        source_modules: crate::source_loader::SourceModuleSet::default(),
        dependency_root_export_ids: HashMap::new(),
        source_map: Default::default(),
        language_items: Default::default(),
    };
    let mut lowerer = Lowerer::from_declarations(decls).unwrap();

    let expr = lowerer.lower_identifier_path(&identifier_path(&["puts"]));

    match expr.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "dep::puts");
            assert_eq!(reference.target, HirVarTarget::Extern(dependency_id));
        }
        other => panic!("expected resolved extern import alias, got {other:?}"),
    }
}

#[test]
fn injected_prelude_scope_only_extern_alias_lowers_to_resolved_extern() {
    let puts_id = DefId::new(CrateId(7), LocalDefId(4));
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_extern(HirExtern {
        id: puts_id,
        name: "std::libc::puts".to_string(),
        params: vec![Type::I32],
        ret: Type::I32,
        variadic: false,
        is_unsafe: false,
    });
    lowerer.scope.define(
        "std::libc::puts".to_string(),
        Type::function(vec![Type::I32], Type::I32),
        false,
    );
    lowerer.prelude.capture_loaded_prelude_exports(
        "std",
        &std::collections::BTreeMap::from([(
            "puts".to_string(),
            ArtifactExport {
                source: "std::libc::puts".to_string(),
                id: puts_id,
            },
        )]),
    );
    let errors = lowerer.prelude.inject_loaded_prelude(
        &mut lowerer.scope,
        &mut lowerer.items,
        &mut lowerer.resolver,
    );
    assert!(errors.is_empty(), "unexpected prelude errors: {errors:?}");

    let expr = lowerer.lower_identifier_path(&identifier_path(&["puts"]));

    match expr.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "std::libc::puts");
            assert_eq!(reference.target, HirVarTarget::Extern(puts_id));
        }
        other => panic!("expected resolved prelude extern alias, got {other:?}"),
    }
}

#[test]
fn glob_import_targets_uses_dependency_resolver_exports() {
    let dependency_id = DefId::new(CrateId(7), LocalDefId(3));
    let mut dep_resolver = ResolverTables::default();
    dep_resolver.insert_export_alias_with_name(
        "answer".to_string(),
        "dep::answer".to_string(),
        dependency_id,
    );
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .dependency_resolvers
        .insert("dep".to_string(), dep_resolver);

    let targets = lowerer
        .glob_import_targets(&["dep".to_string()])
        .expect("dependency resolver glob should resolve from export aliases");

    assert_eq!(
        targets,
        vec![("answer".to_string(), "dep::answer".to_string())]
    );
}

#[test]
fn handle_glob_import_records_dependency_resolver_export_ids() {
    let dependency_id = DefId::new(CrateId(7), LocalDefId(3));
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .items
        .insert_function(test_function(dependency_id, "dep::answer", Vec::new()));
    let mut dep_resolver = ResolverTables::default();
    dep_resolver.insert_export_alias_with_name(
        "answer".to_string(),
        "dep::answer".to_string(),
        dependency_id,
    );
    lowerer
        .dependency_resolvers
        .insert("dep".to_string(), dep_resolver);

    lowerer.handle_glob_import(&["dep".to_string()]);

    assert_eq!(
        lowerer.resolver.import_aliases.get("answer").copied(),
        Some(dependency_id)
    );
    assert_eq!(
        lowerer
            .resolver
            .item_names_by_id
            .get(&dependency_id)
            .map(String::as_str),
        Some("dep::answer")
    );
}

#[test]
fn injected_prelude_function_alias_remains_id_backed_view() {
    let prelude_id = DefId::new(CrateId(7), LocalDefId(3));
    let mut lowerer = Lowerer::new_for_test();
    lowerer.prelude.capture_loaded_prelude_exports(
        "std",
        &std::collections::BTreeMap::from([(
            "answer".to_string(),
            ArtifactExport {
                source: "std::answer".to_string(),
                id: prelude_id,
            },
        )]),
    );
    lowerer.items.insert_function(test_function(
        prelude_id,
        "std::answer",
        vec![int_return(42)],
    ));

    let errors = lowerer.prelude.inject_loaded_prelude(
        &mut lowerer.scope,
        &mut lowerer.items,
        &mut lowerer.resolver,
    );

    assert!(errors.is_empty(), "unexpected prelude errors: {errors:?}");
    assert_eq!(
        lowerer.resolver.import_aliases.get("answer").copied(),
        Some(prelude_id)
    );
    assert_eq!(lowerer.items.functions().count(), 1);
    assert_eq!(
        lowerer.items.function(prelude_id).unwrap().body.stmts.len(),
        1
    );
}

#[test]
fn lowerer_from_declarations_preserves_real_local_extern_id_zero() {
    let extern_id = DefId::new(CrateId(0), LocalDefId(0));
    let collision_id = DefId::new(CrateId(0), LocalDefId(1));
    let mut resolver = ResolverTables::default();
    resolver.item_paths.insert("puts".to_string(), collision_id);
    resolver
        .item_names_by_id
        .insert(extern_id, "demo::puts".to_string());

    let decls = Declarations {
        indexing_ids: IndexingIds::new_root(),
        item_index: ItemIndex::new(),
        resolver,
        current_def_ids: BTreeSet::from([extern_id]),
        items: declaration_items(
            HashMap::new(),
            HashMap::from([(
                extern_id,
                HirExtern {
                    id: extern_id,
                    name: "demo::puts".to_string(),
                    params: vec![Type::Pointer(Box::new(Type::U8))],
                    ret: Type::I32,
                    variadic: false,
                    is_unsafe: false,
                },
            )]),
        ),
        function_type_vars: HashMap::new(),
        infix_precedence: HashMap::new(),
        loaded_module_paths: Vec::new(),
        type_vars: DeclarationTypeVars::new(),
        inject_prelude: false,
        loaded_prelude_export_ids: HashMap::new(),
        module_file_cache: HashMap::new(),
        source_modules: crate::source_loader::SourceModuleSet::default(),
        dependency_root_export_ids: HashMap::new(),
        source_map: Default::default(),
        language_items: Default::default(),
    };

    let lowerer = Lowerer::from_declarations(decls).unwrap();

    assert_eq!(lowerer.items.extern_def(extern_id).unwrap().id, extern_id);
}

#[test]
fn lowerer_construction_uses_explicit_service_wrappers() {
    let services = crate::lower::services::LowererServices::new(true);
    let lowerer = Lowerer::from_services(services);

    assert!(lowerer.prelude.is_enabled());
    assert!(lowerer.diagnostics.is_empty());
    assert!(lowerer.items.functions().next().is_none());
}

#[test]
fn body_lowering_context_owns_body_local_ids_and_restores_lowerer_state() {
    let mut lowerer = Lowerer::new_for_test();
    let outer_owner = DefId::new(CrateId(0), LocalDefId(99));
    lowerer.generic_context = Some(crate::lower::body_context::GenericLoweringContext::new(
        outer_owner,
        vec!["Outer".to_string()],
    ));

    let body_owner = DefId::new(CrateId(0), LocalDefId(7));
    let context = crate::lower::body_context::BodyLoweringContext::new(
        "body".to_string(),
        BodyOwner::Function(body_owner),
        Some(body_owner),
        vec!["T".to_string()],
        HirGenericBounds::new(),
        true,
    );

    let ids = lowerer.with_body_context(context, |lowerer| {
        assert_eq!(lowerer.current_function_name(), Some("body"));
        assert_eq!(
            lowerer.current_body_owner(),
            Some(BodyOwner::Function(body_owner))
        );
        assert_eq!(lowerer.current_generic_owner(), Some(body_owner));
        assert_eq!(lowerer.current_generic_params(), &["T".to_string()]);
        assert!(lowerer.is_in_unsafe());
        (lowerer.fresh_local_id(), lowerer.fresh_local_id())
    });

    assert_eq!(ids.0, HirLocalId(0));
    assert_eq!(ids.1, HirLocalId(1));
    assert_eq!(lowerer.current_generic_owner(), Some(outer_owner));
    assert_eq!(lowerer.current_generic_params(), &["Outer".to_string()]);
    assert!(!lowerer.is_in_unsafe());
    assert!(lowerer.current_function_name().is_none());
    assert!(lowerer.current_body_owner().is_none());
}

#[test]
fn body_owner_mismatched_method_name_and_id_does_not_rediscover_method() {
    let mut lowerer = Lowerer::new_for_test();
    let impl_id = DefId::new(CrateId(0), LocalDefId(20));
    let first_method_id = DefId::new(CrateId(0), LocalDefId(21));
    let second_method_id = DefId::new(CrateId(0), LocalDefId(22));
    let mut first = test_function(first_method_id, "first", Vec::new());
    first.ret_type = Type::I32;
    let mut second = test_function(second_method_id, "second", Vec::new());
    second.ret_type = Type::Bool;
    let mut impl_def = test_impl(impl_id);
    impl_def.methods.insert("first".to_string(), first);
    impl_def.methods.insert("second".to_string(), second);
    lowerer.items.insert_impl(impl_def).unwrap();

    let context = BodyLoweringContext::new(
        "first".to_string(),
        BodyOwner::ImplMethod {
            impl_id,
            method_id: second_method_id,
            method_name: "first".to_string(),
        },
        Some(impl_id),
        Vec::new(),
        HirGenericBounds::new(),
        false,
    );

    let return_type =
        lowerer.with_body_context(context, |lowerer| lowerer.current_body_return_type());

    assert_eq!(return_type, None);
    assert!(!lowerer.diagnostics.is_empty());
}
