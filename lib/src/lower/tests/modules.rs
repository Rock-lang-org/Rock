use super::*;

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
