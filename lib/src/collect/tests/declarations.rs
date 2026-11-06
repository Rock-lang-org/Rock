use super::*;

#[test]
fn declaration_items_reject_key_payload_id_mismatch() {
    let key = DefId::new(CrateId(0), LocalDefId(1));
    let payload_id = DefId::new(CrateId(0), LocalDefId(2));
    let errors = DeclarationItems::from_id_maps(
        HashMap::from([(key, test_function(payload_id, "answer"))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
    )
    .unwrap_err();

    assert!(errors[0].message.contains("function declaration key"));
}

#[test]
fn collection_outputs_structs_keyed_by_canonical_def_id() {
    let declarations = collect(
        &program_with_struct("Point"),
        &crate::crate_system::CrateContext::new(),
        false,
        None,
    )
    .expect("collection should succeed");
    let point_id = DefId::new(CrateId(0), LocalDefId(0));

    assert_eq!(declarations.items.structs()[&point_id].id, point_id);
}

#[test]
fn local_collector_uses_source_records_before_current_id_repairs() {
    let source_path = PathBuf::from("/test/source.rk");
    let source_module = Module {
        name: Some(ident("source")),
        top_levels: vec![
            TopLevel::InfixOperator(7, crate::ast::Operator { value: "++".to_string(), span: crate::lexer::Span::new("test.rk".into(), 0, 0) }),
            TopLevel::StructDecl(StructDecl {
                name: type_inner("Same"),
                generic_params: vec![],
                fields: vec![],
                exported: false,
            }),
        ],
        is_inline: false,
        filepath: Some(source_path.clone()),
    };
    let module = Module {
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
                    exported: false,
                })],
            ),
            TopLevel::Mod(ident("source"), false),
            TopLevel::Impl(inherent_impl("Same", "first")),
            TopLevel::Impl(inherent_impl("Same", "second")),
        ],
        is_inline: false,
        filepath: None,
    };
    let mut source_modules = HashMap::new();
    source_modules.insert(
        vec!["test".to_string(), "source".to_string()],
        source_module,
    );
    let mut indexing_ids = IndexingIds::new_root();
    let root_module_id = indexing_ids.root_module_id();
    let item_index = index_root_module_items_with_sources(
        &mut indexing_ids,
        &module,
        Some("test"),
        &source_modules,
    );
    let mut id_environment = collected_id_environment_from_index(&item_index, root_module_id);
    populate_member_id_environment(
        &mut id_environment,
        &mut indexing_ids,
        &item_index,
        &module,
        Some("test"),
        &source_modules,
    );

    let mut context = context::CollectContext::bootstrap_for_collection(false, Some("test"), None);
    context
        .modules
        .add_path("test::source".to_string(), source_path.clone());
    context.modules.cache_module(
        source_path,
        source_modules[&vec!["test".to_string(), "source".to_string()]].clone(),
    );
    let mut collector = collector::LocalCollector::new_with_id_environment(context, id_environment);
    collector.collect_local_declarations(&module, root_module_id);
    let collected = collector.finish(&item_index, root_module_id, Some("test"));

    assert_eq!(
        collected.structs["Same"].id,
        item_index.item_at_source(root_module_id, 1).unwrap().def_id
    );
    let inline_module_id = item_index
        .module_id_by_path(&["inline".to_string()])
        .unwrap();
    assert_eq!(
        collected.structs["inline::Same"].id,
        item_index
            .item_at_source(inline_module_id, 0)
            .unwrap()
            .def_id
    );
    let source_module_id = item_index
        .module_id_by_path(&["source".to_string()])
        .unwrap();
    assert_eq!(
        collected.structs["test::source::Same"].id,
        item_index
            .item_at_source(source_module_id, 1)
            .unwrap()
            .def_id
    );
    assert_eq!(
        collected
            .impls
            .iter()
            .map(|impl_| impl_.id)
            .collect::<Vec<_>>(),
        vec![
            item_index.item_at_source(root_module_id, 4).unwrap().def_id,
            item_index.item_at_source(root_module_id, 5).unwrap().def_id,
        ]
    );
}

#[test]
fn collect_assigns_supported_current_crate_ids_before_lowering() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
                TopLevel::FunctionSig(function_sig(
                    "identity",
                    ParseType::Function(vec![ParseType::Type(type_inner("I64"))]),
                )),
                TopLevel::FunctionDecl(single_param_function_decl("identity", "value")),
                TopLevel::StructDecl(StructDecl {
                    name: type_inner("Box"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                }),
                TopLevel::EnumDecl(crate::ast::EnumDecl {
                    generic_params: Vec::new(),
                    name: type_inner("Maybe"),
                    variants: vec![crate::ast::EnumVariant {
                        name: type_inner("None"),
                        fields: crate::ast::NamedFieldsOrTypesList::TypesList(vec![]),
                    }],
                    exported: false,
                    language_items: Default::default(),
                }),
                TopLevel::TraitDecl(crate::ast::TraitDecl {
                    where_clauses: Vec::new(),
                    name: type_inner("Show"),
                    generic_params: vec![],
                    for_: None,
                    associated_types: vec![],
                    methods: HashMap::from([(ident("show"), function_decl("show"))]),
                    signatures: HashMap::from([(
                        ident("convert"),
                        function_sig(
                            "convert",
                            ParseType::Function(vec![
                                ParseType::Type(type_inner("T")),
                                ParseType::Type(type_inner("T")),
                            ]),
                        ),
                    )]),
                    exported: false,
                    language_items: Default::default(),
                }),
                TopLevel::Impl(trait_impl("Box", "Show", "show")),
                TopLevel::Extern(function_sig(
                    "puts",
                    ParseType::Function(vec![ParseType::Type(type_inner("I32"))]),
                )),
            ],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("collect assigns supported current-crate ids");

    assert_no_invalid_current_crate_ids(&decls);
}

#[test]
fn collect_generic_param_descriptor_fields_from_declarations() {
    let decls = collect(
        &generic_declaration_program(),
        &CrateContext::new(),
        false,
        Some("test"),
    )
    .expect("generic declaration collection should use indexed IDs during construction");

    assert_generic_declaration_ids(&decls);
}

#[test]
fn collect_errors_when_trait_impl_defines_unknown_associated_type() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
                TopLevel::StructDecl(StructDecl {
                    name: type_inner("Box"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                }),
                TopLevel::TraitDecl(crate::ast::TraitDecl {
                    where_clauses: Vec::new(),
                    name: type_inner("Iterable"),
                    generic_params: vec![],
                    for_: None,
                    associated_types: vec![crate::ast::AssociatedTypeDecl {
                        name: ident("Item"),
                        kind: None,
                    }],
                    methods: HashMap::new(),
                    signatures: HashMap::new(),
                    exported: false,
                    language_items: Default::default(),
                }),
                TopLevel::Impl(Impl {
                    name: ParseType::Type(type_inner("Iterable")),
                    for_: Some(ParseType::Type(type_inner("Box"))),
                    associated_types: vec![crate::ast::AssociatedTypeDef {
                        name: ident("Output"),
                        kind: None,
                        ty: ParseType::Type(type_inner("I64")),
                    }],
                    methods: HashMap::new(),
                    signatures: HashMap::new(),
                    where_clauses: vec![],
                }),
            ],
            is_inline: false,
            filepath: None,
        },
    };

    let errors = match collect(&program, &CrateContext::new(), false, Some("test")) {
        Ok(_) => panic!("unknown trait associated type should be a collection error"),
        Err(errors) => errors,
    };

    assert!(errors
        .iter()
        .any(|err| err.message.contains("unknown associated type 'Output'")));
}

fn add_artifact_extern_crate_with_language_items(
    crate_ctx: &mut CrateContext,
    crate_name: &str,
    interface: ArtifactCrateInterface,
    resolver: crate::collect::resolver::ResolverTables,
    language_items: LanguageItems<DefId>,
) {
    crate_ctx
        .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
            CrateId(1),
            crate_name.to_string(),
            crate::crate_system::ExternCrateMetadata::new(interface, resolver, BTreeMap::new())
                .with_language_items(language_items),
            crate::crate_system::ExternCrateBodies::default(),
            crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .unwrap();
}

#[test]
fn generated_current_marked_sized_impl_is_emitted_to_products() {
    let sized_id = DefId::new(CrateId(7), LocalDefId(1));
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_trait(
        "core::StaticLayout".to_string(),
        hir_trait(sized_id, "StaticLayout", &[]),
    );
    let mut resolver = crate::collect::resolver::ResolverTables::default();
    resolver
        .item_paths
        .insert("core::StaticLayout".to_string(), sized_id);
    resolver
        .item_names_by_id
        .insert(sized_id, "core::StaticLayout".to_string());
    let mut crate_ctx = CrateContext::new();
    add_artifact_extern_crate_with_language_items(
        &mut crate_ctx,
        "core",
        interface,
        resolver,
        LanguageItems {
            sized: Some(SizedLanguageItems { trait_id: sized_id }),
            ..LanguageItems::default()
        },
    );

    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![TopLevel::StructDecl(StructDecl {
                name: type_inner("Point"),
                generic_params: vec![],
                fields: vec![],
                exported: false,
            })],
            is_inline: false,
            filepath: None,
        },
    };
    let decls =
        collect(&program, &crate_ctx, false, Some("demo")).expect("collection should succeed");
    let partial =
        crate::lower::program::lower_from_declarations(&program, decls, &crate_ctx, Some("demo"))
            .expect("lowering should auto-impl the marked trait");
    let resolved = crate::infer::finalize(partial).expect("inference should finalize");
    let sized_impl = resolved
        .program
        .impls_in_order()
        .map(|(_, imp)| imp)
        .find(|imp| imp.type_name == "Point" && imp.trait_name.as_deref() == Some("StaticLayout"))
        .expect("Point should have generated marked Sized impl");
    assert_eq!(sized_impl.trait_id, Some(sized_id));

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        std::collections::BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("generated marked Sized HIR has valid product language items");

    assert!(products
        .interface
        .impls
        .contains_key(&ProductDefId::from(sized_impl.id)));
}

#[test]
fn collect_module_defines_local_collector() {
    let context = context::CollectContext::bootstrap_for_collection(false, None, None);
    let _ = super::super::collector::LocalCollector::new(context);
}

#[test]
fn collect_builds_reverse_canonical_tables_for_current_crate_items() {
    let fixture = current_crate_canonical_fixture();
    let decls = collect_with_source_graph(
        &fixture.program,
        &fixture.graph,
        &CrateContext::new(),
        false,
        Some("test"),
    )
    .expect("collect should build canonical resolver tables");

    let root_thing_id = decls.item_index.defs_named("RootThing")[0];
    let vector_id = decls.item_index.defs_named("Vector")[0];
    let writer_id = decls.item_index.defs_named("Writer")[0];
    let io_module_def_id = decls.item_index.defs_named("io")[0];

    let math_module_id = *decls
        .resolver
        .module_paths
        .get("math")
        .expect("math module should be indexed");
    let io_module_id = *decls
        .resolver
        .module_paths
        .get("test::io")
        .expect("test::io module should be indexed");

    assert_eq!(
        decls.resolver.item_paths.get("math"),
        Some(&decls.item_index.defs_named("math")[0])
    );
    assert_eq!(
        decls.resolver.item_paths.get("test::io"),
        Some(&io_module_def_id)
    );

    assert_eq!(
        decls
            .resolver
            .module_names_by_id
            .get(&math_module_id)
            .map(String::as_str),
        Some("math")
    );
    assert_eq!(
        decls
            .resolver
            .module_names_by_id
            .get(&io_module_id)
            .map(String::as_str),
        Some("test::io")
    );

    assert_eq!(
        decls.resolver.item_paths.get("RootThing"),
        Some(&root_thing_id)
    );
    assert_eq!(
        decls.resolver.item_paths.get("math::Vector"),
        Some(&vector_id)
    );
    assert_eq!(
        decls.resolver.item_paths.get("test::io::Writer"),
        Some(&writer_id)
    );

    assert_eq!(
        decls
            .resolver
            .item_names_by_id
            .get(&root_thing_id)
            .map(String::as_str),
        Some("RootThing")
    );
    assert_eq!(
        decls
            .resolver
            .item_names_by_id
            .get(&vector_id)
            .map(String::as_str),
        Some("math::Vector")
    );
    assert_eq!(
        decls
            .resolver
            .item_names_by_id
            .get(&writer_id)
            .map(String::as_str),
        Some("test::io::Writer")
    );
    assert_eq!(
        decls
            .resolver
            .item_names_by_id
            .get(&io_module_def_id)
            .map(String::as_str),
        Some("test::io")
    );
}

#[test]
fn lowered_hir_indexes_trait_defaults_and_impl_methods_by_def_id() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
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
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("collect should assign method DefIds");
    let trait_default_id = trait_named(&decls, "Show")
        .and_then(|trait_def| trait_def.methods.get("show"))
        .map(|method| method.id)
        .expect("trait default method should have a DefId");
    let trait_id = trait_named(&decls, "Show")
        .map(|trait_def| trait_def.id)
        .expect("trait should have a DefId");
    let impl_id = decls
        .items
        .impls()
        .values()
        .next()
        .map(|imp| imp.id)
        .expect("impl should have a DefId");
    let impl_method_id = decls
        .items
        .impls()
        .values()
        .next()
        .and_then(|imp| imp.methods.get("show"))
        .map(|method| method.id)
        .expect("impl method should have a DefId");

    let partial = crate::lower::program::lower_from_declarations(
        &program,
        decls,
        &CrateContext::new(),
        Some("test"),
    )
    .expect("lowering should preserve method DefIds");
    let resolved = crate::infer::finalize(partial).expect("inference should finalize methods");

    assert_eq!(
        resolved
            .program
            .indexes
            .methods_by_id
            .get(&trait_default_id),
        Some(&HirMethodLocation::TraitDefault {
            trait_id,
            method_id: trait_default_id,
            method_name: "show".to_string(),
        })
    );
    assert_eq!(
        resolved.program.indexes.methods_by_id.get(&impl_method_id),
        Some(&HirMethodLocation::ImplMethod {
            impl_id,
            method_id: impl_method_id,
            method_name: "show".to_string(),
        })
    );
}

#[test]
fn collection_rejects_supertrait_cycles_with_user_facing_path() {
    let program = crate::parser::parse_string(
        "trait First for F _ where F: Second\n\ntrait Second for F _ where F: First\n",
        &crate::Config::default(),
    )
    .expect("cyclic supertraits should parse before collection validation");

    let errors = match collect(&program, &CrateContext::new(), false, Some("test")) {
        Ok(_) => panic!("supertrait cycle should be rejected"),
        Err(errors) => errors,
    };
    let message = errors
        .iter()
        .map(|error| error.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        message.contains("supertrait cycle detected"),
        "unexpected diagnostics: {message}"
    );
    assert!(message.contains("First"));
    assert!(message.contains("Second"));
    assert!(!message.contains("trait#"));
}

#[test]
fn collection_rejects_missing_supertrait_by_source_name() {
    let program = crate::parser::parse_string(
        "trait Child for F _ where F: Missing\n",
        &crate::Config::default(),
    )
    .expect("missing supertrait should parse before collection validation");

    let errors = match collect(&program, &CrateContext::new(), false, Some("test")) {
        Ok(_) => panic!("missing supertrait should be rejected"),
        Err(errors) => errors,
    };

    assert!(errors.iter().any(|error| error
        .message
        .contains("unknown trait 'Missing' in where clause")));
}

#[test]
fn typed_predicate_constructor_declaration_sets_generic_kind_and_obligation() {
    let program = crate::parser::parse_string(
        "trait Functor for F _\n\napply_f: F A -> F A where F _: Functor\n",
        &crate::Config::default(),
    )
    .expect("constructor predicate source should parse");

    let declarations = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("constructor predicate should collect");
    let signature = function_sig_named(&declarations, "apply_f").unwrap();
    let functor = trait_named(&declarations, "Functor").unwrap();
    let constructor = signature
        .generic_params
        .iter()
        .find(|param| param.name == "F")
        .expect("F generic descriptor");

    assert_eq!(
        constructor.kind,
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        )
    );
    assert_eq!(
        signature.generic_bounds.predicates,
        vec![crate::types::Predicate::Trait {
            subject: Type::Generic(constructor.id),
            trait_id: functor.id,
            args: Vec::new(),
        }]
    );
}

#[test]
fn collect_rejects_try_without_from_residual_or_control_flow() {
    let program = parse_language_item_source(
            "lang try\n< trait Carrier\n    lang output\n    type Value\n    lang residual\n    type Remainder\n    lang branch\n    split: I64\n",
        );
    let root = marker_span_bounds(&program, 0, None);

    let Err(errors) = collect_language_item_program(&program) else {
        panic!("partial try family should be rejected");
    };

    assert_eq!(
        error_details(errors),
        vec![
            (
                "language item from_residual is missing".to_string(),
                Some(root)
            ),
            (
                "language item control_flow is missing".to_string(),
                Some(root)
            ),
        ],
    );
}

#[test]
fn collect_rejects_from_residual_without_try_or_control_flow() {
    let program = parse_language_item_source(
        "lang from_residual\n< trait Rebuild\n    lang method\n    rebuild: I64\n",
    );
    let root = marker_span_bounds(&program, 0, None);

    let Err(errors) = collect_language_item_program(&program) else {
        panic!("orphan from_residual should be rejected");
    };

    assert_eq!(
        error_details(errors),
        vec![
            ("language item try is missing".to_string(), Some(root)),
            (
                "language item control_flow is missing".to_string(),
                Some(root)
            ),
        ],
    );
}

#[test]
fn collect_rejects_control_flow_without_try_or_from_residual() {
    let program = parse_language_item_source(
        "lang control_flow\n< enum Flow\n    lang break\n    Stop\n    lang continue\n    Next\n",
    );
    let root = marker_span_bounds(&program, 0, None);

    let Err(errors) = collect_language_item_program(&program) else {
        panic!("orphan control_flow should be rejected");
    };

    assert_eq!(
        error_details(errors),
        vec![
            ("language item try is missing".to_string(), Some(root)),
            (
                "language item from_residual is missing".to_string(),
                Some(root)
            ),
        ],
    );
}
