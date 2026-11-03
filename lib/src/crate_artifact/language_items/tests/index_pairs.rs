use super::*;

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
