use super::*;

#[test]
fn collect_artifact_declarations_skips_current_crate_during_dependency_bootstrap() {
    let io_path = PathBuf::from("/dep/io.rk");
    let dep_module = Module {
        name: None,
        top_levels: vec![
            TopLevel::StructDecl(StructDecl {
                name: type_inner("Point"),
                generic_params: vec![],
                fields: vec![],
                exported: false,
            }),
            TopLevel::Impl(inherent_impl("Point", "origin")),
            TopLevel::Mod(ident("io"), true),
        ],
        is_inline: false,
        filepath: Some(PathBuf::from("/dep/lib.rk")),
    };
    let dep_io_module = Module {
        name: Some(ident("io")),
        top_levels: vec![TopLevel::StructDecl(StructDecl {
            name: type_inner("Writer"),
            generic_params: vec![],
            fields: vec![],
            exported: false,
        })],
        is_inline: false,
        filepath: Some(io_path.clone()),
    };

    let mut support_interface = ArtifactCrateInterface::default();
    support_interface.insert_struct(
        "support::Helper".to_string(),
        crate::hir::HirStruct {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            name: "Helper".to_string(),
            generic_params: vec![],
            fields: vec![],
        },
    );

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
        dep_module.clone(),
    );
    let source_crate = crate_ctx.source_crate_mut("dep").unwrap();
    source_crate
        .loaded_module_paths
        .push(("dep".to_string(), PathBuf::from("/dep/lib.rk")));
    source_crate
        .loaded_module_paths
        .push(("dep::io".to_string(), io_path.clone()));
    source_crate
        .file_cache
        .insert(io_path.clone(), dep_io_module);
    add_artifact_extern_crate(
        &mut crate_ctx,
        "support",
        support_interface,
        crate::collect::resolver::ResolverTables::default(),
        BTreeMap::new(),
    );

    let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
        .expect("artifact declaration collection should succeed");
    let decls = decls.declarations;

    let point_id = decls.item_index.defs_named("Point")[0];
    assert!(decls.items.structs().contains_key(&point_id));
    assert_eq!(decls.items.structs().len(), 2);

    let point_impls: Vec<_> = decls
        .items
        .impls()
        .values()
        .filter(|imp| imp.type_name == "Point" && imp.trait_name.is_none())
        .collect();
    assert_eq!(point_impls.len(), 1);
    assert_eq!(point_impls[0].methods.len(), 1);
    assert!(point_impls[0].methods.contains_key("origin"));
    assert!(decls.module_file_cache.contains_key(&io_path));
    assert!(struct_named(&decls, "dep::io::Writer").is_some());

    let point = decls.item_index.get(point_id).unwrap();
    assert_eq!(point.module_id, decls.indexing_ids.root_module_id());
    assert_eq!(point.def_id.crate_id, decls.indexing_ids.root_crate_id());

    let writer_id = decls.item_index.defs_named("Writer")[0];
    let writer = decls.item_index.get(writer_id).unwrap();
    assert_ne!(writer.module_id, decls.indexing_ids.root_module_id());
}

#[test]
fn collect_artifact_declarations_preserves_indexed_inline_and_impl_ids() {
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
            top_levels: vec![
                TopLevel::InfixOperator(7, crate::ast::Operator { value: "++".to_string(), span: crate::lexer::Span::new("test.rk".into(), 0, 0) }),
                TopLevel::StructDecl(StructDecl {
                    name: type_inner("Same"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                }),
                inline_module(
                    "inline",
                    vec![TopLevel::StructDecl(StructDecl {
                        name: type_inner("Same"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: true,
                    })],
                ),
                TopLevel::GlobExport(vec!["inline".to_string()]),
                TopLevel::Impl(inherent_impl("Same", "first")),
                TopLevel::Impl(inherent_impl("Same", "second")),
            ],
            is_inline: false,
            filepath: Some(PathBuf::from("/dep/lib.rk")),
        },
    );

    let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
        .expect("artifact declaration collection should preserve source identities")
        .declarations;
    let root_module_id = decls.indexing_ids.root_module_id();
    let inline_module_id = decls
        .item_index
        .module_id_by_path(&["inline".to_string()])
        .unwrap();

    assert!(decls.items.structs().contains_key(
        &decls
            .item_index
            .item_at_source(root_module_id, 1)
            .unwrap()
            .def_id
    ));
    assert!(decls.items.structs().contains_key(
        &decls
            .item_index
            .item_at_source(inline_module_id, 0)
            .unwrap()
            .def_id
    ));
    assert_eq!(
        decls
            .items
            .impls()
            .values()
            .map(|impl_| impl_.id)
            .collect::<BTreeSet<_>>(),
        [
            decls
                .item_index
                .item_at_source(root_module_id, 4)
                .unwrap()
                .def_id,
            decls
                .item_index
                .item_at_source(root_module_id, 5)
                .unwrap()
                .def_id,
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn collect_artifact_declarations_builds_generic_nominal_and_owner_ids_from_item_records() {
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
        generic_declaration_program().module,
    );

    let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
        .expect("artifact collection should use indexed IDs during construction")
        .declarations;

    assert_generic_declaration_ids(&decls);
}

#[test]
fn collect_artifact_declarations_preserves_function_signatures_with_indexed_ids() {
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
            top_levels: vec![TopLevel::FunctionSig(function_sig(
                "declared_only",
                ParseType::Function(vec![ParseType::Type(type_inner("I64"))]),
            ))],
            is_inline: false,
            filepath: Some(PathBuf::from("/dep/lib.rk")),
        },
    );

    let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
        .expect("artifact declaration collection should preserve signatures")
        .declarations;
    let signature = function_sig_named(&decls, "declared_only")
        .expect("qualified signature should be collected");

    assert_eq!(
        signature.id,
        decls.item_index.defs_named("declared_only")[0]
    );
    assert!(decls.current_def_ids.contains(&signature.id));
    assert_no_invalid_current_crate_ids(&decls);
}

#[test]
fn collect_artifact_declarations_expands_inline_module_glob_exports_with_qualified_prefix() {
    let inner_path = PathBuf::from("/dep/outer/inner.rk");
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
            top_levels: vec![
                inline_module(
                    "outer",
                    vec![
                        TopLevel::Mod(ident("inner"), false),
                        TopLevel::GlobExport(vec!["inner".to_string()]),
                    ],
                ),
                TopLevel::GlobExport(vec!["outer".to_string()]),
            ],
            is_inline: false,
            filepath: Some(PathBuf::from("/dep/lib.rk")),
        },
    );
    let source_crate = crate_ctx.source_crate_mut("dep").unwrap();
    source_crate
        .loaded_module_paths
        .push(("dep::outer::inner".to_string(), inner_path.clone()));
    source_crate.file_cache.insert(
        inner_path.clone(),
        Module {
            name: Some(ident("inner")),
            top_levels: vec![TopLevel::StructDecl(StructDecl {
                name: type_inner("Thing"),
                generic_params: vec![],
                fields: vec![],
                exported: true,
            })],
            is_inline: false,
            filepath: Some(inner_path),
        },
    );

    let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
        .expect("artifact declaration collection should expand nested glob exports")
        .declarations;
    let thing_id = decls.item_index.defs_named("Thing")[0];

    assert!(decls.items.structs().contains_key(&thing_id));
    assert_eq!(
        decls.resolver.item_paths.get("dep::outer::inner::Thing"),
        Some(&thing_id)
    );
    assert_eq!(
        decls.resolver.export_aliases.get("dep::outer::Thing"),
        Some(&thing_id)
    );
}

#[test]
fn collect_artifact_declarations_uses_source_crate_loaded_module_paths() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_artifact_source_paths_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(temp_dir.join("src").join("util")).unwrap();
    std::fs::write(
        temp_dir.join("rock.toml"),
        "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
    )
    .unwrap();
    std::fs::write(temp_dir.join("src").join("lib.rk"), "mod util\n< util::*\n").unwrap();
    std::fs::write(
        temp_dir.join("src").join("util").join("mod.rk"),
        "< mod sub\n< sub::*\n",
    )
    .unwrap();
    std::fs::write(
        temp_dir.join("src").join("util").join("sub.rk"),
        "< struct Writer\n",
    )
    .unwrap();

    let mut crate_ctx = CrateContext::new();
    crate_ctx
        .load_crate_from_dir(temp_dir.clone())
        .expect("source crate should load");

    let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
        .expect("artifact declaration collection should use source graph module paths")
        .declarations;

    assert!(struct_named(&decls, "dep::util::sub::Writer").is_some());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_artifact_declarations_preserves_child_exports_when_parent_reference_key_collides() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_artifact_export_collision_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(temp_dir.join("src").join("b")).unwrap();
    std::fs::write(
        temp_dir.join("rock.toml"),
        "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
    )
    .unwrap();
    std::fs::write(
        temp_dir.join("src").join("lib.rk"),
        "mod b\n< b::a::Thing\n",
    )
    .unwrap();
    std::fs::write(
        temp_dir.join("src").join("b").join("mod.rk"),
        "mod a\nmod c\n< c::*\n",
    )
    .unwrap();
    std::fs::write(
        temp_dir.join("src").join("b").join("a.rk"),
        "< struct Thing\n",
    )
    .unwrap();
    std::fs::write(
        temp_dir.join("src").join("b").join("c.rk"),
        "< struct Thing\n",
    )
    .unwrap();

    let mut crate_ctx = CrateContext::new();
    crate_ctx
        .load_crate_from_dir(temp_dir.clone())
        .expect("source crate should load");

    let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
        .expect("artifact declaration collection should succeed")
        .declarations;

    assert!(struct_named(&decls, "dep::b::a::Thing").is_some());
    assert!(struct_named(&decls, "dep::b::c::Thing").is_some());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_artifact_declarations_capture_dependency_bootstrap_for_cross_crate_hir() {
    let support_identity_id = DefId::new(CrateId(7), LocalDefId(42));
    let dep_module = Module {
        name: None,
        top_levels: vec![
            TopLevel::FunctionDecl(FunctionDecl {
                name: ident("identity"),
                lambda: LambdaDecl {
                    parameters: vec![],
                    body: crate::ast::Block { statements: vec![] },
                    arrow_kind: LambdaArrowKind::Normal,
                    span: crate::lexer::Span::test(),
                },
                self_receiver: None,
                is_unsafe: false,
                exported: true,
            }),
            TopLevel::StructDecl(StructDecl {
                name: type_inner("Box"),
                generic_params: vec![],
                fields: vec![],
                exported: false,
            }),
            TopLevel::TraitDecl(crate::ast::TraitDecl {
                where_clauses: Vec::new(),
                name: type_inner("Show"),
                generic_params: vec![],
                for_: None,
                associated_types: vec![],
                methods: HashMap::from([(ident("show"), function_decl("show"))]),
                signatures: HashMap::new(),
                exported: false,
                language_items: Default::default(),
            }),
            TopLevel::Impl(trait_impl("Box", "Show", "show")),
        ],
        is_inline: false,
        filepath: Some(PathBuf::from("/dep/lib.rk")),
    };

    let mut support_interface = ArtifactCrateInterface::default();
    support_interface.insert_function(
        "support::identity".to_string(),
        HirFunction {
            id: support_identity_id,
            name: "identity".to_string(),
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
    support_interface.insert_struct(
        "support::Helper".to_string(),
        crate::hir::HirStruct {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            name: "Helper".to_string(),
            generic_params: vec![],
            fields: vec![],
        },
    );

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
        dep_module,
    );
    add_artifact_extern_crate(
        &mut crate_ctx,
        "support",
        support_interface,
        crate::collect::resolver::ResolverTables::default(),
        BTreeMap::new(),
    );

    let artifact = collect_artifact_hir_declarations(&crate_ctx, false, "dep")
        .expect("artifact HIR declaration collection should succeed");

    assert!(artifact
        .declarations
        .items
        .functions()
        .contains_key(&support_identity_id));
    assert!(artifact
        .declarations
        .items
        .structs()
        .values()
        .any(|structure| structure.name == "Helper"));
    assert!(artifact
        .cross_crate_hir_bootstrap
        .function_keys
        .contains("support::identity"));
    assert!(artifact
        .cross_crate_hir_bootstrap
        .struct_keys
        .contains("support::Helper"));

    let decls = &artifact.declarations;
    let trait_id = decls.item_index.defs_named("Show")[0];
    let trait_def = decls.items.traits().get(&trait_id).unwrap();
    let impl_ = decls
        .items
        .impls()
        .values()
        .find(|imp| imp.type_name == "Box" && imp.trait_name.as_deref() == Some("Show"))
        .expect("current crate artifact impl should be collected");
    let impl_id = impl_.id;
    let trait_method_id = trait_def.methods["show"].id;
    let impl_method_id = impl_.methods["show"].id;

    assert_ne!(trait_method_id, trait_id);
    assert_ne!(impl_method_id, impl_id);
    assert_ne!(trait_method_id, impl_method_id);
    assert_eq!(impl_.methods["show"].id, impl_method_id);
    assert!(decls.current_def_ids.contains(&trait_method_id));
    assert!(decls.current_def_ids.contains(&impl_method_id));
}
