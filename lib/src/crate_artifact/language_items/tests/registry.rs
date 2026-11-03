use super::*;

#[test]
fn product_language_item_validation_accepts_all_complete_protocols() {
    validate_product_language_items(&valid_products()).expect("complete registry is valid");
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
