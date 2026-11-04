use super::*;

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
