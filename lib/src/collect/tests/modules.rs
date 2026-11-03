use super::*;

#[test]
fn collect_uses_source_graph_for_source_backed_modules() {
    let temp_dir =
        std::env::temp_dir().join(format!("rock_collect_source_graph_{}", std::process::id()));
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
    std::fs::remove_file(temp_dir.join("util.rk")).unwrap();
    let program = crate::ast::Program {
        module: graph.root_module().clone(),
    };

    let decls = collect_with_source_graph(
        &program,
        &graph,
        &crate::crate_system::CrateContext::new(),
        false,
        Some("demo"),
    )
    .expect("collection should use preloaded graph instead of reading util.rk again");

    assert!(function_named(&decls, "demo::util::answer").is_some());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_uses_source_graph_path_for_directory_modules() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_source_graph_dir_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(temp_dir.join("util")).unwrap();
    let entry = temp_dir.join("main.rk");
    std::fs::write(&entry, "mod util\n> util::answer\nmain = -> answer!\n").unwrap();
    std::fs::write(temp_dir.join("util/mod.rk"), "answer = -> 42\n< answer\n").unwrap();

    let config = crate::Config {
        entry_file: entry.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("demo".to_string()),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(entry, &config).unwrap();
    assert!(!temp_dir.join("util.rk").exists());
    let util_mod_path = temp_dir.join("util/mod.rk");
    let util_flat_path = temp_dir.join("util.rk");
    let program = crate::ast::Program {
        module: graph.root_module().clone(),
    };

    let decls = collect_with_source_graph(
        &program,
        &graph,
        &crate::crate_system::CrateContext::new(),
        false,
        Some("demo"),
    )
    .expect("collection should use graph path for util/mod.rk");

    assert!(function_named(&decls, "demo::util::answer").is_some());
    assert!(decls
        .loaded_module_paths
        .iter()
        .any(|(name, path)| name == "demo::util" && path == &util_mod_path));
    assert!(!decls
        .loaded_module_paths
        .iter()
        .any(|(name, path)| name == "demo::util" && path == &util_flat_path));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn nested_modules_resolve_later_nominals_and_collect_private_children() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_nested_nominals_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(temp_dir.join("api")).unwrap();
    std::fs::create_dir_all(temp_dir.join("model")).unwrap();
    let entry = temp_dir.join("main.rk");
    std::fs::write(&entry, "mod api\nmod model\n").unwrap();
    std::fs::write(
            temp_dir.join("api/mod.rk"),
            "mod state\n> demo::model::Value\n> demo::model::Choice\n> demo::api::state::State\n< struct Envelope\n    value: Choice Value\n    state: State\ntake: Choice Value -> Value\n< take = choice -> match choice\n    Choice::Some value => value\n",
        )
        .unwrap();
    std::fs::write(
        temp_dir.join("api/state.rk"),
        "< struct State\n    count: I64\n",
    )
    .unwrap();
    std::fs::write(
        temp_dir.join("model/mod.rk"),
        "< struct Value\n    number: I64\n< enum Choice T\n    Some T\n",
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
    let graph = db.load_entry(entry, &config).unwrap();
    let program = crate::ast::Program {
        module: graph.root_module().clone(),
    };
    let decls = collect_with_source_graph(
        &program,
        &graph,
        &crate::crate_system::CrateContext::new(),
        false,
        Some("demo"),
    )
    .expect("nested headers should resolve later nominal declarations");
    let state = decls
        .items
        .structs()
        .values()
        .find(|item| item.name == "State")
        .unwrap();
    assert_eq!(
        state.fields.len(),
        1,
        "private children need complete declarations"
    );
    let take = function_named(&decls, "demo::api::take").unwrap();
    assert!(
        take.generic_params.is_empty(),
        "imported types are not inferred generics"
    );
    assert!(matches!(take.params[0].ty, Type::Enum { .. }));
    assert!(matches!(take.ret_type, Type::Struct { .. }));
    let _ = std::fs::remove_dir_all(&temp_dir);
}

fn ident_expr(name: &str) -> crate::ast::Expression {
    crate::ast::Expression::UnaryExpr(crate::ast::UnaryExpr::PrimaryExpr(
        crate::ast::PrimaryExpr {
            operand: crate::ast::Operand::Ident(crate::ast::IdentifierPath {
                path: vec![crate::ast::IdentOrType::Ident(ident(name))],
            }),
            secondaries: None,
            type_annotation: None,
        },
    ))
}

#[test]
fn collect_indexes_loaded_source_backed_module_bodies() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_item_index_{}_{}",
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
    std::fs::write(&io_path, "struct Writer\n").unwrap();
    let (program, graph) = load_source_program(root_path, "mod io\n", Some("test"));
    let crate_ctx = CrateContext::new();

    let decls = collect_with_source_graph(&program, &graph, &crate_ctx, false, Some("test"))
        .expect("collect should load and index io module");

    let writer = decls
        .item_index
        .get(decls.item_index.defs_named("Writer")[0])
        .unwrap();
    assert_eq!(writer.module_id, ModuleId(1));
    assert_eq!(writer.def_id, DefId::new(CrateId(0), LocalDefId(1)));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_indexes_source_backed_module_nested_inside_inline_module() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_nested_item_index_{}_{}",
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
    std::fs::write(&io_path, "struct Writer\n").unwrap();
    let (_, graph) = load_source_program(root_path.clone(), "mod io\n", Some("math"));
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![inline_module(
                "math",
                vec![TopLevel::Mod(ident("io"), false)],
            )],
            is_inline: false,
            filepath: Some(root_path),
        },
    };
    let crate_ctx = CrateContext::new();

    let decls = collect_with_source_graph(&program, &graph, &crate_ctx, false, Some("test"))
        .expect("collect should load and index nested io module");

    let writer = decls
        .item_index
        .get(decls.item_index.defs_named("Writer")[0])
        .unwrap();
    assert_eq!(writer.module_id, ModuleId(2));
    assert_eq!(writer.def_id, DefId::new(CrateId(0), LocalDefId(2)));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_indexes_current_crate_prefixed_source_backed_module_inside_inline_module() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_current_crate_nested_item_index_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let root_path = temp_dir.join("main.rk");
    let math_path = temp_dir.join("math.rk");
    let io_path = temp_dir.join("io.rk");
    std::fs::write(&math_path, "mod io\n").unwrap();
    std::fs::write(&io_path, "struct Writer\n").unwrap();
    let (_, graph) = load_source_program(root_path.clone(), "mod math\n", Some("test"));
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![inline_module(
                "math",
                vec![TopLevel::Mod(ident("io"), false)],
            )],
            is_inline: false,
            filepath: Some(root_path),
        },
    };
    assert!(graph
        .loaded_module_paths()
        .iter()
        .any(|(name, path)| name == "test::math::io" && path == &io_path));

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("collect should use current-crate-prefixed graph module path");

    assert!(!decls
        .loaded_module_paths
        .iter()
        .any(|(name, _)| name == "math::io"));
    assert_eq!(decls.item_index.defs_named("Writer").len(), 1);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_gathers_local_inline_and_source_backed_declarations_without_lowerer_collect() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_local_only_{}_{}",
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
    std::fs::write(&io_path, "struct Writer\n").unwrap();
    let (_, graph) = load_source_program(root_path.clone(), "mod io\n", Some("test"));
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
                TopLevel::StructDecl(StructDecl {
                    name: type_inner("RootThing"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                }),
                inline_module(
                    "math",
                    vec![TopLevel::StructDecl(StructDecl {
                        name: type_inner("Vector"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    })],
                ),
                TopLevel::Mod(ident("io"), false),
            ],
            is_inline: false,
            filepath: Some(root_path),
        },
    };

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("collect should gather local declarations");

    assert!(struct_named(&decls, "RootThing").is_some());
    assert!(struct_named(&decls, "Vector").is_some());
    assert!(struct_named(&decls, "test::io::Writer").is_some());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_preserves_source_backed_module_function_qualification_and_import_aliases() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_source_backed_function_alias_{}_{}",
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

    let (program, graph) =
        load_source_program(root_path, "mod io\n> io::writer_name\n", Some("test"));

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("collect should preserve source-backed module function aliases");

    assert!(function_named(&decls, "test::io::writer_name").is_some());
    let writer_id = function_named(&decls, "test::io::writer_name").unwrap().id;
    assert_eq!(
        decls.resolver.import_aliases.get("writer_name"),
        Some(&writer_id)
    );
    assert!(function_named(&decls, "io::writer_name").is_none());
    assert!(decls
        .loaded_module_paths
        .iter()
        .any(|(name, path)| name == "test::io" && path == &io_path));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_resolves_source_backed_forward_sibling_trait_imports_in_impls() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_source_backed_forward_trait_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let root_path = temp_dir.join("main.rk");
    std::fs::write(
        temp_dir.join("early.rk"),
        "> demo::traits::Marker\n< struct Thing\nimpl Marker for Thing\n    @mark = -> 0\n",
    )
    .unwrap();
    std::fs::write(
        temp_dir.join("traits.rk"),
        "< trait Marker\n    @mark: I64\n",
    )
    .unwrap();

    let (program, graph) = load_source_program(
        root_path,
        "< mod early\n< mod traits\n< early::Thing\n",
        Some("demo"),
    );

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("demo"))
            .expect("collection should resolve later sibling trait imports before impl headers");
    assert!(trait_named(&decls, "demo::traits::Marker").is_some());
    let trait_id = trait_named(&decls, "demo::traits::Marker").unwrap().id;
    let marker_impl = decls
        .items
        .impls()
        .values()
        .find(|imp| imp.type_name == "Thing" && imp.trait_name.as_deref() == Some("Marker"))
        .expect("Marker impl for Thing should be collected");

    assert_eq!(marker_impl.trait_id, Some(trait_id));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_preserves_source_backed_parent_export_references_for_nested_modules() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_source_backed_parent_export_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(temp_dir.join("b")).unwrap();

    let root_path = temp_dir.join("main.rk");
    std::fs::write(temp_dir.join("b").join("mod.rk"), "mod a\n").unwrap();
    std::fs::write(temp_dir.join("b").join("a.rk"), "< struct Thing\n").unwrap();

    let (program, graph) = load_source_program(root_path, "mod b\n< b::a::Thing\n", Some("test"));

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("collect should preserve source-backed parent export references");

    assert!(struct_named(&decls, "test::b::a::Thing").is_some());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_keeps_nested_source_backed_imports_out_of_global_aliases() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_nested_source_backed_context_{}_{}",
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

    let (_, graph) = load_source_program(root_path.clone(), "mod io\n", Some("math"));
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![inline_module(
                "math",
                vec![
                    TopLevel::Mod(ident("io"), false),
                    TopLevel::Import(crate::ast::Path::Ident(crate::ast::IdentifierPath {
                        path: vec![
                            crate::ast::IdentOrType::Ident(ident("io")),
                            crate::ast::IdentOrType::Ident(ident("writer_name")),
                        ],
                    })),
                ],
            )],
            is_inline: false,
            filepath: Some(root_path),
        },
    };

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("collect should preserve nested source-backed module context");

    assert!(function_named(&decls, "math::io::writer_name").is_some());
    assert!(function_named(&decls, "test::io::writer_name").is_none());
    assert!(function_named(&decls, "test::math::io::writer_name").is_none());
    assert!(!decls.resolver.import_aliases.contains_key("writer_name"));
    assert_eq!(
        decls
            .loaded_module_paths
            .iter()
            .filter(|(name, path)| name == "math::io" && path == &io_path)
            .count(),
        1
    );
    assert_eq!(
        decls
            .loaded_module_paths
            .iter()
            .filter(|(name, path)| name == "test::io" && path == &io_path)
            .count(),
        0
    );
    assert!(decls
        .loaded_module_paths
        .iter()
        .any(|(name, path)| name == "math::io" && path == &io_path));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn lower_from_declarations_resolves_nested_source_backed_imports_inside_inline_module_bodies() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_nested_source_backed_context_{}_{}",
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
    let (_, graph) = load_source_program(root_path.clone(), "mod io\n", Some("math"));

    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![inline_module(
                "math",
                vec![
                    TopLevel::Mod(ident("io"), false),
                    TopLevel::Import(crate::ast::Path::Ident(crate::ast::IdentifierPath {
                        path: vec![
                            crate::ast::IdentOrType::Ident(ident("io")),
                            crate::ast::IdentOrType::Ident(ident("writer_name")),
                        ],
                    })),
                    TopLevel::FunctionDecl(function_decl_with_body_expr(
                        "use_writer",
                        ident_expr("writer_name"),
                    )),
                ],
            )],
            is_inline: false,
            filepath: Some(root_path),
        },
    };

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("collect should preserve nested source-backed module context");
    assert!(!decls.resolver.import_aliases.contains_key("writer_name"));
    let writer_id = function_named(&decls, "math::io::writer_name").unwrap().id;
    let use_writer_id = function_named(&decls, "math::use_writer").unwrap().id;

    let partial = crate::lower::program::lower_from_declarations(
        &program,
        decls,
        &CrateContext::new(),
        Some("test"),
    )
    .expect("lowering should resolve nested source-backed imports module-locally");

    let function = partial
        .functions
        .get(&use_writer_id)
        .expect("inline module body should lower under its qualified name");

    match function.body.stmts.as_slice() {
        [crate::hir::HirStmt::Expr(expr)] => match &expr.kind {
            crate::hir::HirExprKind::ResolvedVar(reference) => {
                assert_eq!(reference.name, "math::io::writer_name");
                assert_eq!(
                    reference.target,
                    crate::hir::HirVarTarget::Function(writer_id)
                );
            }
            other => panic!("expected lowered import to be a resolved var, got {other:?}"),
        },
        other => panic!("expected single expression body, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_keeps_root_qualified_local_imports_stable_inside_nested_modules() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
                inline_module(
                    "math",
                    vec![TopLevel::StructDecl(StructDecl {
                        name: type_inner("Vector"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    })],
                ),
                inline_module(
                    "inner",
                    vec![TopLevel::Import(crate::ast::Path::Type(
                        crate::ast::TypePath {
                            path: vec![
                                crate::ast::IdentOrType::Ident(ident("math")),
                                crate::ast::IdentOrType::Type(crate::ast::ParseType::Type(
                                    type_inner("Vector"),
                                )),
                            ],
                        },
                    ))],
                ),
            ],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("collect should preserve root-qualified local imports in nested modules");

    assert!(struct_named(&decls, "Vector").is_some());
    assert!(struct_named(&decls, "math::Vector").is_some());
    assert!(struct_named(&decls, "inner::math::Vector").is_none());
}

#[test]
fn collect_keeps_inline_qualified_aliases_in_local_namespace() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![inline_module(
                "math",
                vec![
                    TopLevel::StructDecl(StructDecl {
                        name: type_inner("Vector"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    TopLevel::FunctionDecl(crate::ast::FunctionDecl {
                        name: ident("sum"),
                        lambda: crate::ast::LambdaDecl {
                            parameters: vec![],
                            body: crate::ast::Block { statements: vec![] },
                            arrow_kind: crate::ast::LambdaArrowKind::Normal,
                            span: crate::lexer::Span::test(),
                        },
                        self_receiver: None,
                        is_unsafe: false,
                        exported: false,
                    }),
                ],
            )],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("collect should preserve inline qualified aliases");

    assert!(struct_named(&decls, "Vector").is_some());
    assert!(struct_named(&decls, "math::Vector").is_some());
    assert!(struct_named(&decls, "test::math::Vector").is_none());
    assert!(function_named(&decls, "sum").is_some());
    assert!(function_named(&decls, "math::sum").is_some());
    assert!(function_named(&decls, "test::math::sum").is_none());
}
