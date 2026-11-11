use super::*;

fn product_def_id(local_id: u32) -> ProductDefId {
    ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(local_id),
    }
}

fn resolved_hir_with_language_items(
    language_items: LanguageItems<DefId>,
    current_def_ids: BTreeSet<DefId>,
) -> ResolvedHirProgram {
    let mut traits = HashMap::new();
    if let Some(items) = &language_items.sized {
        traits.insert(
            items.trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: items.trait_id,
                name: "SizedProvider".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
    }
    if let Some(items) = &language_items.index {
        let key = Type::Generic(GenericParamId {
            owner: items.trait_id,
            index: 0,
        });
        let self_ty = Type::Generic(GenericParamId {
            owner: items.trait_id,
            index: 1,
        });
        traits.insert(
            items.trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: items.trait_id,
                name: "IndexProvider".to_string(),
                generic_params: generic_params(items.trait_id, &["Key"]),
                associated_types: vec![HirAssociatedTypeDecl {
                    id: items.output_id,
                    name: "Output".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                }],
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "index".to_string(),
                    HirFunctionSig {
                        id: items.method_id,
                        name: "index".to_string(),
                        generic_params: Vec::new(),
                        params: vec![
                            Type::Reference {
                                mutable: false,
                                inner: Box::new(self_ty.clone()),
                            },
                            key.clone(),
                        ],
                        ret: Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::Projection {
                                ty: Box::new(self_ty),
                                trait_id: items.trait_id,
                                assoc_type: AssociatedTypeKey {
                                    owner: items.trait_id,
                                    assoc_type_id: items.output_id,
                                },
                                trait_args: vec![key],
                            }),
                        },
                        generic_bounds: HashMap::new().into(),
                        self_receiver: Some(crate::types::ReceiverMode::Shared),
                        is_unsafe: false,
                    },
                )]),
            },
        );
    }
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        traits,
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        language_items,
        &HashMap::new(),
    );
    let accepted =
        AcceptedHirProgram::try_from(program).expect("product fixture must construct accepted HIR");
    ResolvedHirProgram::from_accepted(
        accepted,
        ResolverTables::default(),
        current_def_ids,
        CrateId(0),
        IdGen::new(),
    )
}

#[test]
fn product_artifact_roundtrip_preserves_complete_language_item_registry() {
    let language_items = ProductLanguageItems {
        fold: None,
        sized: Some(SizedLanguageItems {
            trait_id: product_def_id(17),
        }),
        drop: Some(DropLanguageItems {
            trait_id: product_def_id(23),
            method_id: product_def_id(29),
        }),
        index: Some(IndexLanguageItems {
            trait_id: product_def_id(31),
            output_id: AssocTypeId(37),
            method_id: product_def_id(41),
        }),
        index_mut: Some(IndexMutLanguageItems {
            trait_id: product_def_id(11),
            output_id: AssocTypeId(0),
            method_id: product_def_id(12),
        }),
        fn_once: None,
        fn_mut: None,
        fn_trait: None,
        send: None,
        owned: Some(crate::language_items::OwnedLanguageItems { trait_id: product_def_id(89) }),
        sync: None,
        try_protocol: Some(TryLanguageItems {
            try_trait_id: product_def_id(43),
            output_id: AssocTypeId(47),
            residual_id: AssocTypeId(53),
            branch_method_id: product_def_id(59),
            from_residual_trait_id: product_def_id(61),
            from_residual_method_id: product_def_id(67),
            control_flow_enum_id: product_def_id(71),
            break_variant_id: VariantId(73),
            continue_variant_id: VariantId(79),
        }),
        range: Some(RangeLanguageItems {
            enum_id: product_def_id(83),
            full_variant_id: VariantId(0),
            from_variant_id: VariantId(1),
            to_variant_id: VariantId(2),
            to_inclusive_variant_id: VariantId(3),
            exclusive_variant_id: VariantId(4),
            inclusive_variant_id: VariantId(5),
        }),
    };
    let mut products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("language-items".to_string()),
        identity_table: ProductIdentityTable::default(),
        interface: super::super::ProductInterface {
            language_items: language_items.clone(),
            ..super::super::ProductInterface::default()
        },
        bodies: ProductBodies::default(),
        link: ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: BTreeMap::new(),
        proc_macros: Vec::new(),
    };
    for (local_id, receiver) in [
        None,
        Some(crate::types::ReceiverMode::Shared),
        Some(crate::types::ReceiverMode::Mut),
        Some(crate::types::ReceiverMode::Move),
    ]
    .into_iter()
    .enumerate()
    {
        let local_id = local_id as u32 + 100;
        products.interface.functions.insert(
            product_def_id(local_id),
            super::super::ProductFunctionInterface {
                id: DefId::new(CrateId(0), LocalDefId(local_id)),
                name: format!("receiver_{local_id}"),
                generic_params: Vec::new(),
                generic_bounds: HashMap::new().into(),
                params: Vec::new(),
                ret_type: Type::Unit,
                is_curried: false,
                is_method: receiver.is_some(),
                self_receiver: receiver,
                is_unsafe: false,
                constant_arithmetic: None,
            },
        );
    }
    let body_id = DefId::new(CrateId(0), LocalDefId(104));
    let mut body = test_function(body_id, "body_receiver", Vec::new());
    body.is_method = true;
    body.self_receiver = Some(crate::types::ReceiverMode::Move);
    products
        .bodies
        .functions
        .insert(product_def_id(104), accept_function(body));
    products.interface.traits.insert(
        product_def_id(105),
        super::super::ProductTraitInterface {
            target: None,
            predicates: Vec::new(),
            id: DefId::new(CrateId(0), LocalDefId(105)),
            name: "ReceiverTrait".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: BTreeMap::new(),
            signatures: HashMap::from([(
                "receive".to_string(),
                HirFunctionSig {
                    id: DefId::new(CrateId(0), LocalDefId(106)),
                    name: "receive".to_string(),
                    generic_params: Vec::new(),
                    params: Vec::new(),
                    ret: Type::Unit,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(crate::types::ReceiverMode::Mut),
                    is_unsafe: false,
                },
            )]),
        },
    );
    products.identity_table.local_crate = Some(ProductCrateId(0));

    let bytes = products.to_artifact_bytes().expect("artifact serializes");
    let decoded = CompilerProducts::from_artifact_bytes(&bytes).expect("artifact decodes");

    assert_eq!(decoded.interface.language_items, language_items);
    assert_eq!(
        decoded
            .interface
            .functions
            .values()
            .map(|function| function.self_receiver)
            .collect::<Vec<_>>(),
        vec![
            None,
            Some(crate::types::ReceiverMode::Shared),
            Some(crate::types::ReceiverMode::Mut),
            Some(crate::types::ReceiverMode::Move),
        ]
    );
    assert_eq!(
        decoded.bodies.functions[&product_def_id(104)].self_receiver,
        Some(crate::types::ReceiverMode::Move)
    );
    assert_eq!(
        decoded.interface.traits[&product_def_id(105)].signatures["receive"].self_receiver,
        Some(crate::types::ReceiverMode::Mut)
    );
}

#[test]
fn product_mapping_omits_dependency_sized_bundle() {
    let dependency_sized_trait = DefId::new(CrateId(9), LocalDefId(17));
    let hir = resolved_hir_with_language_items(
        LanguageItems {
            sized: Some(SizedLanguageItems {
                trait_id: dependency_sized_trait,
            }),
            ..LanguageItems::default()
        },
        BTreeSet::new(),
    );

    let products = products_from_hir(&hir).expect("dependency-only bundle is omitted");

    assert_eq!(
        products.interface.language_items,
        ProductLanguageItems::default()
    );
}

#[test]
fn product_mapping_maps_current_index_bundle_preserving_output_id() {
    let index_trait = DefId::new(CrateId(0), LocalDefId(17));
    let index_method = DefId::new(CrateId(0), LocalDefId(19));
    let hir = resolved_hir_with_language_items(
        LanguageItems {
            index: Some(IndexLanguageItems {
                trait_id: index_trait,
                output_id: AssocTypeId(23),
                method_id: index_method,
            }),
            ..LanguageItems::default()
        },
        BTreeSet::from([index_trait, index_method]),
    );

    let products = products_from_hir(&hir).expect("current bundle maps");

    assert_eq!(
        products.interface.language_items.index,
        Some(IndexLanguageItems {
            trait_id: ProductDefId::from(index_trait),
            output_id: AssocTypeId(23),
            method_id: ProductDefId::from(index_method),
        })
    );
}

#[test]
fn product_mapping_rejects_mixed_drop_bundle() {
    let current_drop_trait = DefId::new(CrateId(0), LocalDefId(17));
    let dependency_drop_method = DefId::new(CrateId(9), LocalDefId(19));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        LanguageItems {
            drop: Some(DropLanguageItems {
                trait_id: current_drop_trait,
                method_id: dependency_drop_method,
            }),
            ..LanguageItems::default()
        },
        &HashMap::new(),
    );

    let error = super::product_language_items_from_program(
        &program,
        &BTreeSet::from([current_drop_trait]),
        &super::super::ProductIdRemap::new(),
    )
    .expect_err("mixed bundle is an invariant error");

    assert!(error.contains("drop"));
}

#[test]
fn product_mapping_rejects_current_index_mut_without_current_index() {
    let dependency_index_trait = DefId::new(CrateId(9), LocalDefId(17));
    let dependency_index_method = DefId::new(CrateId(9), LocalDefId(19));
    let index_mut_trait = DefId::new(CrateId(0), LocalDefId(23));
    let index_mut_method = DefId::new(CrateId(0), LocalDefId(29));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        LanguageItems {
            index: Some(IndexLanguageItems {
                trait_id: dependency_index_trait,
                output_id: AssocTypeId(0),
                method_id: dependency_index_method,
            }),
            index_mut: Some(IndexMutLanguageItems {
                trait_id: index_mut_trait,
                output_id: AssocTypeId(0),
                method_id: index_mut_method,
            }),
            ..LanguageItems::default()
        },
        &HashMap::new(),
    );

    let mut remap = super::super::ProductIdRemap::new();
    for id in [index_mut_trait, index_mut_method] {
        remap.insert(
            ProductDefId::from(id),
            BTreeSet::from([ProductDefId::from(id)]),
        );
    }
    let error = super::product_language_items_from_program(
        &program,
        &BTreeSet::from([index_mut_trait, index_mut_method]),
        &remap,
    )
    .expect_err("current IndexMut without current Index is an invariant error");

    assert_eq!(
            error,
            "language item index/index_mut provider pair mixes current-crate and dependency definitions"
        );
}

#[test]
fn product_mapping_rejects_current_index_without_current_index_mut() {
    let index_trait = DefId::new(CrateId(0), LocalDefId(17));
    let index_method = DefId::new(CrateId(0), LocalDefId(19));
    let dependency_index_mut_trait = DefId::new(CrateId(9), LocalDefId(23));
    let dependency_index_mut_method = DefId::new(CrateId(9), LocalDefId(29));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        LanguageItems {
            index: Some(IndexLanguageItems {
                trait_id: index_trait,
                output_id: AssocTypeId(0),
                method_id: index_method,
            }),
            index_mut: Some(IndexMutLanguageItems {
                trait_id: dependency_index_mut_trait,
                output_id: AssocTypeId(0),
                method_id: dependency_index_mut_method,
            }),
            ..LanguageItems::default()
        },
        &HashMap::new(),
    );

    let mut remap = super::super::ProductIdRemap::new();
    for id in [index_trait, index_method] {
        remap.insert(
            ProductDefId::from(id),
            BTreeSet::from([ProductDefId::from(id)]),
        );
    }
    let error = super::product_language_items_from_program(
        &program,
        &BTreeSet::from([index_trait, index_method]),
        &remap,
    )
    .expect_err("current Index without current IndexMut is an invariant error");

    assert_eq!(
            error,
            "language item index/index_mut provider pair mixes current-crate and dependency definitions"
        );
}

#[test]
fn product_mapping_maps_current_index_mut_bundle_preserving_output_id() {
    let index_trait = DefId::new(CrateId(0), LocalDefId(17));
    let index_method = DefId::new(CrateId(0), LocalDefId(19));
    let index_mut_trait = DefId::new(CrateId(0), LocalDefId(23));
    let index_mut_method = DefId::new(CrateId(0), LocalDefId(29));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables::default(),
        LanguageItems {
            index: Some(IndexLanguageItems {
                trait_id: index_trait,
                output_id: AssocTypeId(31),
                method_id: index_method,
            }),
            index_mut: Some(IndexMutLanguageItems {
                trait_id: index_mut_trait,
                output_id: AssocTypeId(37),
                method_id: index_mut_method,
            }),
            ..LanguageItems::default()
        },
        &HashMap::new(),
    );
    let mut remap = super::super::ProductIdRemap::new();
    for id in [index_trait, index_method, index_mut_trait, index_mut_method] {
        remap.insert(
            ProductDefId::from(id),
            BTreeSet::from([ProductDefId::from(id)]),
        );
    }

    let items = super::product_language_items_from_program(
        &program,
        &BTreeSet::from([index_trait, index_method, index_mut_trait, index_mut_method]),
        &remap,
    )
    .expect("current index pair maps");

    assert_eq!(
        items.index_mut,
        Some(IndexMutLanguageItems {
            trait_id: ProductDefId::from(index_mut_trait),
            output_id: AssocTypeId(37),
            method_id: ProductDefId::from(index_mut_method),
        })
    );
}
