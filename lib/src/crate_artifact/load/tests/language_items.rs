use super::*;

#[test]
fn load_product_artifact_rejects_invalid_drop_trait_language_item_id() {
    let (base, _cleanup) = temp_test_dir("invalid_drop_trait_language_item");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.interface.language_items.drop = Some(DropLanguageItems {
        trait_id: product_def_id(0, 99),
        method_id: product_def_id(0, 100),
    });
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("invalid drop trait language items should be rejected");

    assert_eq!(
            err,
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(99) }: drop.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(99) } is no declaration"
        );
}

#[test]
fn load_product_artifact_rejects_nonlocal_sized_language_item_without_mutating_context() {
    let (base, _cleanup) = temp_test_dir("nonlocal_sized_language_item");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.interface.language_items.sized = Some(crate::language_items::SizedLanguageItems {
        trait_id: product_def_id(1, 7),
    });
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let mut type_context = crate::type_context::TypeContext::new();
    let error = ctx
        .load_product_artifact_from_path_with_type_context(artifact_path, &mut type_context)
        .expect_err("non-local language items must be rejected before loading");

    assert_eq!(
            error,
            "Product artifact language item sized.trait uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(7) }; expected product crate 0"
        );
    assert!(ctx.extern_crate("dep").is_none());
    assert_eq!(type_context.len(), 0);
}

#[test]
fn load_product_artifact_rejects_mismatched_drop_method_language_item_id() {
    let (base, _cleanup) = temp_test_dir("mismatched_drop_method_language_item");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let drop_trait_id = product_def_id(0, 7);
    let other_trait_id = product_def_id(0, 9);
    let other_method_id = product_def_id(0, 10);
    let other_method_def = product_hir_def_id(other_method_id);

    insert_interface_trait(
        &mut products,
        drop_trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: product_hir_def_id(drop_trait_id),
            name: "Drop".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    products
        .identity_table
        .display_names
        .insert(drop_trait_id, "dep::Drop".to_string());
    insert_interface_trait(
        &mut products,
        other_trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: product_hir_def_id(other_trait_id),
            name: "Other".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "drop".to_string(),
                HirFunctionSig {
                    id: other_method_def,
                    name: "drop".to_string(),
                    generic_params: Vec::new(),
                    params: Vec::new(),
                    ret: Type::Unit,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        },
    );
    products
        .identity_table
        .display_names
        .insert(other_trait_id, "dep::Other".to_string());
    products.interface.language_items.drop = Some(DropLanguageItems {
        trait_id: drop_trait_id,
        method_id: other_method_id,
    });
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("drop method language items must belong to the drop trait");

    assert_eq!(
            err,
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(7) }: drop.method DefId { crate_id: CrateId(0), local: LocalDefId(10) } is owned by trait DefId { crate_id: CrateId(0), local: LocalDefId(9) }, not DefId { crate_id: CrateId(0), local: LocalDefId(7) }"
        );
}

#[test]
fn load_product_artifact_rejects_drop_method_language_item_not_declared_by_trait() {
    let (base, _cleanup) = temp_test_dir("drop_method_not_declared_by_trait");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let drop_trait_id = product_def_id(0, 7);
    insert_interface_trait(
        &mut products,
        drop_trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: product_hir_def_id(drop_trait_id),
            name: "Drop".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    products
        .identity_table
        .display_names
        .insert(drop_trait_id, "dep::Drop".to_string());
    products.interface.language_items.drop = Some(DropLanguageItems {
        trait_id: drop_trait_id,
        method_id: product_def_id(0, 8),
    });
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("drop method language item must be declared by the drop trait");

    assert_eq!(
            err,
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(7) }: drop.method DefId { crate_id: CrateId(0), local: LocalDefId(8) } is not declared by any trait"
        );
}
