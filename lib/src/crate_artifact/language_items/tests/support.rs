use super::*;

pub(super) static ARTIFACT_CASE_ID: AtomicU64 = AtomicU64::new(0);

pub(super) fn product_id(local_id: u32) -> crate::products::ProductDefId {
    crate::products::ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(local_id),
    }
}

pub(super) fn signature(
    id: crate::products::ProductDefId,
    receiver: Option<ReceiverMode>,
    params: Vec<Type>,
    ret: Type,
) -> HirFunctionSig {
    HirFunctionSig {
        id: product_def_id(id),
        name: format!("m{}", id.local_id.0),
        generic_params: Vec::new(),
        params,
        ret,
        generic_bounds: HashMap::new().into(),
        self_receiver: receiver,
        is_unsafe: false,
    }
}

pub(super) fn self_ty(id: crate::products::ProductDefId, index: u32) -> Type {
    Type::Generic(GenericParamId {
        owner: product_def_id(id),
        index,
    })
}

pub(super) fn trait_def(
    id: crate::products::ProductDefId,
    generic_params: Vec<&str>,
    associated_types: Vec<AssocTypeId>,
    signatures: Vec<HirFunctionSig>,
) -> HirTrait {
    HirTrait {
        target: None,
        predicates: Vec::new(),
        id: product_def_id(id),
        name: format!("T{}", id.local_id.0),
        generic_params: GenericParamDecl::type_params(
            product_def_id(id),
            generic_params.into_iter(),
        ),
        associated_types: associated_types
            .into_iter()
            .map(|id| HirAssociatedTypeDecl {
                id,
                name: format!("A{}", id.0),
                kind: crate::type_services::kind::Kind::Type,
            })
            .collect(),
        methods: HashMap::new(),
        signatures: signatures
            .into_iter()
            .map(|signature| (signature.name.clone(), signature))
            .collect(),
    }
}

pub(super) fn valid_products() -> CompilerProducts {
    let sized = product_id(1);
    let drop_trait = product_id(2);
    let drop_method = product_id(3);
    let index_trait = product_id(4);
    let index_method = product_id(5);
    let try_trait = product_id(6);
    let branch_method = product_id(7);
    let residual_trait = product_id(8);
    let residual_method = product_id(9);
    let control_flow = product_id(10);
    let index_mut_trait = product_id(11);
    let index_mut_method = product_id(12);

    let index_key = Type::Generic(GenericParamId {
        owner: product_def_id(index_trait),
        index: 0,
    });
    let index_self = self_ty(index_trait, 1);
    let index_output = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Projection {
            ty: Box::new(index_self.clone()),
            trait_id: product_def_id(index_trait),
            assoc_type: AssociatedTypeKey {
                owner: product_def_id(index_trait),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![index_key.clone()],
        }),
    };
    let index_mut_key = Type::Generic(GenericParamId {
        owner: product_def_id(index_mut_trait),
        index: 0,
    });
    let index_mut_self = self_ty(index_mut_trait, 1);
    let index_mut_output = Type::Reference {
        mutable: true,
        inner: Box::new(Type::Projection {
            ty: Box::new(index_mut_self.clone()),
            trait_id: product_def_id(index_mut_trait),
            assoc_type: AssociatedTypeKey {
                owner: product_def_id(index_mut_trait),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![index_mut_key.clone()],
        }),
    };
    let try_projection = |assoc_type_id| Type::Projection {
        ty: Box::new(self_ty(try_trait, 0)),
        trait_id: product_def_id(try_trait),
        assoc_type: AssociatedTypeKey {
            owner: product_def_id(try_trait),
            assoc_type_id,
        },
        trait_args: Vec::new(),
    };

    let mut interface = ProductInterface::default();
    interface.traits.insert(
        sized,
        crate::products::ProductTraitInterface::from(&trait_def(
            sized,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )),
    );
    interface.traits.insert(
        drop_trait,
        crate::products::ProductTraitInterface::from(&trait_def(
            drop_trait,
            Vec::new(),
            Vec::new(),
            vec![signature(
                drop_method,
                Some(ReceiverMode::Move),
                vec![self_ty(drop_trait, 0)],
                Type::Unit,
            )],
        )),
    );
    interface.traits.insert(
        index_trait,
        crate::products::ProductTraitInterface::from(&trait_def(
            index_trait,
            vec!["Key"],
            vec![AssocTypeId(0)],
            vec![signature(
                index_method,
                Some(ReceiverMode::Shared),
                vec![
                    Type::Reference {
                        mutable: false,
                        inner: Box::new(index_self),
                    },
                    index_key,
                ],
                index_output,
            )],
        )),
    );
    interface.traits.insert(
        index_mut_trait,
        crate::products::ProductTraitInterface::from(&trait_def(
            index_mut_trait,
            vec!["Key"],
            vec![AssocTypeId(0)],
            vec![signature(
                index_mut_method,
                Some(ReceiverMode::Mut),
                vec![
                    Type::Reference {
                        mutable: true,
                        inner: Box::new(index_mut_self),
                    },
                    index_mut_key,
                ],
                index_mut_output,
            )],
        )),
    );
    interface.traits.insert(
        try_trait,
        crate::products::ProductTraitInterface::from(&trait_def(
            try_trait,
            Vec::new(),
            vec![AssocTypeId(0), AssocTypeId(1)],
            vec![signature(
                branch_method,
                Some(ReceiverMode::Move),
                vec![self_ty(try_trait, 0)],
                Type::Enum {
                    id: product_def_id(control_flow),
                    args: vec![
                        try_projection(AssocTypeId(1)),
                        try_projection(AssocTypeId(0)),
                    ],
                },
            )],
        )),
    );
    interface.traits.insert(
        residual_trait,
        crate::products::ProductTraitInterface::from(&trait_def(
            residual_trait,
            vec!["R"],
            Vec::new(),
            vec![signature(
                residual_method,
                None,
                vec![self_ty(residual_trait, 0)],
                self_ty(residual_trait, 1),
            )],
        )),
    );
    interface.enums.insert(
        control_flow,
        crate::products::ProductEnumInterface::from(&HirEnum {
            id: product_def_id(control_flow),
            name: "ControlFlow".to_string(),
            generic_params: GenericParamDecl::type_params(
                product_def_id(control_flow),
                ["B", "C"].into_iter(),
            ),
            variants: vec![
                HirVariant {
                    id: VariantId(0),
                    name: "Break".to_string(),
                    fields: HirVariantFields::Positional(vec![self_ty(control_flow, 0)]),
                },
                HirVariant {
                    id: VariantId(1),
                    name: "Continue".to_string(),
                    fields: HirVariantFields::Positional(vec![self_ty(control_flow, 1)]),
                },
            ],
        }),
    );
    interface.language_items = LanguageItems {
        fold: None,
        sized: Some(SizedLanguageItems { trait_id: sized }),
        drop: Some(DropLanguageItems {
            trait_id: drop_trait,
            method_id: drop_method,
        }),
        index: Some(IndexLanguageItems {
            trait_id: index_trait,
            output_id: AssocTypeId(0),
            method_id: index_method,
        }),
        index_mut: Some(IndexMutLanguageItems {
            trait_id: index_mut_trait,
            output_id: AssocTypeId(0),
            method_id: index_mut_method,
        }),
        fn_once: None,
        fn_mut: None,
        fn_trait: None,
        send: None,
        sync: None,
        try_protocol: Some(TryLanguageItems {
            try_trait_id: try_trait,
            output_id: AssocTypeId(0),
            residual_id: AssocTypeId(1),
            branch_method_id: branch_method,
            from_residual_trait_id: residual_trait,
            from_residual_method_id: residual_method,
            control_flow_enum_id: control_flow,
            break_variant_id: VariantId(0),
            continue_variant_id: VariantId(1),
        }),
        range: None,
    };

    CompilerProducts {
        crate_identity: ProductCrateIdentity::local("protocol".to_string()),
        identity_table: ProductIdentityTable {
            local_crate: Some(ProductCrateId(0)),
            ..ProductIdentityTable::default()
        },
        interface,
        bodies: ProductBodies::default(),
        link: ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    }
}

pub(super) fn assert_loader_error_in_both_modes(products: &CompilerProducts, expected: &str) {
    let case_id = ARTIFACT_CASE_ID.fetch_add(1, Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "rock_language_item_loader_{}_{}",
        std::process::id(),
        case_id
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("temporary artifact directory");
    let artifact_path = base.join("protocol.rkca");
    products
        .write_artifact_to_path(&artifact_path)
        .expect("serialized malformed product artifact");

    for alias_mode in [false, true] {
        let mut crate_context = crate::crate_system::CrateContext::new();
        let mut type_context = crate::type_context::TypeContext::new();
        let error = if alias_mode {
            crate_context.load_product_artifact_from_path_as_with_type_context(
                "protocol",
                artifact_path.clone(),
                &mut type_context,
            )
        } else {
            crate_context.load_product_artifact_from_path_with_type_context(
                artifact_path.clone(),
                &mut type_context,
            )
        }
        .expect_err("malformed artifact must fail to load");

        assert_eq!(error, expected);
        assert!(crate_context.product_crate_ids.is_empty());
        assert!(crate_context.extern_crate("protocol").is_none());
        assert_eq!(type_context.len(), 0);
    }

    let _ = std::fs::remove_dir_all(base);
}
