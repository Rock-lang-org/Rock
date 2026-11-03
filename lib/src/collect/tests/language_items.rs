use super::*;

#[test]
fn resolver_contract_keeps_same_name_module_items_qualified_by_id() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
                inline_module(
                    "left",
                    vec![TopLevel::StructDecl(StructDecl {
                        name: type_inner("Thing"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: true,
                    })],
                ),
                inline_module(
                    "right",
                    vec![TopLevel::StructDecl(StructDecl {
                        name: type_inner("Thing"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: true,
                    })],
                ),
            ],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("same-name module items should collect");
    let left_id = struct_named(&decls, "left::Thing").unwrap().id;
    let right_id = struct_named(&decls, "right::Thing").unwrap().id;

    assert_ne!(left_id, right_id);
    assert_eq!(decls.resolver.item_paths.get("left::Thing"), Some(&left_id));
    assert_eq!(
        decls.resolver.item_paths.get("test::left::Thing"),
        Some(&left_id)
    );
    assert_eq!(
        decls.resolver.item_paths.get("right::Thing"),
        Some(&right_id)
    );
    assert_eq!(decls.resolver.item_paths.get("Thing"), None);
}

#[test]
fn collect_artifact_declarations_does_not_collect_unreferenced_same_named_sibling_module() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_artifact_same_named_sibling_{}",
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
        "mod a\nmod b\n< b::a::*\n",
    )
    .unwrap();
    std::fs::write(temp_dir.join("src").join("a.rk"), "< struct Wrong\n").unwrap();
    std::fs::write(temp_dir.join("src").join("b").join("mod.rk"), "mod a\n").unwrap();
    std::fs::write(
        temp_dir.join("src").join("b").join("a.rk"),
        "< struct Right\n",
    )
    .unwrap();

    let mut crate_ctx = CrateContext::new();
    crate_ctx
        .load_crate_from_dir(temp_dir.clone())
        .expect("source crate should load");

    let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
        .expect("artifact declaration collection should succeed")
        .declarations;

    assert!(struct_named(&decls, "dep::b::a::Right").is_some());
    assert!(struct_named(&decls, "dep::a::Wrong").is_none());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

fn collect_language_item_source(source: &str) -> Result<Declarations, Vec<ResolveError>> {
    collect_language_item_program(&parse_language_item_source(source))
}

fn language_item_provider_context(
    providers: impl IntoIterator<Item = (&'static str, LanguageItems<DefId>)>,
) -> CrateContext {
    let mut crate_ctx = CrateContext::new();
    for (index, (name, language_items)) in providers.into_iter().enumerate() {
        crate_ctx
            .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
                CrateId((index + 1) as u32),
                name.to_string(),
                crate::crate_system::ExternCrateMetadata::empty_for_test()
                    .with_language_items(language_items),
                crate::crate_system::ExternCrateBodies::default(),
                crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .expect("test language item provider should register");
    }
    crate_ctx
}

#[test]
fn collect_binds_renamed_sized_trait_to_its_indexed_id() {
    let decls = collect_language_item_source("lang sized\n< trait StaticLayout\n")
        .expect("renamed sized root should collect");
    let indexed_id = decls
        .item_index
        .item_at_source(decls.indexing_ids.root_module_id(), 0)
        .expect("sized trait should be indexed")
        .def_id;

    assert_eq!(decls.language_items.sized.unwrap().trait_id, indexed_id);
}

#[test]
fn collect_binds_renamed_index_mut_bundle_by_id() {
    let program = parse_language_item_source(
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue
"#,
    );
    let decls = collect(&program, &CrateContext::new(), false, Some("core"))
        .expect("renamed Index and IndexMut roots should collect");
    let index_mut = decls
        .language_items
        .index_mut
        .expect("IndexMut bundle should be bound");

    assert_eq!(
        decls.resolver.item_paths.get("core::WriteAt"),
        Some(&index_mut.trait_id),
    );
    assert_ne!(index_mut.method_id, index_mut.trait_id);
}

#[test]
fn collect_language_items_merges_std_metadata_without_prelude_injection() {
    let sized_id = DefId::new(CrateId(7), LocalDefId(11));
    let crate_ctx = language_item_provider_context([(
        "std",
        LanguageItems {
            sized: Some(SizedLanguageItems { trait_id: sized_id }),
            ..LanguageItems::default()
        },
    )]);

    let decls = collect(
        &parse_language_item_source("struct App\n"),
        &crate_ctx,
        false,
        Some("app"),
    )
    .expect("explicit std metadata should merge without prelude injection");

    assert_eq!(
        decls.language_items.sized.map(|items| items.trait_id),
        Some(sized_id),
    );
}

#[test]
fn collect_artifact_declarations_merges_explicit_dependency_language_items() {
    let sized_id = DefId::new(CrateId(7), LocalDefId(11));
    let mut crate_ctx = CrateContext::new();
    crate_ctx.register_crate(
        crate::crate_system::CrateManifest {
            crate_: crate::crate_system::CrateConfig {
                name: "app".to_string(),
                version: "0.1.0".to_string(),
                no_std: false,
            },
            targets: vec![crate::crate_system::CrateTarget::Lib {
                path: "lib.rk".to_string(),
            }],
            dependencies: None,
        },
        PathBuf::from("/app"),
        parse_language_item_source("struct App\n").module,
    );
    crate_ctx
        .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
            CrateId(7),
            "tools".to_string(),
            crate::crate_system::ExternCrateMetadata::empty_for_test().with_language_items(
                LanguageItems {
                    sized: Some(SizedLanguageItems { trait_id: sized_id }),
                    ..LanguageItems::default()
                },
            ),
            crate::crate_system::ExternCrateBodies::default(),
            crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .expect("test language item provider should register");

    let decls = collect_artifact_declarations(&crate_ctx, false, "app")
        .expect("artifact collection should merge explicit metadata")
        .declarations;

    assert_eq!(
        decls.language_items.sized.map(|items| items.trait_id),
        Some(sized_id),
    );
}

fn artifact_context_with_same_name_language_item_record(
    source: &str,
    language_items: LanguageItems<DefId>,
) -> CrateContext {
    let mut crate_ctx = CrateContext::new();
    crate_ctx.register_crate(
        crate::crate_system::CrateManifest {
            crate_: crate::crate_system::CrateConfig {
                name: "app".to_string(),
                version: "0.1.0".to_string(),
                no_std: false,
            },
            targets: vec![crate::crate_system::CrateTarget::Lib {
                path: "lib.rk".to_string(),
            }],
            dependencies: None,
        },
        PathBuf::from("/app"),
        parse_language_item_source(source).module,
    );
    crate_ctx
        .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
            CrateId(7),
            "app".to_string(),
            crate::crate_system::ExternCrateMetadata::empty_for_test()
                .with_language_items(language_items),
            crate::crate_system::ExternCrateBodies::default(),
            crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .expect("test self artifact record should register");
    crate_ctx
}

#[test]
fn collect_artifact_declarations_excludes_same_name_artifact_language_items() {
    let crate_ctx = artifact_context_with_same_name_language_item_record(
        "lang sized\n< trait SourceSized\n",
        LanguageItems {
            sized: Some(SizedLanguageItems {
                trait_id: DefId::new(CrateId(7), LocalDefId(11)),
            }),
            ..LanguageItems::default()
        },
    );

    let decls = collect_artifact_declarations(&crate_ctx, false, "app")
        .expect("current source provider should not conflict with its artifact record")
        .declarations;
    let current_sized_id = decls.item_index.defs_named("SourceSized")[0];

    assert_eq!(
        decls.language_items.sized.map(|items| items.trait_id),
        Some(current_sized_id),
    );
}

#[test]
fn collect_artifact_declarations_does_not_use_same_name_artifact_language_items() {
    let crate_ctx = artifact_context_with_same_name_language_item_record(
        "struct App\n",
        LanguageItems {
            sized: Some(SizedLanguageItems {
                trait_id: DefId::new(CrateId(7), LocalDefId(11)),
            }),
            ..LanguageItems::default()
        },
    );

    let decls = collect_artifact_declarations(&crate_ctx, false, "app")
        .expect("same-name artifact metadata should not become a dependency provider")
        .declarations;

    assert_eq!(decls.language_items.sized, None);
}

#[test]
fn collect_excludes_same_name_extern_from_declarations_and_language_items() {
    let artifact_function_id = DefId::new(CrateId(7), LocalDefId(10));
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function(
        "app::from_self_artifact".to_string(),
        test_function(artifact_function_id, "from_self_artifact"),
    );
    let program = parse_language_item_source("lang sized\n< trait SourceSized\n");
    let mut crate_ctx = CrateContext::new();
    crate_ctx.register_crate(
        crate::crate_system::CrateManifest {
            crate_: crate::crate_system::CrateConfig {
                name: "app".to_string(),
                version: "0.1.0".to_string(),
                no_std: false,
            },
            targets: vec![crate::crate_system::CrateTarget::Lib {
                path: "lib.rk".to_string(),
            }],
            dependencies: None,
        },
        PathBuf::from("/app"),
        program.module.clone(),
    );
    crate_ctx
        .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
            CrateId(7),
            "app".to_string(),
            crate::crate_system::ExternCrateMetadata::new(
                interface,
                crate::collect::resolver::ResolverTables::default(),
                BTreeMap::new(),
            )
            .with_language_items(LanguageItems {
                sized: Some(SizedLanguageItems {
                    trait_id: DefId::new(CrateId(7), LocalDefId(11)),
                }),
                ..LanguageItems::default()
            }),
            crate::crate_system::ExternCrateBodies::default(),
            crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .expect("test self artifact record should register");

    let decls = collect(&program, &crate_ctx, false, Some("app"))
        .expect("same-name extern should not participate in normal collection");
    let current_sized_id = decls.item_index.defs_named("SourceSized")[0];

    assert!(!decls.items.functions().contains_key(&artifact_function_id));
    assert!(!decls
        .resolver
        .item_names_by_id
        .contains_key(&artifact_function_id));
    assert_eq!(
        decls.language_items.sized.map(|items| items.trait_id),
        Some(current_sized_id),
    );
}

#[test]
fn collect_language_items_keeps_same_name_extern_without_registered_source_crate() {
    let sized_id = DefId::new(CrateId(7), LocalDefId(11));
    let crate_ctx = language_item_provider_context([(
        "app",
        LanguageItems {
            sized: Some(SizedLanguageItems { trait_id: sized_id }),
            ..LanguageItems::default()
        },
    )]);

    let decls = collect(
        &parse_language_item_source("struct App\n"),
        &crate_ctx,
        false,
        Some("app"),
    )
    .expect("same-name explicit extern should remain a dependency without a source crate");

    assert_eq!(
        decls.language_items.sized.map(|items| items.trait_id),
        Some(sized_id),
    );
}

#[test]
fn collect_language_items_conflicts_with_same_name_extern_without_registered_source_crate() {
    let crate_ctx = language_item_provider_context([(
        "app",
        LanguageItems {
            sized: Some(SizedLanguageItems {
                trait_id: DefId::new(CrateId(7), LocalDefId(11)),
            }),
            ..LanguageItems::default()
        },
    )]);

    let Err(errors) = collect(
        &parse_language_item_source("lang sized\n< trait SourceSized\n"),
        &crate_ctx,
        false,
        Some("app"),
    ) else {
        panic!("same-name explicit extern should conflict with a current provider");
    };

    assert_eq!(
        errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>(),
        vec!["multiple language-item providers claim protocol 'sized': app, app"],
    );
}

#[test]
fn collect_language_items_rejects_current_and_dependency_sized_providers() {
    let crate_ctx = language_item_provider_context([(
        "dep",
        LanguageItems {
            sized: Some(SizedLanguageItems {
                trait_id: DefId::new(CrateId(7), LocalDefId(11)),
            }),
            ..LanguageItems::default()
        },
    )]);

    let Err(errors) = collect(
        &parse_language_item_source("lang sized\n< trait Current\n"),
        &crate_ctx,
        false,
        Some("app"),
    ) else {
        panic!("current and dependency sized providers should conflict");
    };

    assert_eq!(
        errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>(),
        vec!["multiple language-item providers claim protocol 'sized': app, dep"],
    );
}

#[test]
fn collect_language_items_rejects_dependency_sized_providers_in_lexical_order() {
    let crate_ctx = language_item_provider_context([
        (
            "zeta",
            LanguageItems {
                sized: Some(SizedLanguageItems {
                    trait_id: DefId::new(CrateId(7), LocalDefId(11)),
                }),
                ..LanguageItems::default()
            },
        ),
        (
            "alpha",
            LanguageItems {
                sized: Some(SizedLanguageItems {
                    trait_id: DefId::new(CrateId(8), LocalDefId(12)),
                }),
                ..LanguageItems::default()
            },
        ),
    ]);

    let Err(errors) = collect(
        &parse_language_item_source("struct App\n"),
        &crate_ctx,
        false,
        Some("app"),
    ) else {
        panic!("duplicate dependency sized providers should conflict");
    };

    assert_eq!(
        errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>(),
        vec!["multiple language-item providers claim protocol 'sized': alpha, zeta"],
    );
}

#[test]
fn collect_language_items_merges_disjoint_dependency_bundles() {
    let sized_id = DefId::new(CrateId(7), LocalDefId(11));
    let drop_trait_id = DefId::new(CrateId(8), LocalDefId(12));
    let drop_method_id = DefId::new(CrateId(8), LocalDefId(13));
    let crate_ctx = language_item_provider_context([
        (
            "sized_provider",
            LanguageItems {
                sized: Some(SizedLanguageItems { trait_id: sized_id }),
                ..LanguageItems::default()
            },
        ),
        (
            "drop_provider",
            LanguageItems {
                drop: Some(DropLanguageItems {
                    trait_id: drop_trait_id,
                    method_id: drop_method_id,
                }),
                ..LanguageItems::default()
            },
        ),
    ]);

    let decls = collect(
        &parse_language_item_source("struct App\n"),
        &crate_ctx,
        false,
        Some("app"),
    )
    .expect("disjoint dependency bundles should merge");

    assert_eq!(
        decls.language_items.sized.map(|items| items.trait_id),
        Some(sized_id),
    );
    assert_eq!(
        decls.language_items.drop,
        Some(DropLanguageItems {
            trait_id: drop_trait_id,
            method_id: drop_method_id,
        }),
    );
}

#[test]
fn collect_language_items_reports_duplicate_try_providers_once_per_protocol() {
    let try_items = |crate_id| TryLanguageItems {
        try_trait_id: DefId::new(CrateId(crate_id), LocalDefId(1)),
        output_id: AssocTypeId(0),
        residual_id: AssocTypeId(1),
        branch_method_id: DefId::new(CrateId(crate_id), LocalDefId(2)),
        from_residual_trait_id: DefId::new(CrateId(crate_id), LocalDefId(3)),
        from_residual_method_id: DefId::new(CrateId(crate_id), LocalDefId(4)),
        control_flow_enum_id: DefId::new(CrateId(crate_id), LocalDefId(5)),
        break_variant_id: VariantId(0),
        continue_variant_id: VariantId(1),
    };
    let crate_ctx = language_item_provider_context([
        (
            "zeta",
            LanguageItems {
                try_protocol: Some(try_items(7)),
                ..LanguageItems::default()
            },
        ),
        (
            "alpha",
            LanguageItems {
                try_protocol: Some(try_items(8)),
                ..LanguageItems::default()
            },
        ),
    ]);

    let Err(errors) = collect(
        &parse_language_item_source("struct App\n"),
        &crate_ctx,
        false,
        Some("app"),
    ) else {
        panic!("duplicate try providers should conflict");
    };

    assert_eq!(
        errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>(),
        vec!["multiple language-item providers claim protocol 'try': alpha, zeta"],
    );
}

#[test]
fn collect_reports_each_missing_try_protocol_component_deterministically() {
    let Err(errors) = collect_language_item_source("lang try\n< trait Carrier\n") else {
        panic!("incomplete try protocol should be rejected");
    };
    let messages = errors
        .into_iter()
        .map(|error| error.message)
        .collect::<Vec<_>>();

    assert_eq!(
        messages,
        vec![
            "language item try.output is missing".to_string(),
            "language item try.residual is missing".to_string(),
            "language item try.branch is missing".to_string(),
            "language item from_residual is missing".to_string(),
            "language item control_flow is missing".to_string(),
        ],
    );
}

#[test]
fn collect_binds_complete_try_protocol_across_inline_and_source_modules() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_language_items_{}_{}",
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
        temp_dir.join("flow.rk"),
        "lang control_flow\n< enum Flow\n    lang break\n    Stop\n    lang continue\n    Go\n",
    )
    .unwrap();
    let (mut program, graph) = load_source_program(
            root_path,
            "lang try\n< trait Carrier\n    lang output\n    type Value\n    lang residual\n    type Remainder\n    lang branch\n    split: I64\nmod flow\n",
            Some("test"),
        );
    let inline = crate::parser::parse_string(
        "lang from_residual\n< trait Rebuild\n    lang method\n    rebuild: I64\n",
        &crate::Config::default(),
    )
    .unwrap();
    program
        .module
        .top_levels
        .insert(1, inline_module("conversion", inline.module.top_levels));

    let decls =
        collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
            .expect("complete try protocol should collect");
    let try_items = decls
        .language_items
        .try_protocol
        .expect("try protocol should bind");
    let carrier_id = decls.item_index.defs_named("Carrier")[0];
    let rebuild_id = decls.item_index.defs_named("Rebuild")[0];
    let flow_id = decls.item_index.defs_named("Flow")[0];

    assert_eq!(try_items.try_trait_id, carrier_id);
    assert_eq!(try_items.output_id, crate::ids::AssocTypeId(0));
    assert_eq!(try_items.residual_id, crate::ids::AssocTypeId(1));
    assert_eq!(
        try_items.branch_method_id,
        decls.items.traits().get(&carrier_id).unwrap().signatures["split"].id,
    );
    assert_eq!(try_items.from_residual_trait_id, rebuild_id);
    assert_eq!(
        try_items.from_residual_method_id,
        decls.items.traits().get(&rebuild_id).unwrap().signatures["rebuild"].id,
    );
    assert_eq!(try_items.control_flow_enum_id, flow_id);
    assert_eq!(try_items.break_variant_id, crate::ids::VariantId(0));
    assert_eq!(try_items.continue_variant_id, crate::ids::VariantId(1));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn collect_rejects_duplicate_language_item_root_role() {
    let program =
        parse_language_item_source("lang sized\n< trait First\nlang sized\n< trait Second\n");
    let second_root = marker_span_bounds(&program, 1, None);

    let Err(errors) = collect_language_item_program(&program) else {
        panic!("duplicate root role should be rejected");
    };

    assert_eq!(
        error_details(errors),
        vec![(
            "language item sized is duplicated".to_string(),
            Some(second_root),
        )],
    );
}

#[test]
fn collect_rejects_duplicate_language_item_child_role() {
    let program = parse_language_item_source(
            "lang drop\n< trait Cleaner\n    lang method\n    first: I64\n    lang method\n    second: I64\n",
        );
    let second_method = marker_span_bounds(&program, 0, Some(1));

    let Err(errors) = collect_language_item_program(&program) else {
        panic!("duplicate child role should be rejected");
    };

    assert_eq!(
        error_details(errors),
        vec![(
            "language item drop.method is duplicated".to_string(),
            Some(second_method),
        )],
    );
}

#[test]
fn collect_rejects_language_item_child_role_in_wrong_protocol_context() {
    let program = parse_language_item_source(
        "lang drop\n< trait Cleaner\n    lang residual\n    clean: I64\n",
    );
    let root = marker_span_bounds(&program, 0, None);
    let residual = marker_span_bounds(&program, 0, Some(0));

    let Err(errors) = collect_language_item_program(&program) else {
        panic!("wrong child role context should be rejected");
    };

    assert_eq!(
        error_details(errors),
        vec![
            (
                "language item drop.method is missing".to_string(),
                Some(root),
            ),
            (
                "language item residual is not valid for drop".to_string(),
                Some(residual),
            ),
        ],
    );
}

#[test]
fn collect_rejects_language_item_child_with_wrong_declaration_kind() {
    let program = parse_language_item_source(
            "lang index\n< trait Lookup\n    lang output\n    output: I64\n    lang method\n    index: I64\n",
        );
    let root = marker_span_bounds(&program, 0, None);
    let output = marker_span_bounds(&program, 0, Some(0));

    let Err(errors) = collect_language_item_program(&program) else {
        panic!("wrong child declaration kind should be rejected");
    };

    assert_eq!(
        error_details(errors),
        vec![
            (
                "language item index.output must mark an associated type".to_string(),
                Some(output),
            ),
            (
                "language item index.output is missing".to_string(),
                Some(root),
            ),
        ],
    );
}

#[test]
fn collect_rejects_nonexported_language_item_root() {
    let program = parse_language_item_source("lang sized\ntrait Hidden\n");
    let root = marker_span_bounds(&program, 0, None);

    let Err(errors) = collect_language_item_program(&program) else {
        panic!("nonexported root should be rejected");
    };

    assert_eq!(
        error_details(errors),
        vec![(
            "language item sized root must be exported".to_string(),
            Some(root),
        )],
    );
}

#[test]
fn collect_rejects_language_item_marker_with_missing_member_target() {
    let mut program =
        parse_language_item_source("lang drop\n< trait Cleaner\n    lang method\n    clean: I64\n");
    let root = marker_span_bounds(&program, 0, None);
    let method = marker_span_bounds(&program, 0, Some(0));
    let TopLevel::TraitDecl(trait_decl) = &mut program.module.top_levels[0] else {
        panic!("expected marked trait");
    };
    trait_decl.language_items.members[0].member_name = "missing".to_string();
    let Err(errors) = collect_language_item_program(&program) else {
        panic!("missing language item member target should be rejected");
    };

    assert_eq!(
        error_details(errors),
        vec![
            (
                "language item drop.method targets missing member missing".to_string(),
                Some(method),
            ),
            (
                "language item drop.method is missing".to_string(),
                Some(root),
            ),
        ],
    );
}
