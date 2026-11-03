use super::*;

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
