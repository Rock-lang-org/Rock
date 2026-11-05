use super::*;

#[test]
fn collect_records_dependency_import_alias_by_canonical_def_id() {
    let answer_id = DefId::new(CrateId(7), LocalDefId(11));
    let mut answer = hir_function("answer");
    answer.id = answer_id;

    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function("dep::io::answer".to_string(), answer);
    interface.root_export_ids.insert(
        "answer".to_string(),
        ArtifactExport {
            source: "dep::io::answer".to_string(),
            id: answer_id,
        },
    );

    let mut resolver = crate::collect::resolver::ResolverTables::default();
    resolver
        .item_paths
        .insert("dep::io::answer".to_string(), answer_id);
    resolver
        .item_names_by_id
        .insert(answer_id, "dep::io::answer".to_string());

    let mut crate_ctx = CrateContext::new();
    add_artifact_extern_crate(&mut crate_ctx, "dep", interface, resolver, BTreeMap::new());
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![TopLevel::Import(crate::ast::Path::Ident(
                crate::ast::IdentifierPath {
                    path: vec![
                        crate::ast::IdentOrType::Ident(ident("dep")),
                        crate::ast::IdentOrType::Ident(ident("answer")),
                    ],
                },
            ))],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &crate_ctx, false, Some("test"))
        .expect("collect should record dependency import alias IDs");

    assert!(function_named(&decls, "dep::io::answer").is_some());
    assert!(function_named(&decls, "dep::answer").is_none());
    assert_eq!(
        decls.resolver.import_aliases.get("answer"),
        Some(&answer_id)
    );
    assert_eq!(
        decls.resolver.item_names_by_id.get(&answer_id),
        Some(&"dep::io::answer".to_string())
    );
}

#[test]
fn collect_records_canonical_names_for_artifact_payloads_without_dependency_resolver_names() {
    let answer_id = DefId::new(CrateId(7), LocalDefId(11));
    let mut answer = hir_function("answer");
    answer.id = answer_id;

    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function("dep::answer".to_string(), answer);

    let mut crate_ctx = CrateContext::new();
    add_artifact_extern_crate(
        &mut crate_ctx,
        "dep",
        interface,
        crate::collect::resolver::ResolverTables::default(),
        BTreeMap::new(),
    );
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &crate_ctx, false, Some("test"))
        .expect("collection should retain artifact declarations");

    assert_eq!(
        decls.resolver.item_names_by_id.get(&answer_id),
        Some(&"dep::answer".to_string())
    );
    assert!(crate::lower::Lowerer::from_declarations(decls).is_ok());
}

#[test]
fn collect_records_module_local_artifact_root_import_alias_by_canonical_def_id() {
    let drop_id = DefId::new(CrateId(7), LocalDefId(13));

    let mut interface = ArtifactCrateInterface::default();
    interface.insert_trait(
        "dep::drop::Drop".to_string(),
        hir_trait(drop_id, "Drop", &[]),
    );
    interface.root_export_ids.insert(
        "Drop".to_string(),
        ArtifactExport {
            source: "dep::drop::Drop".to_string(),
            id: drop_id,
        },
    );

    let mut resolver = crate::collect::resolver::ResolverTables::default();
    resolver
        .item_paths
        .insert("dep::drop::Drop".to_string(), drop_id);
    resolver
        .item_names_by_id
        .insert(drop_id, "dep::drop::Drop".to_string());

    let mut crate_ctx = CrateContext::new();
    add_artifact_extern_crate(&mut crate_ctx, "dep", interface, resolver, BTreeMap::new());
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![inline_module(
                "inner",
                vec![TopLevel::Import(crate::ast::Path::Ident(
                    crate::ast::IdentifierPath {
                        path: vec![
                            crate::ast::IdentOrType::Ident(ident("dep")),
                            crate::ast::IdentOrType::Ident(ident("Drop")),
                        ],
                    },
                ))],
            )],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &crate_ctx, false, Some("test"))
        .expect("collect should record module-local artifact import alias IDs");

    assert_eq!(decls.resolver.import_aliases.get("Drop"), None);
    assert_eq!(
        decls.resolver.import_aliases.get("inner::Drop"),
        Some(&drop_id)
    );
    assert_eq!(
        decls.resolver.item_names_by_id.get(&drop_id),
        Some(&"dep::drop::Drop".to_string())
    );
}

#[test]
fn collect_records_artifact_glob_import_alias_by_export_def_id() {
    let answer_id = DefId::new(CrateId(7), LocalDefId(12));
    let mut answer = hir_function("answer");
    answer.id = answer_id;

    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function("dep::io::answer".to_string(), answer);
    interface.root_export_ids.insert(
        "answer".to_string(),
        ArtifactExport {
            source: "dep::io::answer".to_string(),
            id: answer_id,
        },
    );

    let mut resolver = crate::collect::resolver::ResolverTables::default();
    resolver
        .item_paths
        .insert("dep::io::answer".to_string(), answer_id);
    resolver
        .item_names_by_id
        .insert(answer_id, "dep::io::answer".to_string());

    let mut crate_ctx = CrateContext::new();
    add_artifact_extern_crate(&mut crate_ctx, "dep", interface, resolver, BTreeMap::new());
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![TopLevel::GlobImport(vec!["dep".to_string()])],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &crate_ctx, false, None)
        .expect("collect should record artifact glob import alias IDs");

    assert_eq!(
        decls.resolver.import_aliases.get("answer"),
        Some(&answer_id)
    );
    assert_eq!(
        decls.resolver.item_names_by_id.get(&answer_id),
        Some(&"dep::io::answer".to_string())
    );
}

#[test]
fn collect_context_rejects_source_backed_dependency_crate() {
    let mut crate_ctx = CrateContext::new();
    crate_ctx.register_crate(
        crate::crate_system::CrateManifest {
            crate_: crate::crate_system::CrateConfig {
                name: "dep".to_string(),
                version: "0.1.0".to_string(),
                no_std: false,
            },
            targets: vec![crate::crate_system::CrateTarget::Lib {
                path: "lib.rk".to_string(),
            }],
            dependencies: None,
        },
        PathBuf::from("/dep"),
        Module {
            name: None,
            top_levels: vec![TopLevel::StructDecl(StructDecl {
                name: type_inner("DepThing"),
                generic_params: vec![],
                fields: vec![],
                exported: false,
            })],
            is_inline: false,
            filepath: None,
        },
    );

    let mut context = context::CollectContext::bootstrap_for_collection(false, Some("test"), None);
    context.register_crate_functions(&crate_ctx);

    assert!(crate_ctx.source_crate("dep").is_some());
    assert!(!context.structs.contains_key("dep::DepThing"));
    assert!(!context
        .loaded_module_paths
        .iter()
        .any(|(name, _)| name == "dep"));
    assert!(context.errors.is_empty());
}

#[test]
fn collect_context_records_canonical_aliases_for_dependency_crates() {
    let mut context = context::CollectContext::bootstrap_for_collection(false, Some("test"), None);
    let writer_id = DefId::new(CrateId(0), LocalDefId(0));
    context.structs.insert(
        "dep::io::Writer".to_string(),
        HirStruct {
            id: writer_id,
            name: "Writer".to_string(),
            generic_params: vec![],
            fields: vec![],
        },
    );

    let path = crate::ast::Path::Ident(crate::ast::IdentifierPath {
        path: vec![
            crate::ast::IdentOrType::Ident(ident("dep")),
            crate::ast::IdentOrType::Ident(ident("io")),
            crate::ast::IdentOrType::Ident(ident("Writer")),
        ],
    });
    context.handle_import(&path, false, None);

    let collection = context.into_local_collection(&ItemIndex::new(), ModuleId(0), Some("test"));
    assert!(collection.structs.contains_key("dep::io::Writer"));
    assert!(!collection.structs.contains_key("Writer"));
    assert_eq!(
        collection.resolver.import_aliases.get("Writer").copied(),
        Some(writer_id)
    );
    assert_eq!(
        collection.resolver.canonical_name(writer_id),
        Some("dep::io::Writer")
    );
}

#[test]
fn collect_context_bootstraps_interface_std_prelude() {
    let print_id = DefId::new(CrateId(0), LocalDefId(0));
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function(
        "std::print".to_string(),
        HirFunction {
            id: print_id,
            name: "print".to_string(),
            generic_params: vec![],
            generic_bounds: HashMap::new().into(),
            params: vec![],
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: vec![],
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        },
    );

    let mut crate_ctx = CrateContext::new();
    crate_ctx
        .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
            CrateId(1),
            "std".to_string(),
            crate::crate_system::ExternCrateMetadata::new(
                interface,
                crate::collect::resolver::ResolverTables::default(),
                BTreeMap::from([(
                    "print".to_string(),
                    ArtifactExport {
                        source: "std::print".to_string(),
                        id: print_id,
                    },
                )]),
            ),
            crate::crate_system::ExternCrateBodies::default(),
            crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .unwrap();

    let mut context = context::CollectContext::bootstrap_for_collection(true, Some("test"), None);
    context.register_crate_functions(&crate_ctx);
    context.inject_loaded_prelude(&crate_ctx);

    assert_eq!(context.names.import_alias("print"), Some("std::print"));
    assert!(context.functions.contains_key("std::print"));
    assert!(!context.functions.contains_key("print"));
}

#[test]
fn collect_ignores_source_backed_external_dependency_consumption() {
    let mut crate_ctx = CrateContext::new();
    crate_ctx.register_crate(
        crate::crate_system::CrateManifest {
            crate_: crate::crate_system::CrateConfig {
                name: "dep".to_string(),
                version: "0.1.0".to_string(),
                no_std: false,
            },
            targets: vec![crate::crate_system::CrateTarget::Lib {
                path: "lib.rk".to_string(),
            }],
            dependencies: None,
        },
        std::path::PathBuf::from("/dep"),
        Module {
            name: None,
            top_levels: vec![TopLevel::StructDecl(StructDecl {
                name: type_inner("DepThing"),
                generic_params: vec![],
                fields: vec![],
                exported: false,
            })],
            is_inline: false,
            filepath: None,
        },
    );

    let program = program_with_struct("Point");
    let decls = collect(&program, &crate_ctx, false, Some("test"))
        .expect("source-backed external dependency should not be registered as an extern");

    assert!(crate_ctx.source_crate("dep").is_some());
    assert!(struct_named(&decls, "dep::DepThing").is_none());
}

#[test]
fn collect_accepts_artifact_dependency_without_registering_source_module_path() {
    let mut context = context::CollectContext::new();
    let mut crate_ctx = CrateContext::new();
    crate_ctx
        .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
            CrateId(1),
            "dep".to_string(),
            crate::crate_system::ExternCrateMetadata::new(
                ArtifactCrateInterface::default(),
                crate::collect::resolver::ResolverTables::default(),
                BTreeMap::new(),
            ),
            crate::crate_system::ExternCrateBodies::default(),
            crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .unwrap();

    context.register_extern_crate(crate_ctx.extern_crate("dep").unwrap());

    assert!(context.errors.is_empty());
    assert!(context.loaded_module_paths.is_empty());
}

#[test]
fn collect_registers_artifact_dependency_from_extern_store() {
    let mut crate_ctx = CrateContext::new();
    let id = DefId::new(CrateId(7), LocalDefId(1));
    let mut interface = crate::crate_artifact::ArtifactCrateInterface::default();
    let mut answer = hir_function("answer");
    answer.id = id;
    interface.insert_function("dep::answer".to_string(), answer);
    interface.root_export_ids.insert(
        "answer".to_string(),
        ArtifactExport {
            source: "dep::answer".to_string(),
            id,
        },
    );
    let mut resolver = crate::collect::resolver::ResolverTables::default();
    resolver.item_paths.insert("dep::answer".to_string(), id);
    resolver
        .item_names_by_id
        .insert(id, "dep::answer".to_string());

    crate_ctx
        .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
            CrateId(7),
            "dep".to_string(),
            crate::crate_system::ExternCrateMetadata::new(interface, resolver, BTreeMap::new()),
            crate::crate_system::ExternCrateBodies::default(),
            crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .unwrap();

    let mut context = context::CollectContext::bootstrap_for_collection(false, Some("test"), None);
    context.register_crate_functions(&crate_ctx);

    assert!(context.functions.contains_key("dep::answer"));
    assert_eq!(
        context
            .dependency_root_export_ids
            .get("dep")
            .and_then(|exports| exports.get("answer"))
            .map(|export| export.source.as_str()),
        Some("dep::answer")
    );
}

#[test]
fn collect_registers_artifact_dependency_through_provider_capabilities() {
    let mut crate_ctx = CrateContext::new();
    let mut interface = crate::crate_artifact::ArtifactCrateInterface::default();
    let id = DefId::new(CrateId(7), LocalDefId(1));
    let mut answer = hir_function("answer");
    answer.id = id;
    interface.insert_function("dep::answer".to_string(), answer);
    interface.root_export_ids.insert(
        "answer".to_string(),
        ArtifactExport {
            source: "dep::answer".to_string(),
            id,
        },
    );

    add_artifact_extern_crate(
        &mut crate_ctx,
        "dep",
        interface,
        crate::collect::resolver::ResolverTables::default(),
        BTreeMap::new(),
    );

    let mut context = context::CollectContext::bootstrap_for_collection(false, Some("test"), None);
    context.register_crate_functions(&crate_ctx);

    assert!(context.functions.contains_key("dep::answer"));
    assert_eq!(
        context
            .dependency_root_export_ids
            .get("dep")
            .and_then(|exports| exports.get("answer"))
            .map(|export| export.source.as_str()),
        Some("dep::answer")
    );
}

#[test]
fn lower_from_declarations_rejects_source_backed_external_dependency_consumption() {
    let program = program_with_struct("Point");
    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("collection without dependencies should succeed");

    let mut crate_ctx = CrateContext::new();
    crate_ctx.register_crate(
        crate::crate_system::CrateManifest {
            crate_: crate::crate_system::CrateConfig {
                name: "dep".to_string(),
                version: "0.1.0".to_string(),
                no_std: false,
            },
            targets: vec![crate::crate_system::CrateTarget::Lib {
                path: "lib.rk".to_string(),
            }],
            dependencies: None,
        },
        PathBuf::from("/dep"),
        Module {
            name: None,
            top_levels: vec![TopLevel::FunctionDecl(function_decl("identity"))],
            is_inline: false,
            filepath: None,
        },
    );

    let errors = match crate::lower::program::lower_from_declarations(
        &program,
        decls,
        &crate_ctx,
        Some("test"),
    ) {
        Ok(_) => panic!("source-backed external dependency lowering should fail"),
        Err(errors) => errors,
    };

    let source_backed_rejections = errors
        .iter()
        .filter(|error| {
            error.message.contains(
                "source-backed external dependency 'dep' is not supported during lowering",
            )
        })
        .count();
    assert_eq!(source_backed_rejections, 1);
}
