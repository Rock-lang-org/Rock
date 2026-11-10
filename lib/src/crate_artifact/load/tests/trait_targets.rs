use super::*;

#[test]
fn load_product_artifact_rejects_interface_trait_method_foreign_product_crate_id() {
    let (base, _cleanup) = temp_test_dir("interface_trait_method_foreign_product_crate");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let trait_id = product_def_id(0, 7);
    let foreign_method_id = product_def_id(1, 8);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("foreign".to_string()),
    );
    let mut method = test_function(product_hir_def_id(foreign_method_id), "show");
    method.is_method = true;
    insert_interface_trait(
        &mut products,
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: product_hir_def_id(trait_id),
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("show".to_string(), method.clone())]),
            signatures: HashMap::new(),
        },
    );
    products
        .identity_table
        .display_names
        .insert(trait_id, "dep::Show".to_string());
    products
        .bodies
        .trait_default_methods
        .insert(foreign_method_id, method);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("nested trait method IDs must be owned by the artifact crate");

    assert!(
        err.contains("trait") && err.contains("method 'show'") && err.contains("non-local"),
        "unexpected error: {err}"
    );
}

fn trait_method_fixture(
    generic_params: Vec<GenericParamDecl>,
    self_receiver: Option<crate::types::ReceiverMode>,
) -> (CompilerProducts, DefId, DefId) {
    let trait_product_id = product_def_id(0, 30);
    let trait_id = product_hir_def_id(trait_product_id);
    let member_id = DefId::new(CrateId(0), LocalDefId(31));
    let mut member = test_function(member_id, "value");
    member.is_method = self_receiver.is_some();
    member.self_receiver = self_receiver;
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_trait(
        &mut products,
        trait_product_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "dep::Trait".to_string(),
            generic_params,
            associated_types: Vec::new(),
            methods: HashMap::from([("value".to_string(), member)]),
            signatures: HashMap::new(),
        },
    );
    (products, trait_id, member_id)
}

#[test]
fn product_artifact_load_rejects_receiver_trait_member_mapped_to_static_impl_method() {
    let (mut products, trait_id, member_id) =
        trait_method_fixture(Vec::new(), Some(crate::types::ReceiverMode::Move));
    let impl_product_id = product_def_id(0, 40);
    let impl_id = product_hir_def_id(impl_product_id);
    let method_id = DefId::new(CrateId(0), LocalDefId(41));
    insert_interface_impl(
        &mut products,
        impl_product_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
            trait_name: Some("Trait".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([(
                "different_source_name".to_string(),
                test_function(method_id, "different_source_name"),
            )]),
        },
    );
    products.interface.effective_trait_methods.insert(
        (impl_product_id, ProductDefId::from(member_id)),
        ProductDefId::from(method_id),
    );
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };

    let error =
        super::cross_crate_hir_from_products(&products, "dep", &remap, &CrateContext::new())
            .expect_err("malformed effective trait-member relation must fail artifact loading");

    assert!(
        error.contains("effective trait member")
            && error.contains(&format!("{member_id:?}"))
            && error.contains(&format!("{method_id:?}"))
            && error.contains("receiver/static mismatch"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_trait_method_target_with_missing_trait_argument() {
    let (products, trait_id, member_id) = trait_method_fixture(
        generic_params(DefId::new(CrateId(0), LocalDefId(30)), &["T"]),
        Some(crate::types::ReceiverMode::Move),
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            Some(crate::types::ReceiverMode::Move),
            HirMethodCallTarget::trait_method(
                trait_id,
                member_id,
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

    assert!(
        error.contains("trait argument arity"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_impl_target_with_inconsistent_trait_arguments() {
    let (mut products, trait_id, member_id) = trait_method_fixture(
        generic_params(DefId::new(CrateId(0), LocalDefId(30)), &["T"]),
        Some(crate::types::ReceiverMode::Move),
    );
    let impl_product_id = product_def_id(0, 40);
    let impl_id = product_hir_def_id(impl_product_id);
    let method_id = DefId::new(CrateId(0), LocalDefId(41));
    let mut method = test_function(method_id, "value");
    method.is_method = true;
    method.self_receiver = Some(crate::types::ReceiverMode::Move);
    insert_interface_impl(
        &mut products,
        impl_product_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
            trait_name: Some("Trait".to_string()),
            trait_id: Some(trait_id),
            trait_generics: generic_params(trait_id, &["T"]),
            trait_arg_types: vec![Type::I64],
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        },
    );
    products.interface.effective_trait_methods.insert(
        (impl_product_id, ProductDefId::from(member_id)),
        ProductDefId::from(method_id),
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            Some(crate::types::ReceiverMode::Move),
            HirMethodCallTarget::impl_method(
                impl_id,
                method_id,
                Some(crate::hir::HirSelectedTraitMember {
                    trait_id,
                    member_id,
                    trait_args: vec![Type::Bool],
                }),
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

    assert!(
        error.contains("trait arguments do not match impl"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_trait_impl_target_without_selected_trait_identity() {
    let (mut products, trait_id, member_id) =
        trait_method_fixture(Vec::new(), Some(crate::types::ReceiverMode::Move));
    let impl_product_id = product_def_id(0, 40);
    let impl_id = product_hir_def_id(impl_product_id);
    let method_id = DefId::new(CrateId(0), LocalDefId(41));
    let mut method = test_function(method_id, "value");
    method.is_method = true;
    method.self_receiver = Some(crate::types::ReceiverMode::Move);
    insert_interface_impl(
        &mut products,
        impl_product_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::I64),
            trait_name: Some("Trait".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        },
    );
    products.interface.effective_trait_methods.insert(
        (impl_product_id, ProductDefId::from(member_id)),
        ProductDefId::from(method_id),
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            Some(crate::types::ReceiverMode::Move),
            HirMethodCallTarget::impl_method(impl_id, method_id, None),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

    assert!(
        error.contains("has no selected trait identity"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_malformed_trait_method_authority_with_substitutions() {
    let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let mut target = HirMethodCallTarget::trait_method(
        DefId::new(CrateId(0), LocalDefId(999)),
        DefId::new(CrateId(0), LocalDefId(998)),
        Vec::new(),
        crate::hir::HirTraitDispatchKind::TraitBound,
    );
    target.owner_substitution.push(crate::hir::HirTypeBinding {
        param: crate::types::GenericParamId {
            owner: DefId::new(CrateId(0), LocalDefId(1)),
            index: 0,
        },
        ty: Type::I64,
    });
    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::ArrayLiteral(Vec::new()),
                ty: Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Value(0)),
                span: Span::test(),
            }),
            "index".to_string(),
            vec![HirExpr {
                kind: HirExprKindFor::IntLiteral(0),
                ty: Type::I64,
                span: Span::test(),
            }],
            None,
            target,
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

    assert!(
        error.contains("references unknown trait target"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_trait_target_on_inherent_impl_method_call() {
    let trait_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(30),
    };
    let impl_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(10),
    };
    let method_id = DefId::new(CrateId(0), LocalDefId(11));
    let mut method = test_function(method_id, "dep::Box_value");
    method.is_method = true;

    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_trait(
        &mut products,
        trait_product_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: DefId::new(CrateId(0), LocalDefId(30)),
            name: "dep::Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    insert_interface_impl(
        &mut products,
        impl_product_id,
        HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(10)),
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        },
    );

    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            None,
            HirMethodCallTarget::impl_method(
                DefId::new(CrateId(0), LocalDefId(10)),
                method_id,
                Some(crate::hir::HirSelectedTraitMember {
                    trait_id: DefId::new(CrateId(0), LocalDefId(30)),
                    member_id: method_id,
                    trait_args: Vec::new(),
                }),
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        err.contains("impl target") && err.contains("trait target"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_trait_signature_target_using_trait_id_sentinel() {
    let trait_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(30),
    };
    let trait_id = DefId::new(CrateId(0), LocalDefId(30));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_trait(
        &mut products,
        trait_product_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "dep::Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "value".to_string(),
                HirFunctionSig {
                    id: DefId::new(CrateId(0), LocalDefId(31)),
                    name: "value".to_string(),
                    generic_params: Vec::new(),
                    params: Vec::new(),
                    ret: Type::I64,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        },
    );

    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "value".to_string(),
            Vec::new(),
            None,
            HirMethodCallTarget::trait_method(
                trait_id,
                trait_id,
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        err.contains("does not own selected member"),
        "unexpected error: {err}"
    );
}
