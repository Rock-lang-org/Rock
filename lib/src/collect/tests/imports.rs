use super::*;

fn function_at_canonical_name<'a>(
    decls: &'a Declarations,
    canonical_name: &str,
) -> Option<&'a HirFunction> {
    decls
        .resolver
        .item_paths
        .get(canonical_name)
        .and_then(|id| decls.items.functions().get(id))
}

#[test]
fn collect_reports_unresolved_import() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![TopLevel::Import(crate::ast::Path::Ident(
                crate::ast::IdentifierPath {
                    path: vec![
                        crate::ast::IdentOrType::Ident(ident("missing")),
                        crate::ast::IdentOrType::Ident(ident("value")),
                    ],
                },
            ))],
            is_inline: false,
            filepath: None,
        },
    };

    let errors = match collect(&program, &CrateContext::new(), false, Some("test")) {
        Ok(_) => panic!("an import that matches nothing must be an error"),
        Err(errors) => errors,
    };

    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message, "unresolved import 'missing::value'");
}

#[test]
fn resolver_contract_preserves_inline_function_signature_metadata_by_qualified_id() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![inline_module(
                "inner",
                vec![
                    TopLevel::FunctionSig(function_sig(
                        "id",
                        ParseType::Function(vec![ParseType::Type(type_inner("I64"))]),
                    )),
                    TopLevel::FunctionDecl(function_decl("id")),
                ],
            )],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("inline signature-backed function should collect");
    let function = function_at_canonical_name(&decls, "inner::id")
        .expect("qualified inline function should be collected");

    assert_eq!(function.ret_type, Type::I64);
    assert_eq!(function.name, "id");
    assert_eq!(
        decls
            .resolver
            .item_names_by_id
            .get(&function.id)
            .map(String::as_str),
        Some("inner::id")
    );
    assert!(decls.function_type_vars.contains_key(&function.id));
    assert_eq!(decls.function_type_vars.len(), 1);
    assert_eq!(
        decls.resolver.item_paths.get("inner::id"),
        Some(&function.id)
    );
}

#[test]
fn resolver_contract_preserves_inline_function_body_type_vars_by_qualified_id() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![inline_module(
                "inner",
                vec![TopLevel::FunctionDecl(single_param_function_decl(
                    "identity", "value",
                ))],
            )],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("inline function should collect");
    let function = function_at_canonical_name(&decls, "inner::identity")
        .expect("qualified inline function should be collected");

    assert_eq!(function.name, "identity");
    assert_eq!(
        decls
            .resolver
            .item_names_by_id
            .get(&function.id)
            .map(String::as_str),
        Some("inner::identity")
    );
    assert!(decls.function_type_vars.contains_key(&function.id));
    assert_eq!(decls.function_type_vars.len(), 1);
    assert_eq!(
        decls.resolver.item_paths.get("inner::identity"),
        Some(&function.id)
    );
}

#[test]
fn resolver_contract_preserves_root_function_metadata_when_inline_name_collides() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
                TopLevel::FunctionSig(function_sig(
                    "identity",
                    ParseType::Function(vec![ParseType::Type(type_inner("I64"))]),
                )),
                TopLevel::FunctionDecl(function_decl("identity")),
                inline_module(
                    "inner",
                    vec![TopLevel::FunctionDecl(single_param_function_decl(
                        "identity", "value",
                    ))],
                ),
            ],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("same-name root and inline functions should collect");
    let root = function_at_canonical_name(&decls, "identity")
        .expect("root function should remain collected");
    let inner = function_at_canonical_name(&decls, "inner::identity")
        .expect("qualified inline function should be collected");

    assert_ne!(root.id, inner.id);
    assert_eq!(root.params.len(), 0);
    assert_eq!(inner.params.len(), 1);
    assert_eq!(root.ret_type, Type::I64);
    assert_eq!(root.name, "identity");
    assert_eq!(inner.name, "identity");
    assert_eq!(
        decls
            .resolver
            .item_names_by_id
            .get(&inner.id)
            .map(String::as_str),
        Some("inner::identity")
    );
    assert!(decls.function_type_vars.contains_key(&root.id));
    assert!(decls.function_type_vars.contains_key(&inner.id));
    assert_eq!(decls.resolver.item_paths.get("identity"), Some(&root.id));
    assert_eq!(
        decls.resolver.item_paths.get("inner::identity"),
        Some(&inner.id)
    );
}

#[test]
fn collect_records_std_prelude_alias_by_canonical_def_id() {
    let print_id = DefId::new(CrateId(8), LocalDefId(3));
    let mut print = hir_function("print");
    print.id = print_id;

    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function("std::prelude::print".to_string(), print);

    let mut resolver = crate::collect::resolver::ResolverTables::default();
    resolver
        .item_paths
        .insert("std::prelude::print".to_string(), print_id);
    resolver
        .item_names_by_id
        .insert(print_id, "std::prelude::print".to_string());
    let prelude_export_ids = BTreeMap::from([(
        "print".to_string(),
        ArtifactExport {
            source: "std::prelude::print".to_string(),
            id: print_id,
        },
    )]);

    let mut crate_ctx = CrateContext::new();
    add_artifact_extern_crate(
        &mut crate_ctx,
        "std",
        interface,
        resolver,
        prelude_export_ids,
    );
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &crate_ctx, true, Some("test"))
        .expect("collect should record std prelude alias IDs");

    assert_eq!(decls.resolver.import_aliases.get("print"), Some(&print_id));
    assert_eq!(
        decls.resolver.item_names_by_id.get(&print_id),
        Some(&"std::prelude::print".to_string())
    );
}

#[test]
fn collect_preserves_non_std_prelude_capability_without_unqualified_injection() {
    let util_id = DefId::new(CrateId(9), LocalDefId(3));
    let mut util = hir_function("util");
    util.id = util_id;

    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function("dep::prelude::util".to_string(), util);

    let mut resolver = crate::collect::resolver::ResolverTables::default();
    resolver
        .item_paths
        .insert("dep::prelude::util".to_string(), util_id);
    resolver
        .item_names_by_id
        .insert(util_id, "dep::prelude::util".to_string());
    let prelude_export_ids = BTreeMap::from([(
        "util".to_string(),
        ArtifactExport {
            source: "dep::prelude::util".to_string(),
            id: util_id,
        },
    )]);

    let mut crate_ctx = CrateContext::new();
    add_artifact_extern_crate(
        &mut crate_ctx,
        "dep",
        interface,
        resolver,
        prelude_export_ids,
    );
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &crate_ctx, true, Some("test"))
        .expect("collect should preserve non-std prelude providers without injection");

    assert!(decls.resolver.import_aliases.get("util").is_none());
    assert!(decls.items.functions().contains_key(&util_id));
    assert_eq!(decls.items.functions().len(), 1);
}

#[test]
fn collect_resolves_local_import_aliases_to_canonical_def_ids() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_canonical_import_aliases_{}_{}",
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
            .expect("collect should resolve canonical import aliases");

    let expected = *decls
        .resolver
        .item_paths
        .get("test::io::writer_name")
        .expect("canonical item path should resolve");
    assert_eq!(
        decls.resolver.import_aliases.get("writer_name"),
        Some(&expected)
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_skips_non_explicit_glob_import_aliases_in_canonical_import_table() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_non_explicit_glob_aliases_{}_{}",
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

    let (program, graph) = load_source_program(root_path, "mod io\n> io::*\n", Some("test"));

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("collect should preserve legacy glob aliases but skip canonical alias entries");

    assert!(!decls.resolver.import_aliases.contains_key("writer_name"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_resolves_explicit_type_import_aliases_to_canonical_def_ids() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_canonical_type_import_aliases_{}_{}",
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

    let (program, graph) = load_source_program(root_path, "mod io\n> io::Writer\n", Some("test"));

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("collect should resolve canonical type import aliases");

    let expected = *decls
        .resolver
        .item_paths
        .get("test::io::Writer")
        .expect("canonical item path should resolve");
    assert!(decls.items.structs().contains_key(&expected));
    assert_eq!(decls.items.structs().len(), 1);
    assert_eq!(decls.resolver.import_aliases.get("Writer"), Some(&expected));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_resolves_export_function_aliases_to_canonical_def_ids() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_canonical_export_aliases_{}_{}",
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
    std::fs::write(&math_path, "mod io\n< io::writer_name\n").unwrap();
    std::fs::write(&io_path, "writer_name: I64\nwriter_name = -> 0\n").unwrap();

    let (program, graph) = load_source_program(root_path, "mod math\n", Some("test"));

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("collect should resolve canonical export aliases");

    assert!(decls
        .resolver
        .item_paths
        .contains_key("test::math::io::writer_name"));
    assert!(!decls
        .resolver
        .item_paths
        .contains_key("test::math::writer_name"));

    let expected = *decls
        .resolver
        .item_paths
        .get("test::math::io::writer_name")
        .expect("canonical item path should resolve");
    assert_eq!(
        decls.resolver.export_aliases.get("test::math::writer_name"),
        Some(&expected)
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_resolves_export_type_aliases_to_canonical_def_ids() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_canonical_export_type_aliases_{}_{}",
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
    std::fs::write(&math_path, "mod io\n< io::Writer\n").unwrap();
    std::fs::write(&io_path, "struct Writer\n").unwrap();

    let (program, graph) = load_source_program(root_path, "mod math\n", Some("test"));

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("collect should resolve canonical export type aliases");

    assert!(struct_named(&decls, "test::math::Writer").is_none());
    assert!(decls
        .resolver
        .item_paths
        .contains_key("test::math::io::Writer"));
    assert!(!decls.resolver.item_paths.contains_key("test::math::Writer"));

    let expected = *decls
        .resolver
        .item_paths
        .get("test::math::io::Writer")
        .expect("canonical item path should resolve");
    assert_eq!(
        decls.resolver.export_aliases.get("test::math::Writer"),
        Some(&expected)
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn lowered_hir_indexes_import_and_export_aliases_by_canonical_def_id() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_canonical_alias_indexes_{}_{}",
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
    std::fs::write(&math_path, "mod io\n< io::writer_name\n< io::Writer\n").unwrap();
    std::fs::write(
        &io_path,
        "writer_name: I64\nwriter_name = -> 0\nstruct Writer\n",
    )
    .unwrap();

    let (program, graph) = load_source_program(
        root_path,
        "mod math\n> math::writer_name\n> math::Writer\n",
        Some("test"),
    );

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("collect should resolve import and export aliases");

    let writer_name_id = *decls
        .resolver
        .item_paths
        .get("test::math::io::writer_name")
        .expect("canonical exported function should resolve");
    let writer_id = *decls
        .resolver
        .item_paths
        .get("test::math::io::Writer")
        .expect("canonical exported struct should resolve");
    assert_eq!(
        decls.resolver.export_aliases.get("test::math::writer_name"),
        Some(&writer_name_id)
    );
    assert_eq!(
        decls.resolver.export_aliases.get("test::math::Writer"),
        Some(&writer_id)
    );
    assert_eq!(
        decls.resolver.import_aliases.get("writer_name"),
        Some(&writer_name_id)
    );
    assert_eq!(
        decls.resolver.import_aliases.get("Writer"),
        Some(&writer_id)
    );

    let partial = crate::lower::program::lower_from_declarations(
        &program,
        decls,
        &CrateContext::new(),
        Some("test"),
    )
    .expect("lowering should preserve canonical alias DefIds");
    let resolved =
        crate::infer::finalize(partial).expect("inference should finalize canonical alias indexes");

    assert_eq!(
        resolved
            .program
            .indexes
            .functions_by_id
            .get(&writer_name_id),
        Some(&"test::math::io::writer_name".to_string())
    );
    assert_eq!(
        resolved.program.indexes.structs_by_id.get(&writer_id),
        Some(&"test::math::io::Writer".to_string())
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_preserves_import_aliases_infix_precedence_and_function_signatures() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
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
                TopLevel::Import(crate::ast::Path::Ident(crate::ast::IdentifierPath {
                    path: vec![crate::ast::IdentOrType::Ident(ident("sum"))],
                })),
                TopLevel::InfixOperator(7, "++".to_string()),
                TopLevel::FunctionSig(crate::ast::FunctionSig {
                    name: ident("declared_only"),
                    sig: crate::ast::ParseType::Unit(crate::lexer::Span::test()),
                    where_clauses: vec![],
                    self_receiver: None,
                    is_unsafe: false,
                    exported: false,
                }),
            ],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("collect should preserve declaration-side bookkeeping");

    let sum_id = function_named(&decls, "sum").unwrap().id;
    assert_eq!(decls.resolver.import_aliases.get("sum"), Some(&sum_id));
    assert_eq!(decls.infix_precedence.get("++"), Some(&7));
    assert!(function_sig_named(&decls, "declared_only").is_some());
}

#[test]
fn lower_from_declarations_resolves_nested_imported_types_inside_inline_module_bodies() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_lower_nested_source_backed_type_context_{}_{}",
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
    std::fs::write(&io_path, "< struct Writer\n").unwrap();
    let (_, graph) = load_source_program(root_path.clone(), "mod io\n", Some("math"));

    let writer_instance = crate::ast::Expression::UnaryExpr(crate::ast::UnaryExpr::PrimaryExpr(
        crate::ast::PrimaryExpr {
            operand: crate::ast::Operand::Instance(crate::ast::Instance {
                name: crate::ast::TypePath {
                    path: vec![crate::ast::IdentOrType::Type(crate::ast::ParseType::Type(
                        type_inner("Writer"),
                    ))],
                },
                fields: HashMap::new(),
            }),
            secondaries: None,
            type_annotation: None,
        },
    ));

    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![inline_module(
                "math",
                vec![
                    TopLevel::Mod(ident("io"), false),
                    TopLevel::Import(crate::ast::Path::Type(crate::ast::TypePath {
                        path: vec![
                            crate::ast::IdentOrType::Ident(ident("io")),
                            crate::ast::IdentOrType::Type(crate::ast::ParseType::Type(type_inner(
                                "Writer",
                            ))),
                        ],
                    })),
                    TopLevel::FunctionDecl(function_decl_with_body_expr(
                        "make_writer",
                        writer_instance,
                    )),
                ],
            )],
            is_inline: false,
            filepath: Some(root_path),
        },
    };

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("collect should preserve nested source-backed module type context");

    let make_writer_id = function_named(&decls, "math::make_writer").unwrap().id;
    let writer_id = struct_named(&decls, "math::io::Writer").unwrap().id;
    let partial = crate::lower::program::lower_from_declarations(
        &program,
        decls,
        &CrateContext::new(),
        Some("test"),
    )
    .expect("lowering should resolve nested imported types module-locally");

    let function = partial
        .functions
        .get(&make_writer_id)
        .expect("inline module body should lower under its qualified name");

    assert_eq!(
        function.ret_type,
        crate::types::Type::Struct {
            id: writer_id,
            args: vec![],
        }
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
