use super::*;

#[test]
fn product_artifact_rejects_unknown_method_call_target_impl_id() {
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
                DefId::new(CrateId(0), LocalDefId(99)),
                DefId::new(CrateId(0), LocalDefId(0)),
                None,
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
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
        err.contains("unknown impl target"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_method_call_authority_for_static_method() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
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
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("make".to_string(), test_function(method_id, "make"))]),
        },
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKindFor::IntLiteral(1),
                ty: Type::I64,
                span: Span::test(),
            }),
            "make".to_string(),
            Vec::new(),
            None,
            HirMethodCallTarget::impl_method(impl_id, method_id, None),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

    assert!(
        error.contains("references a static method"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_static_authority_for_receiver_method() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
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
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        },
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::Call(
            Box::new(HirExpr {
                kind: HirExprKindFor::Var("value".to_string()),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            }),
            Vec::new(),
            Some(crate::hir::HirCallTarget::StaticMethod(
                crate::hir::HirStaticMethodTarget {
                    owner_ty: Type::I64,
                    method: HirMethodCallTarget::impl_method(impl_id, method_id, None),
                },
            )),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let error = validate_artifact_expr(&products, &mut expr).unwrap_err();

    assert!(
        error.contains("references a receiver method"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_method_call_target_method_from_other_impl() {
    let first_impl_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(10),
    };
    let second_impl_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(20),
    };
    let first_method_id = DefId::new(CrateId(0), LocalDefId(11));
    let second_method_id = DefId::new(CrateId(0), LocalDefId(21));
    let mut first_method = test_function(first_method_id, "dep::Box_First_value");
    first_method.is_method = true;
    let mut second_method = test_function(second_method_id, "dep::Box_Second_value");
    second_method.is_method = true;

    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_impl(
        &mut products,
        first_impl_id,
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
            methods: HashMap::from([("value".to_string(), first_method)]),
        },
    );
    insert_interface_impl(
        &mut products,
        second_impl_id,
        HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(20)),
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
            methods: HashMap::from([("value".to_string(), second_method)]),
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
                second_method_id,
                None,
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
        err.contains("method target") && err.contains("impl target"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_method_call_target_method_from_other_trait() {
    let first_trait_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(30),
    };
    let second_trait_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(40),
    };
    let first_method_id = DefId::new(CrateId(0), LocalDefId(31));
    let second_method_id = DefId::new(CrateId(0), LocalDefId(41));
    let mut first_method = test_function(first_method_id, "dep::First_value");
    first_method.is_method = true;
    let mut second_method = test_function(second_method_id, "dep::Second_value");
    second_method.is_method = true;

    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_trait(
        &mut products,
        first_trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: DefId::new(CrateId(0), LocalDefId(30)),
            name: "dep::First".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("value".to_string(), first_method)]),
            signatures: HashMap::new(),
        },
    );
    insert_interface_trait(
        &mut products,
        second_trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: DefId::new(CrateId(0), LocalDefId(40)),
            name: "dep::Second".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("value".to_string(), second_method)]),
            signatures: HashMap::new(),
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
                DefId::new(CrateId(0), LocalDefId(30)),
                second_method_id,
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

#[test]
fn load_product_artifact_accepts_dependency_generic_impl_method_target() {
    let (base, _cleanup) = temp_test_dir("dependency_generic_impl_method_target");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_impl_id = DefId::new(CrateId(0), LocalDefId(10));
    let dep_method_id = DefId::new(CrateId(0), LocalDefId(11));
    let mut dep_method = test_function(dep_method_id, "value");
    dep_method.is_method = true;
    dep_method.self_receiver = Some(crate::types::ReceiverMode::Move);
    dep_method.params.push(crate::hir::HirParam {
        name: "self".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Generic(crate::types::GenericParamId {
            owner: dep_impl_id,
            index: 0,
        }),
        mutable: false,
        is_ref: false,
    });
    let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    let dep_impl_product_id = ProductDefId::from(dep_impl_id);
    let dep_impl = HirImpl {
        id: dep_impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: generic_params(dep_impl_id, &["T"]),
        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Generic(
            crate::types::GenericParamId {
                owner: dep_impl_id,
                index: 0,
            },
        )),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("value".to_string(), dep_method)]),
    };
    insert_interface_impl(&mut dep_products, dep_impl_product_id, dep_impl.clone());
    dep_products
        .bodies
        .generic_impls
        .insert(dep_impl_product_id, dep_impl);
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let app_function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    app_products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("dep".to_string()),
    );
    let mut app_function = app_products.bodies.functions[&app_function_id].clone();
    app_function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
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
                    DefId::new(CrateId(1), LocalDefId(10)),
                    DefId::new(CrateId(1), LocalDefId(11)),
                    None,
                ),
            ),
            ty: Type::I64,
            span: Span::test(),
        })],
        ty: Type::I64,
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    ctx.load_product_artifact_from_path(app_artifact)
        .expect("dependency generic impl method targets should validate");
}
