use super::*;

fn enum_named<'a>(decls: &'a Declarations, name: &str) -> Option<&'a HirEnum> {
    decls.items.enums().values().find(|enumeration| {
        decls
            .resolver
            .item_names_by_id
            .get(&enumeration.id)
            .map(String::as_str)
            == Some(name)
            || enumeration.name == name
    })
}

#[test]
fn collect_carries_indexing_ids_used_by_item_index() {
    let program = program_with_struct("Point");
    let crate_ctx = CrateContext::new();

    let decls = match collect(&program, &crate_ctx, false, Some("test")) {
        Ok(decls) => decls,
        Err(_) => panic!("collect should succeed"),
    };

    assert_eq!(decls.indexing_ids.root_crate_id(), CrateId(0));
    assert_eq!(decls.indexing_ids.root_module_id(), ModuleId(0));

    let point_id = decls.item_index.defs_named("Point")[0];
    let point = decls.item_index.get(point_id).unwrap();

    assert_eq!(point.def_id, DefId::new(CrateId(0), LocalDefId(0)));
    assert_eq!(point.def_id.crate_id, decls.indexing_ids.root_crate_id());
    assert_eq!(point.module_id, decls.indexing_ids.root_module_id());
}

#[test]
fn collect_remaps_signature_backed_function_generics_to_function_owner() {
    let generic = ParseType::Type(type_inner("A"));
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
                TopLevel::FunctionSig(function_sig(
                    "id",
                    ParseType::Function(vec![generic.clone(), generic]),
                )),
                TopLevel::FunctionDecl(single_param_function_decl("id", "value")),
            ],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("collect should merge function signature and declaration");
    let function =
        function_named(&decls, "id").expect("signature-backed function should be collected");
    let generic_id = GenericParamId {
        owner: function.id,
        index: 0,
    };

    assert_eq!(
        function.generic_params,
        vec![GenericParamDecl::type_param(generic_id, "A")]
    );
    assert_eq!(function.params[0].ty, Type::Generic(generic_id));
    assert_eq!(function.ret_type, Type::Generic(generic_id));
}

#[test]
fn collect_keeps_item_index_and_local_declarations_in_sync() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
                TopLevel::StructDecl(StructDecl {
                    name: type_inner("Point"),
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
            ],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("collect should succeed");

    assert!(struct_named(&decls, "Point").is_some());
    assert!(struct_named(&decls, "Vector").is_some());
    assert_eq!(decls.item_index.defs_named("Point").len(), 1);
    assert_eq!(decls.item_index.defs_named("Vector").len(), 1);
}

#[test]
fn collect_assigns_canonical_def_ids_to_named_function_and_type_headers() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
                TopLevel::FunctionDecl(function_decl("root_fn")),
                TopLevel::StructDecl(StructDecl {
                    name: type_inner("Point"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                }),
                TopLevel::EnumDecl(crate::ast::EnumDecl {
                    generic_params: Vec::new(),
                    name: type_inner("Color"),
                    variants: vec![],
                    exported: false,
                    language_items: Default::default(),
                }),
                TopLevel::TraitDecl(crate::ast::TraitDecl {
                    where_clauses: Vec::new(),
                    name: type_inner("Show"),
                    generic_params: vec![],
                    for_: None,
                    associated_types: vec![],
                    methods: HashMap::new(),
                    signatures: HashMap::new(),
                    exported: false,
                    language_items: Default::default(),
                }),
                inline_module(
                    "math",
                    vec![TopLevel::FunctionDecl(function_decl("vector_len"))],
                ),
            ],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("collect should assign canonical ids to named headers");

    assert_eq!(
        function_named(&decls, "root_fn").unwrap().id,
        *decls.resolver.item_paths.get("root_fn").unwrap()
    );
    assert_eq!(
        struct_named(&decls, "Point").unwrap().id,
        *decls.resolver.item_paths.get("Point").unwrap()
    );
    assert_eq!(
        enum_named(&decls, "Color").unwrap().id,
        *decls.resolver.item_paths.get("Color").unwrap()
    );
    assert_eq!(
        trait_named(&decls, "Show").unwrap().id,
        *decls.resolver.item_paths.get("Show").unwrap()
    );
    assert_eq!(
        function_named(&decls, "math::vector_len").unwrap().id,
        *decls.resolver.item_paths.get("math::vector_len").unwrap()
    );
}

#[test]
fn collect_keeps_named_item_ids_separate_from_impl_ids() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
                TopLevel::StructDecl(StructDecl {
                    name: type_inner("Foo"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                }),
                trait_decl("Display"),
                trait_decl("Debug"),
                TopLevel::Impl(inherent_impl("Foo", "new")),
                TopLevel::Impl(trait_impl("Foo", "Display", "fmt")),
                TopLevel::Impl(trait_impl("Foo", "Debug", "fmt")),
            ],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("collect should assign canonical ids");

    let struct_id = decls
        .item_index
        .items()
        .iter()
        .find(|item| {
            item.kind == crate::collect::item_index::ItemKind::Struct && item.name == "Foo"
        })
        .map(|item| item.def_id)
        .expect("Foo struct should be indexed");
    let display_trait_id = decls
        .item_index
        .items()
        .iter()
        .find(|item| {
            item.kind == crate::collect::item_index::ItemKind::Trait && item.name == "Display"
        })
        .map(|item| item.def_id)
        .expect("Display trait should be indexed");
    let impl_ids = decls
        .item_index
        .items()
        .iter()
        .filter(|item| item.kind == crate::collect::item_index::ItemKind::Impl)
        .map(|item| item.def_id)
        .collect::<Vec<_>>();

    assert_eq!(decls.resolver.item_paths.get("Foo"), Some(&struct_id));
    assert_eq!(
        decls.resolver.item_paths.get("Display"),
        Some(&display_trait_id)
    );
    assert_eq!(impl_ids.len(), 3);
    assert_eq!(
        decls.items.impls().keys().copied().collect::<BTreeSet<_>>(),
        impl_ids.into_iter().collect()
    );
}

#[test]
fn collect_assigns_unique_def_ids_to_local_trait_and_impl_methods() {
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
        .expect("collect should assign canonical method ids");
    let trait_id = trait_named(&decls, "Show").unwrap().id;
    let impl_ = decls.items.impls().values().next().unwrap();
    let impl_id = impl_.id;
    let trait_method_id = trait_named(&decls, "Show").unwrap().methods["show"].id;
    let impl_method_id = impl_.methods["show"].id;

    assert_ne!(trait_method_id, trait_id);
    assert_ne!(impl_method_id, impl_id);
    assert_ne!(trait_method_id, impl_method_id);
    assert!(!decls
        .resolver
        .item_names_by_id
        .contains_key(&trait_method_id));
    assert!(!decls
        .resolver
        .item_names_by_id
        .contains_key(&impl_method_id));
    assert_eq!(impl_.methods["show"].id, impl_method_id);
    assert!(decls.current_def_ids.contains(&trait_method_id));
    assert!(decls.current_def_ids.contains(&impl_method_id));
}

#[test]
fn collect_current_def_ids_do_not_include_overwritten_member_ids() {
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
                    signatures: HashMap::from([(
                        ident("convert"),
                        function_sig(
                            "convert",
                            ParseType::Function(vec![ParseType::Type(type_inner("I64"))]),
                        ),
                    )]),
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
        .expect("collect should assign explicit member ids");
    let mut used_ids = BTreeSet::new();
    used_ids.extend(decls.item_index.items().iter().map(|item| item.def_id));
    for trait_def in decls.items.traits().values() {
        used_ids.extend(trait_def.methods.values().map(|method| method.id));
        used_ids.extend(trait_def.signatures.values().map(|sig| sig.id));
    }
    for imp in decls.items.impls().values() {
        used_ids.extend(imp.methods.values().map(|method| method.id));
    }

    let orphan_ids = decls
        .current_def_ids
        .difference(&used_ids)
        .copied()
        .collect::<Vec<_>>();
    assert!(
        orphan_ids.is_empty(),
        "current_def_ids contains IDs not used by collected declarations: {orphan_ids:?}"
    );
}

#[test]
fn collect_assigns_inline_module_trait_and_impl_member_ids() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
                trait_decl("Show"),
                inline_module(
                    "math",
                    vec![
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
                ),
            ],
            is_inline: false,
            filepath: None,
        },
    };

    let decls = collect(&program, &CrateContext::new(), false, Some("test"))
        .expect("inline module trait and impl members should use indexed IDs");
    let trait_def = decls
        .items
        .traits()
        .values()
        .find(|trait_def| trait_def.methods.contains_key("show"))
        .expect("trait should be collected");
    let impl_def = decls
        .items
        .impls()
        .values()
        .find(|imp| imp.trait_name.as_deref() == Some("Show"))
        .expect("impl should be collected");

    assert!(decls.current_def_ids.contains(&trait_def.id));
    assert!(decls.current_def_ids.contains(&impl_def.id));
    assert!(decls
        .current_def_ids
        .contains(&trait_def.methods["show"].id));
    assert!(decls.current_def_ids.contains(&impl_def.methods["show"].id));
}

#[test]
fn collect_assigns_canonical_id_to_signature_backed_function_header() {
    let program = Program {
        module: Module {
            name: None,
            top_levels: vec![
                TopLevel::FunctionSig(FunctionSig {
                    name: ident("identity"),
                    sig: ParseType::Function(vec![
                        ParseType::Type(type_inner("T")),
                        ParseType::Type(type_inner("T")),
                    ]),
                    where_clauses: vec![],
                    self_receiver: None,
                    is_unsafe: false,
                    exported: false,
                }),
                TopLevel::FunctionDecl(FunctionDecl {
                    name: ident("identity"),
                    lambda: LambdaDecl {
                        parameters: vec![Pattern {
                            binding: None,
                            kind: PatternKind::Ident(crate::ast::IdentPattern {
                                name: ident("value"),
                                mut_: false,
                            }),
                        }],
                        body: crate::ast::Block { statements: vec![] },
                        arrow_kind: LambdaArrowKind::Normal,
                        span: crate::lexer::Span::test(),
                    },
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
        .expect("collect should assign canonical IDs");
    let function =
        function_named(&decls, "identity").expect("identity function should be collected");

    assert_ne!(function.id, DefId::new(CrateId(0), LocalDefId(0)));
    assert!(decls.current_def_ids.contains(&function.id));
    assert!(
        function
            .generic_params
            .iter()
            .map(|generic| generic.id)
            .all(|generic_id| generic_id.owner == function.id),
        "signature-backed generic owners should use the canonical function ID"
    );
}
