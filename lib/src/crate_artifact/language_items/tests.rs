use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::hir::{
    HirAssociatedTypeDecl, HirEnum, HirFunctionSig, HirImplOwner, HirImplReceiverPattern, HirTrait,
    HirVariant, HirVariantFields,
};
use crate::ids::{AssocTypeId, CrateId, VariantId};
use crate::language_items::{
    DropLanguageItems, IndexLanguageItems, IndexMutLanguageItems, LanguageItems,
    SizedLanguageItems, TryLanguageItems,
};
use crate::products::{
    CompilerProducts, ProductAssociatedTypeInterface, ProductBodies, ProductCrateId,
    ProductCrateIdentity, ProductFunctionInterface, ProductIdentityTable, ProductImplInterface,
    ProductInterface, ProductLinkData, ProductLinkRecord, ProductLocalDefId,
    ProductSourceFingerprint, ProductStructInterface,
};
use crate::types::ReceiverMode;
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, TraitBound, Type};

use super::{product_def_id, validate_product_index_mut_impls, validate_product_language_items};

static ARTIFACT_CASE_ID: AtomicU64 = AtomicU64::new(0);

fn product_id(local_id: u32) -> crate::products::ProductDefId {
    crate::products::ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(local_id),
    }
}

fn signature(
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

fn self_ty(id: crate::products::ProductDefId, index: u32) -> Type {
    Type::Generic(GenericParamId {
        owner: product_def_id(id),
        index,
    })
}

fn trait_def(
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

fn valid_products() -> CompilerProducts {
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

fn product_index_impl_with_outputs(
    id: u32,
    trait_id: crate::products::ProductDefId,
    associated_types: Vec<ProductAssociatedTypeInterface>,
) -> ProductImplInterface {
    let impl_id = product_id(id);
    ProductImplInterface {
        id: product_def_id(impl_id),
        owner: HirImplOwner::Named("Cell".to_string()),
        type_name: "Cell".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: product_def_id(product_id(20)),
            args: Vec::new(),
        }),
        trait_name: Some("RenamedIndex".to_string()),
        trait_id: Some(product_def_id(trait_id)),
        trait_generics: Vec::new(),
        trait_arg_types: vec![Type::I64],
        associated_types,
        bounds: HashMap::new().into(),
        methods: BTreeMap::new(),
    }
}

fn product_index_impl_with_bounds(
    id: u32,
    trait_id: crate::products::ProductDefId,
    bounds: Vec<TraitBound>,
) -> ProductImplInterface {
    let impl_id = product_id(id);
    let impl_def_id = product_def_id(impl_id);
    let generic = GenericParamId {
        owner: impl_def_id,
        index: 0,
    };
    ProductImplInterface {
        id: impl_def_id,
        owner: HirImplOwner::Named("Cell".to_string()),
        type_name: "Cell".to_string(),
        type_generics: vec![GenericParamDecl::type_param(generic, "T")],
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: product_def_id(product_id(20)),
            args: vec![Type::Generic(generic)],
        }),
        trait_name: Some("RenamedIndex".to_string()),
        trait_id: Some(product_def_id(trait_id)),
        trait_generics: Vec::new(),
        trait_arg_types: vec![Type::I64],
        associated_types: vec![ProductAssociatedTypeInterface {
            id: AssocTypeId(0),
            name: "Value".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::Generic(generic),
        }],
        bounds: HashMap::from([(generic, bounds)]).into(),
        methods: BTreeMap::new(),
    }
}

fn add_product_index_impl(
    products: &mut CompilerProducts,
    id: u32,
    trait_id: crate::products::ProductDefId,
    associated_types: Vec<ProductAssociatedTypeInterface>,
) {
    let is_mut = trait_id == product_id(11);
    let trait_method_id = if is_mut {
        product_id(12)
    } else {
        product_id(5)
    };
    let impl_method_id = product_id(id + 100);
    let impl_id = product_id(id);
    let mut imp = product_index_impl_with_outputs(id, trait_id, associated_types);
    imp.methods.insert(
        if is_mut {
            "write_at".to_string()
        } else {
            "read_at".to_string()
        },
        ProductFunctionInterface {
            id: product_def_id(impl_method_id),
            name: if is_mut {
                "write_at".to_string()
            } else {
                "read_at".to_string()
            },
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![
                Type::Reference {
                    mutable: is_mut,
                    inner: Box::new(Type::Struct {
                        id: product_def_id(product_id(20)),
                        args: Vec::new(),
                    }),
                },
                Type::I64,
            ],
            ret_type: Type::Reference {
                mutable: is_mut,
                inner: Box::new(Type::I64),
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(if is_mut {
                ReceiverMode::Mut
            } else {
                ReceiverMode::Shared
            }),
            is_unsafe: false,
            constant_arithmetic: None,
        },
    );
    products.interface.impls.insert(impl_id, imp);
    products
        .interface
        .effective_trait_methods
        .insert((impl_id, trait_method_id), impl_method_id);
    products.link.records.insert(
        impl_method_id,
        ProductLinkRecord {
            backend_symbol: format!("__rock_fn_index_fixture_{id}"),
        },
    );
}

fn assert_loader_error_in_both_modes(products: &CompilerProducts, expected: &str) {
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

fn products_with_reference_index_pair(
    index_receiver_is_mutable: bool,
    index_mut_receiver_is_mutable: bool,
) -> CompilerProducts {
    let mut products = valid_products();
    let index_trait_id = products
        .interface
        .language_items
        .index
        .as_ref()
        .expect("Index language item")
        .trait_id;
    let index_mut_trait_id = products
        .interface
        .language_items
        .index_mut
        .as_ref()
        .expect("IndexMut language item")
        .trait_id;
    let cell_id = product_id(20);
    products.interface.structs.insert(
        cell_id,
        ProductStructInterface {
            id: product_def_id(cell_id),
            name: "Cell".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        },
    );
    add_product_index_impl(
        &mut products,
        30,
        index_trait_id,
        vec![ProductAssociatedTypeInterface {
            id: AssocTypeId(0),
            name: "Value".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::I64,
        }],
    );
    add_product_index_impl(
        &mut products,
        31,
        index_mut_trait_id,
        vec![ProductAssociatedTypeInterface {
            id: AssocTypeId(0),
            name: "Value".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::I64,
        }],
    );
    let cell_type = Type::Struct {
        id: product_def_id(cell_id),
        args: Vec::new(),
    };
    products
        .interface
        .impls
        .get_mut(&product_id(30))
        .expect("Index fixture")
        .receiver_pattern = HirImplReceiverPattern::Exact(Type::Reference {
        mutable: index_receiver_is_mutable,
        inner: Box::new(cell_type.clone()),
    });
    products
        .interface
        .impls
        .get_mut(&product_id(31))
        .expect("IndexMut fixture")
        .receiver_pattern = HirImplReceiverPattern::Exact(Type::Reference {
        mutable: index_mut_receiver_is_mutable,
        inner: Box::new(cell_type),
    });
    products
}

#[test]
fn product_language_item_validation_accepts_all_complete_protocols() {
    validate_product_language_items(&valid_products()).expect("complete registry is valid");
}

#[test]
fn product_artifact_rejects_unpaired_index_mut_impl() {
    let mut products = valid_products();
    let index_mut_trait = products
        .interface
        .language_items
        .index_mut
        .as_ref()
        .expect("IndexMut language item")
        .trait_id;
    let index_mut_method = products
        .interface
        .language_items
        .index_mut
        .as_ref()
        .expect("IndexMut language item")
        .method_id;
    let index_mut_impl_method = product_id(22);
    let cell_id = product_id(20);
    products.interface.structs.insert(
        cell_id,
        ProductStructInterface {
            id: product_def_id(cell_id),
            name: "Cell".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        },
    );
    products.interface.impls.insert(
        product_id(21),
        ProductImplInterface {
            id: product_def_id(product_id(21)),
            owner: HirImplOwner::Named("Cell".to_string()),
            type_name: "Cell".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: product_def_id(cell_id),
                args: Vec::new(),
            }),
            trait_name: Some("WriteAt".to_string()),
            trait_id: Some(product_def_id(index_mut_trait)),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::I64],
            associated_types: vec![ProductAssociatedTypeInterface {
                id: AssocTypeId(0),
                name: "WriteValue".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
            bounds: HashMap::new().into(),
            methods: BTreeMap::from([(
                "write_at".to_string(),
                ProductFunctionInterface {
                    id: product_def_id(index_mut_impl_method),
                    name: "write_at".to_string(),
                    generic_params: Vec::new(),
                    generic_bounds: HashMap::new().into(),
                    params: vec![
                        Type::Reference {
                            mutable: true,
                            inner: Box::new(Type::Struct {
                                id: product_def_id(cell_id),
                                args: Vec::new(),
                            }),
                        },
                        Type::I64,
                    ],
                    ret_type: Type::Reference {
                        mutable: true,
                        inner: Box::new(Type::I64),
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Mut),
                    is_unsafe: false,
                    constant_arithmetic: None,
                },
            )]),
        },
    );
    products
        .interface
        .effective_trait_methods
        .insert((product_id(21), index_mut_method), index_mut_impl_method);
    products.link.records.insert(
        index_mut_impl_method,
        ProductLinkRecord {
            backend_symbol: "__rock_fn_index_mut_fixture".to_string(),
        },
    );

    assert_loader_error_in_both_modes(
            &products,
            "Product artifact IndexMut implementation for Cell with key I64 requires a matching Index implementation",
        );
}

#[test]
fn product_artifact_accepts_shared_mutable_reference_index_pair() {
    let products = products_with_reference_index_pair(false, true);
    validate_product_language_items(&products).expect("shared/mutable pair is valid");
}

#[test]
fn product_artifact_rejects_reverse_reference_index_pair() {
    let products = products_with_reference_index_pair(true, false);
    assert_loader_error_in_both_modes(
            &products,
            "Product artifact IndexMut implementation for Cell with key I64 requires a matching Index implementation",
        );
}

#[test]
fn product_artifact_rejects_same_shared_reference_index_pair() {
    let products = products_with_reference_index_pair(false, false);
    assert_loader_error_in_both_modes(
            &products,
            "Product artifact IndexMut implementation for Cell with key I64 requires a matching Index implementation",
        );
}

#[test]
fn product_artifact_rejects_missing_marked_index_mut_output() {
    let mut products = valid_products();
    let (index_trait_id, index_output_id) = {
        let index = products.interface.language_items.index.as_ref().unwrap();
        (index.trait_id, index.output_id)
    };
    let index_mut_trait_id = products
        .interface
        .language_items
        .index_mut
        .as_ref()
        .unwrap()
        .trait_id;
    products.interface.structs.insert(
        product_id(20),
        ProductStructInterface {
            id: product_def_id(product_id(20)),
            name: "Cell".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        },
    );
    add_product_index_impl(
        &mut products,
        30,
        index_trait_id,
        vec![ProductAssociatedTypeInterface {
            id: index_output_id,
            name: "ReadValue".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::I64,
        }],
    );
    add_product_index_impl(&mut products, 31, index_mut_trait_id, Vec::new());

    assert_loader_error_in_both_modes(
            &products,
            "Product artifact IndexMut implementation for Cell with key I64 is missing marked associated output AssocTypeId(0)",
        );
}

#[test]
fn product_artifact_rejects_duplicate_marked_index_mut_output() {
    let mut products = valid_products();
    let (index_trait_id, index_output_id) = {
        let index = products.interface.language_items.index.as_ref().unwrap();
        (index.trait_id, index.output_id)
    };
    let (index_mut_trait_id, index_mut_output_id) = products
        .interface
        .language_items
        .index_mut
        .as_ref()
        .map(|index_mut| (index_mut.trait_id, index_mut.output_id))
        .unwrap();
    products.interface.structs.insert(
        product_id(20),
        ProductStructInterface {
            id: product_def_id(product_id(20)),
            name: "Cell".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        },
    );
    add_product_index_impl(
        &mut products,
        32,
        index_trait_id,
        vec![ProductAssociatedTypeInterface {
            id: index_output_id,
            name: "ReadValue".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::I64,
        }],
    );
    add_product_index_impl(
        &mut products,
        33,
        index_mut_trait_id,
        vec![
            ProductAssociatedTypeInterface {
                id: index_mut_output_id,
                name: "WriteValue".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            },
            ProductAssociatedTypeInterface {
                id: index_mut_output_id,
                name: "WriteValueAgain".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            },
        ],
    );

    assert_loader_error_in_both_modes(
            &products,
            "Product artifact IndexMut implementation for Cell with key I64 has duplicate marked associated output AssocTypeId(0)",
        );
}

#[test]
fn product_index_pair_accepts_reordered_bounds() {
    let mut products = valid_products();
    let index = products.interface.language_items.index.as_ref().unwrap();
    let index_mut = products
        .interface
        .language_items
        .index_mut
        .as_ref()
        .unwrap();
    let first = TraitBound {
        trait_id: product_def_id(product_id(1)),
        type_args: Vec::new(),
    };
    let second = TraitBound {
        trait_id: product_def_id(product_id(2)),
        type_args: Vec::new(),
    };
    products.interface.impls.insert(
        product_id(34),
        product_index_impl_with_bounds(34, index.trait_id, vec![first.clone(), second.clone()]),
    );
    products.interface.impls.insert(
        product_id(35),
        product_index_impl_with_bounds(35, index_mut.trait_id, vec![second, first]),
    );

    validate_product_index_mut_impls(&products, &products.interface.language_items)
        .expect("reordered conjunctive bounds should pair");
}

#[test]
fn product_language_item_preflight_ignores_unrelated_interface_rows() {
    let mut products = valid_products();
    let row_id = product_id(99);
    products.interface.functions.insert(
        row_id,
        crate::products::ProductFunctionInterface {
            id: product_def_id(product_id(98)),
            name: "unrelated".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::Unit,
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
            constant_arithmetic: None,
        },
    );

    assert_loader_error_in_both_modes(
        &products,
        "Product artifact interface function row 99 has embedded ID 98",
    );
}

#[test]
fn product_language_item_loader_rejects_malformed_registry_matrix() {
    macro_rules! loader_case {
        ($mutate:expr, $expected:expr) => {{
            let mut products = valid_products();
            ($mutate)(&mut products);
            assert_loader_error_in_both_modes(&products, $expected);
        }};
    }

    loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.index.as_mut().unwrap().trait_id = product_id(99);
            },
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(99) }: index.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(99) } is no declaration"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.index.as_mut().unwrap().method_id = product_id(99);
            },
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.method DefId { crate_id: CrateId(0), local: LocalDefId(99) } is not declared by any trait"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .language_items
                    .index_mut
                    .as_mut()
                    .unwrap()
                    .method_id = product_id(99);
            },
            "Product artifact language item index_mut.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(11) }: index_mut.method DefId { crate_id: CrateId(0), local: LocalDefId(99) } is not declared by any trait"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .language_items
                    .index_mut
                    .as_mut()
                    .unwrap()
                    .output_id = AssocTypeId(1);
            },
            "Product artifact language item index_mut.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(11) }: index_mut.output AssocTypeId(1) is declared by trait DefId { crate_id: CrateId(0), local: LocalDefId(6) }, not DefId { crate_id: CrateId(0), local: LocalDefId(11) }"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                let method = products
                    .interface
                    .traits
                    .get_mut(&product_id(11))
                    .unwrap()
                    .signatures
                    .values_mut()
                    .next()
                    .unwrap();
                method.self_receiver = Some(ReceiverMode::Shared);
            },
            "Product artifact language item index_mut.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(11) }: index_mut.method must use a mutable receiver"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                let method = products
                    .interface
                    .traits
                    .get_mut(&product_id(11))
                    .unwrap()
                    .signatures
                    .values_mut()
                    .next()
                    .unwrap();
                let Type::Reference { mutable, .. } = &mut method.ret else {
                    panic!("valid IndexMut return must be a reference");
                };
                *mutable = false;
            },
            "Product artifact language item index_mut.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(11) }: index_mut.method must return a mutable reference to index_mut.output on Self"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.index.as_mut().unwrap().output_id = AssocTypeId(9);
            },
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.output AssocTypeId(9) is not declared by trait DefId { crate_id: CrateId(0), local: LocalDefId(4) }"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .traits
                    .get_mut(&product_id(4))
                    .unwrap()
                    .signatures
                    .values_mut()
                    .next()
                    .unwrap()
                    .ret = Type::Unit;
            },
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.method must return a shared reference to index.output on Self"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.try_protocol.as_mut().unwrap().try_trait_id =
                    product_id(10);
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(10) }: try.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(10) } is an enum"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .language_items
                    .try_protocol
                    .as_mut()
                    .unwrap()
                    .branch_method_id = product_id(99);
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: try.branch DefId { crate_id: CrateId(0), local: LocalDefId(99) } is not declared by any trait"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .traits
                    .get_mut(&product_id(6))
                    .unwrap()
                    .signatures
                    .values_mut()
                    .next()
                    .unwrap()
                    .ret = Type::Unit;
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: try.branch must return control_flow<Residual, Output>"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.try_protocol.as_mut().unwrap().residual_id =
                    AssocTypeId(9);
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: try.residual AssocTypeId(9) is not declared by trait DefId { crate_id: CrateId(0), local: LocalDefId(6) }"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .language_items
                    .try_protocol
                    .as_mut()
                    .unwrap()
                    .from_residual_method_id = product_id(99);
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: from_residual.method DefId { crate_id: CrateId(0), local: LocalDefId(99) } is not declared by any trait"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .traits
                    .get_mut(&product_id(8))
                    .unwrap()
                    .signatures
                    .values_mut()
                    .next()
                    .unwrap()
                    .ret = Type::Unit;
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: from_residual.method must return Self"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .language_items
                    .try_protocol
                    .as_mut()
                    .unwrap()
                    .break_variant_id = VariantId(9);
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.break VariantId(9) is not declared by enum DefId { crate_id: CrateId(0), local: LocalDefId(10) }"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .enums
                    .get_mut(&product_id(10))
                    .unwrap()
                    .variants[0]
                    .fields = HirVariantFields::Unit;
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.break must not be unit"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .enums
                    .get_mut(&product_id(10))
                    .unwrap()
                    .generic_params
                    .push(GenericParamDecl::type_param(
                        GenericParamId {
                            owner: product_def_id(product_id(10)),
                            index: 2,
                        },
                        "Extra",
                    ));
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.enum must declare exactly 2 generic parameters; found 3"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .enums
                    .get_mut(&product_id(10))
                    .unwrap()
                    .variants
                    .push(crate::products::ProductEnumVariantInterface {
                        id: VariantId(2),
                        name: "Extra".to_string(),
                        fields: HirVariantFields::Unit,
                    });
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.enum must declare exactly 2 variants; found 3"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.sized.as_mut().unwrap().trait_id =
                    crate::products::ProductDefId {
                        crate_id: ProductCrateId(1),
                        local_id: ProductLocalDefId(1),
                    };
            },
            "Product artifact language item sized.trait uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(1) }; expected product crate 0"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.drop.as_mut().unwrap().method_id =
                    crate::products::ProductDefId {
                        crate_id: ProductCrateId(1),
                        local_id: ProductLocalDefId(3),
                    };
            },
            "Product artifact language item drop.method uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(3) }; expected product crate 0"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.index.as_mut().unwrap().method_id =
                    crate::products::ProductDefId {
                        crate_id: ProductCrateId(1),
                        local_id: ProductLocalDefId(5),
                    };
            },
            "Product artifact language item index.method uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(5) }; expected product crate 0"
        );
    loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .language_items
                    .try_protocol
                    .as_mut()
                    .unwrap()
                    .branch_method_id = crate::products::ProductDefId {
                    crate_id: ProductCrateId(1),
                    local_id: ProductLocalDefId(7),
                };
            },
            "Product artifact language item try.branch uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(7) }; expected product crate 0"
        );
}

#[test]
fn product_language_items_remap_every_def_id_at_nonzero_runtime_crate_id() {
    let mut first = valid_products();
    first.crate_identity = ProductCrateIdentity::local("first".to_string());
    let mut second = valid_products();
    second.crate_identity = ProductCrateIdentity::local("second".to_string());
    let base = std::env::temp_dir().join(format!(
        "rock_language_item_remap_{}_{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("temporary artifact directory");
    let first_path = base.join("first.rkca");
    let second_path = base.join("second.rkca");
    first
        .write_artifact_to_path(&first_path)
        .expect("first product artifact");
    second
        .write_artifact_to_path(&second_path)
        .expect("second product artifact");
    let mut context = crate::crate_system::CrateContext::new();
    context
        .load_product_artifact_from_path(first_path)
        .expect("first product artifact should load");
    context
        .load_product_artifact_from_path(second_path)
        .expect("second product artifact should load");
    let items = context
        .extern_crate("second")
        .expect("second product extern record")
        .metadata()
        .language_items()
        .clone();
    let runtime_crate = items.sized.as_ref().unwrap().trait_id.crate_id;
    assert_ne!(runtime_crate, CrateId(0));
    let drop = items.drop.as_ref().unwrap();
    assert_eq!(drop.trait_id.crate_id, runtime_crate);
    assert_eq!(drop.method_id.crate_id, runtime_crate);
    let index = items.index.as_ref().unwrap();
    assert_eq!(index.trait_id.crate_id, runtime_crate);
    assert_eq!(index.method_id.crate_id, runtime_crate);
    assert_eq!(index.output_id, AssocTypeId(0));
    let index_mut = items.index_mut.as_ref().unwrap();
    assert_eq!(index_mut.trait_id.crate_id, runtime_crate);
    assert_eq!(index_mut.method_id.crate_id, runtime_crate);
    assert_eq!(index_mut.output_id, AssocTypeId(0));
    let try_protocol = items.try_protocol.as_ref().unwrap();
    assert_eq!(try_protocol.try_trait_id.crate_id, runtime_crate);
    assert_eq!(try_protocol.branch_method_id.crate_id, runtime_crate);
    assert_eq!(try_protocol.from_residual_trait_id.crate_id, runtime_crate);
    assert_eq!(try_protocol.from_residual_method_id.crate_id, runtime_crate);
    assert_eq!(try_protocol.control_flow_enum_id.crate_id, runtime_crate);
    assert_eq!(try_protocol.output_id, AssocTypeId(0));
    assert_eq!(try_protocol.residual_id, AssocTypeId(1));
    assert_eq!(try_protocol.break_variant_id, VariantId(0));
    assert_eq!(try_protocol.continue_variant_id, VariantId(1));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn product_language_item_validation_rejects_nonlocal_ids_in_every_multi_id_bundle() {
    let mut products = valid_products();
    products
        .interface
        .language_items
        .drop
        .as_mut()
        .unwrap()
        .method_id = crate::products::ProductDefId {
        crate_id: ProductCrateId(1),
        local_id: ProductLocalDefId(3),
    };
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item drop.method uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(3) }; expected product crate 0"
        );

    let mut products = valid_products();
    products
        .interface
        .language_items
        .index
        .as_mut()
        .unwrap()
        .method_id = crate::products::ProductDefId {
        crate_id: ProductCrateId(1),
        local_id: ProductLocalDefId(5),
    };
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index.method uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(5) }; expected product crate 0"
        );

    let mut products = valid_products();
    products
        .interface
        .language_items
        .index_mut
        .as_mut()
        .unwrap()
        .method_id = crate::products::ProductDefId {
        crate_id: ProductCrateId(1),
        local_id: ProductLocalDefId(12),
    };
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index_mut.method uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(12) }; expected product crate 0"
        );

    let mut products = valid_products();
    products
        .interface
        .language_items
        .try_protocol
        .as_mut()
        .unwrap()
        .branch_method_id = crate::products::ProductDefId {
        crate_id: ProductCrateId(1),
        local_id: ProductLocalDefId(7),
    };
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.branch uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(7) }; expected product crate 0"
        );
}

#[test]
fn product_language_item_validation_rejects_missing_and_wrong_kind_roots() {
    let mut products = valid_products();
    products
        .interface
        .language_items
        .sized
        .as_mut()
        .unwrap()
        .trait_id = product_id(99);
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item sized.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(99) }: sized.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(99) } is no declaration"
        );

    let mut products = valid_products();
    let function_id = product_id(99);
    products.interface.functions.insert(
        function_id,
        crate::products::ProductFunctionInterface {
            id: product_def_id(function_id),
            name: "wrong".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::Unit,
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
            constant_arithmetic: None,
        },
    );
    products
        .interface
        .language_items
        .sized
        .as_mut()
        .unwrap()
        .trait_id = function_id;
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item sized.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(99) }: sized.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(99) } is a function"
        );

    let mut products = valid_products();
    products
        .interface
        .language_items
        .sized
        .as_mut()
        .unwrap()
        .trait_id = product_id(10);
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item sized.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(10) }: sized.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(10) } is an enum"
        );

    let mut products = valid_products();
    products
        .interface
        .language_items
        .try_protocol
        .as_mut()
        .unwrap()
        .control_flow_enum_id = product_id(6);
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: try.branch must return the marked control_flow enum; control_flow.enum must identify an enum; DefId { crate_id: CrateId(0), local: LocalDefId(6) } is a trait"
        );
}

#[test]
fn product_language_item_validation_rejects_wrong_member_assoc_and_variant_owners() {
    let mut products = valid_products();
    let other_trait = product_id(13);
    let other_method = product_id(14);
    products.interface.traits.insert(
        other_trait,
        crate::products::ProductTraitInterface::from(&trait_def(
            other_trait,
            Vec::new(),
            Vec::new(),
            vec![signature(other_method, None, Vec::new(), Type::Unit)],
        )),
    );
    products
        .interface
        .language_items
        .drop
        .as_mut()
        .unwrap()
        .method_id = other_method;
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(2) }: drop.method DefId { crate_id: CrateId(0), local: LocalDefId(14) } is owned by trait DefId { crate_id: CrateId(0), local: LocalDefId(13) }, not DefId { crate_id: CrateId(0), local: LocalDefId(2) }"
        );

    let mut products = valid_products();
    products
        .interface
        .language_items
        .index
        .as_mut()
        .unwrap()
        .output_id = AssocTypeId(9);
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.output AssocTypeId(9) is not declared by trait DefId { crate_id: CrateId(0), local: LocalDefId(4) }"
        );

    let mut products = valid_products();
    products
        .interface
        .language_items
        .try_protocol
        .as_mut()
        .unwrap()
        .break_variant_id = VariantId(9);
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.break VariantId(9) is not declared by enum DefId { crate_id: CrateId(0), local: LocalDefId(10) }"
        );
}

#[test]
fn product_language_item_validation_reuses_shared_shape_rules() {
    let mut products = valid_products();
    products
        .interface
        .traits
        .get_mut(&product_id(2))
        .unwrap()
        .signatures
        .values_mut()
        .next()
        .unwrap()
        .ret = Type::I64;
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(2) }: drop.method must return ()"
        );

    let mut products = valid_products();
    products
        .interface
        .traits
        .get_mut(&product_id(4))
        .unwrap()
        .signatures
        .values_mut()
        .next()
        .unwrap()
        .ret = Type::Unit;
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.method must return a shared reference to index.output on Self"
        );

    let mut products = valid_products();
    products
        .interface
        .traits
        .get_mut(&product_id(6))
        .unwrap()
        .signatures
        .values_mut()
        .next()
        .unwrap()
        .ret = Type::Unit;
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: try.branch must return control_flow<Residual, Output>"
        );

    let mut products = valid_products();
    products
        .interface
        .traits
        .get_mut(&product_id(8))
        .unwrap()
        .signatures
        .values_mut()
        .next()
        .unwrap()
        .ret = Type::Unit;
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: from_residual.method must return Self"
        );

    let mut products = valid_products();
    products
        .interface
        .enums
        .get_mut(&product_id(10))
        .unwrap()
        .variants[0]
        .fields = HirVariantFields::Unit;
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.break must not be unit"
        );
}

#[test]
fn product_language_item_validation_rejects_index_and_control_flow_shape_matrix() {
    let mut products = valid_products();
    products
        .interface
        .traits
        .get_mut(&product_id(4))
        .unwrap()
        .generic_params
        .push(GenericParamDecl::type_param(
            GenericParamId {
                owner: product_def_id(product_id(4)),
                index: 1,
            },
            "Extra",
        ));
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.trait must declare exactly 1 generic parameters; found 2"
        );

    let mut products = valid_products();
    let method = products
        .interface
        .traits
        .get_mut(&product_id(4))
        .unwrap()
        .signatures
        .values_mut()
        .next()
        .unwrap();
    method.self_receiver = Some(ReceiverMode::Move);
    method.params[1] = Type::Unit;
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.method must use a shared receiver; index.method explicit parameter 0 has the wrong type"
        );

    let mut products = valid_products();
    let method = products
        .interface
        .traits
        .get_mut(&product_id(4))
        .unwrap()
        .signatures
        .values_mut()
        .next()
        .unwrap();
    let Type::Reference { inner, .. } = &mut method.ret else {
        panic!("valid Index return must be a reference");
    };
    let Type::Projection { trait_id, .. } = inner.as_mut() else {
        panic!("valid Index return must be a projection");
    };
    *trait_id = product_def_id(product_id(2));
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.method return projection must use index.trait"
        );

    let mut products = valid_products();
    products
        .interface
        .enums
        .get_mut(&product_id(10))
        .unwrap()
        .generic_params
        .push(GenericParamDecl::type_param(
            GenericParamId {
                owner: product_def_id(product_id(10)),
                index: 2,
            },
            "Extra",
        ));
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.enum must declare exactly 2 generic parameters; found 3"
        );

    let mut products = valid_products();
    products
        .interface
        .enums
        .get_mut(&product_id(10))
        .unwrap()
        .variants
        .push(crate::products::ProductEnumVariantInterface {
            id: VariantId(2),
            name: "Extra".to_string(),
            fields: HirVariantFields::Unit,
        });
    assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.enum must declare exactly 2 variants; found 3"
        );
}
