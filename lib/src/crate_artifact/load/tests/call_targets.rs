use super::*;

#[test]
fn product_artifact_rejects_unknown_resolved_var_function_target() {
    let mut expr = HirExpr {
        kind: HirExprKindFor::ResolvedVar(HirVarRef {
            name: "missing".to_string(),
            target: HirVarTarget::Function(DefId::new(CrateId(0), LocalDefId(99))),
        }),
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

    assert!(err.contains("unknown function"), "unexpected error: {err}");
}

#[test]
fn product_artifact_rejects_resolved_var_function_target_to_extern() {
    let extern_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(7),
    };
    let extern_def = DefId::new(CrateId(0), LocalDefId(7));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_extern(
        &mut products,
        extern_id,
        HirExtern {
            id: extern_def,
            name: "dep::foreign".to_string(),
            params: Vec::new(),
            ret: Type::I64,
            variadic: false,
            is_unsafe: false,
        },
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::ResolvedVar(HirVarRef {
            name: "foreign".to_string(),
            target: HirVarTarget::Function(extern_def),
        }),
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
        err.contains("function") && err.contains("extern"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_resolved_var_extern_target_to_function() {
    let function_def = DefId::new(CrateId(0), LocalDefId(0));
    let mut expr = HirExpr {
        kind: HirExprKindFor::ResolvedVar(HirVarRef {
            name: "answer".to_string(),
            target: HirVarTarget::Extern(function_def),
        }),
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
        err.contains("extern") && err.contains("function"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_accepts_dependency_resolved_var_function_target() {
    let (base, _cleanup) = temp_test_dir("dependency_resolved_var_function_target");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
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
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "dep::answer".to_string(),
                target: HirVarTarget::Function(DefId::new(CrateId(1), LocalDefId(0))),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    ctx.load_product_artifact_from_path(app_artifact)
        .expect("dependency function ResolvedVar targets should validate");
}

#[test]
fn load_product_artifact_rejects_dependency_resolved_var_function_target_to_extern() {
    let (base, _cleanup) = temp_test_dir("dependency_resolved_var_wrong_kind");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_extern_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(7),
    };
    let dep_extern_def = DefId::new(CrateId(0), LocalDefId(7));
    let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    insert_interface_extern(
        &mut dep_products,
        dep_extern_id,
        HirExtern {
            id: dep_extern_def,
            name: "dep::foreign".to_string(),
            params: Vec::new(),
            ret: Type::I64,
            variadic: false,
            is_unsafe: false,
        },
    );
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
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "dep::foreign".to_string(),
                target: HirVarTarget::Function(DefId::new(CrateId(1), LocalDefId(7))),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    let err = ctx
        .load_product_artifact_from_path(app_artifact)
        .expect_err("dependency extern target used as function should be rejected");

    assert!(
        err.contains("dependency extern") && err.contains("as function"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_accepts_dependency_resolved_var_extern_target() {
    let (base, _cleanup) = temp_test_dir("dependency_resolved_var_extern_target");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_extern_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(7),
    };
    let dep_extern_def = DefId::new(CrateId(0), LocalDefId(7));
    let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    insert_interface_extern(
        &mut dep_products,
        dep_extern_id,
        HirExtern {
            id: dep_extern_def,
            name: "dep::foreign".to_string(),
            params: Vec::new(),
            ret: Type::I64,
            variadic: false,
            is_unsafe: false,
        },
    );
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
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "dep::foreign".to_string(),
                target: HirVarTarget::Extern(DefId::new(CrateId(1), LocalDefId(7))),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    ctx.load_product_artifact_from_path(app_artifact)
        .expect("dependency extern ResolvedVar targets should validate");
}

#[test]
fn load_product_artifact_rejects_dependency_resolved_var_extern_target_to_function() {
    let (base, _cleanup) = temp_test_dir("dependency_resolved_var_extern_wrong_kind");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
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
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "dep::answer".to_string(),
                target: HirVarTarget::Extern(DefId::new(CrateId(1), LocalDefId(0))),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    let err = ctx
        .load_product_artifact_from_path(app_artifact)
        .expect_err("dependency function target used as extern should be rejected");

    assert!(
        err.contains("dependency function") && err.contains("as extern"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_accepts_resolved_var_function_display_name_mismatch() {
    let (base, _cleanup) = temp_test_dir("resolved_var_function_name_mismatch");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let mut function = products.bodies.functions[&function_id].clone();
    function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "not_answer".to_string(),
                target: HirVarTarget::Function(DefId::new(CrateId(0), LocalDefId(0))),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    products.bodies.functions.insert(function_id, function);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();
}

#[test]
fn product_artifact_rejects_receiver_method_as_function_target() {
    let impl_id = product_def_id(0, 10);
    let method_id = product_def_id(0, 11);
    let method_def = product_hir_def_id(method_id);
    let mut method = test_function(method_def, "value");
    method.is_method = true;
    method.params.push(HirParam {
        name: "self".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::I64,
        mutable: false,
        is_ref: false,
    });
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_impl(
        &mut products,
        impl_id,
        HirImpl {
            id: product_hir_def_id(impl_id),
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
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut reference = HirVarRef {
        name: "Box_value".to_string(),
        target: HirVarTarget::Function(method_def),
    };

    let err = super::remap_var_target_child_location(
        &mut reference,
        &remap,
        &validator,
        &BTreeSet::new(),
    )
    .unwrap_err();

    assert!(
        err.contains("receiver method") && err.contains("as function"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_accepts_static_method_function_display_name_mismatch() {
    let impl_id = product_def_id(0, 12);
    let method_id = product_def_id(0, 13);
    let method_def = product_hir_def_id(method_id);
    let mut method = test_function(method_def, "make");
    method.is_method = false;
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_impl(
        &mut products,
        impl_id,
        HirImpl {
            id: product_hir_def_id(impl_id),
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
            methods: HashMap::from([("make".to_string(), method)]),
        },
    );
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut reference = HirVarRef {
        name: "not_make".to_string(),
        target: HirVarTarget::Function(method_def),
    };

    super::remap_var_target_child_location(&mut reference, &remap, &validator, &BTreeSet::new())
        .unwrap();
}

#[test]
fn product_artifact_rejects_dependency_receiver_method_as_function_target() {
    let app_products = product_with_function("app", ProductCrateId(0), 0, PathBuf::from("app.o"));
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([
            (ProductCrateId(0), CrateId(7)),
            (ProductCrateId(1), CrateId(8)),
        ]),
    };
    let method_id = product_def_id(1, 11);
    let mut dependencies = super::ProductDependencyDefinitions::default();
    dependencies.methods.insert(method_id);
    let validator = super::ProductNominalTypeValidator::from_products_with_dependencies(
        &app_products,
        &remap,
        dependencies,
    );
    let mut reference = HirVarRef {
        name: "dep::Box_value".to_string(),
        target: HirVarTarget::Function(product_hir_def_id(method_id)),
    };

    let err = super::remap_var_target_child_location(
        &mut reference,
        &remap,
        &validator,
        &BTreeSet::new(),
    )
    .unwrap_err();

    assert!(
        err.contains("dependency receiver method") && err.contains("as function"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_allows_dependency_static_impl_method_canonical_name_as_function_target() {
    let app_products = product_with_function("app", ProductCrateId(0), 0, PathBuf::from("app.o"));
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([
            (ProductCrateId(0), CrateId(7)),
            (ProductCrateId(1), CrateId(8)),
        ]),
    };

    let dep_impl_id = DefId::new(CrateId(8), LocalDefId(10));
    let dep_method_id = DefId::new(CrateId(8), LocalDefId(11));
    let dep_method_product_id = product_def_id(1, 11);
    let dep_method = test_function(dep_method_id, "make");
    let dep_impl = HirImpl {
        id: dep_impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("make".to_string(), dep_method)]),
    };
    let mut interface = ArtifactCrateInterface::default();
    interface
        .canonical_names
        .insert(dep_method_id, "dep::Box::make".to_string());
    interface
        .impls
        .insert(dep_impl_id, ProductImplInterface::from(&dep_impl));
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(ExternCrateRecord::new(
        CrateId(8),
        "dep".to_string(),
        ExternCrateMetadata::new(
            interface,
            crate::collect::resolver::ResolverTables::default(),
            BTreeMap::new(),
        ),
        ExternCrateBodies::default(),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    ))
    .unwrap();

    let validator =
        super::ProductNominalTypeValidator::from_products_with_context(&app_products, &remap, &ctx);
    let mut reference = HirVarRef {
        name: "dep::Box::make".to_string(),
        target: HirVarTarget::Function(product_hir_def_id(dep_method_product_id)),
    };

    super::remap_var_target_child_location(&mut reference, &remap, &validator, &BTreeSet::new())
        .unwrap();

    assert_eq!(
        reference.target,
        HirVarTarget::Function(DefId::new(CrateId(8), LocalDefId(11)))
    );
}

#[test]
fn product_dependency_definitions_ignore_body_provider_generic_impls() {
    let app_products = product_with_function("app", ProductCrateId(0), 0, PathBuf::from("app.o"));
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([
            (ProductCrateId(0), CrateId(7)),
            (ProductCrateId(1), CrateId(8)),
        ]),
    };

    let dep_impl_id = DefId::new(CrateId(8), LocalDefId(10));
    let dep_method_id = DefId::new(CrateId(8), LocalDefId(11));
    let mut dep_method = test_function(dep_method_id, "value");
    dep_method.is_method = true;
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
        bounds: HashMap::new().into(),
        methods: HashMap::from([("value".to_string(), dep_method)]),
    };
    let bodies = ExternCrateBodies::from_cross_crate_hir(ArtifactCrossCrateHir {
        generic_functions: BTreeMap::new(),
        traits_with_defaults: BTreeMap::new(),
        generic_impls: vec![dep_impl],
    });
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(ExternCrateRecord::new(
        CrateId(8),
        "dep".to_string(),
        ExternCrateMetadata::new(
            ArtifactCrateInterface::default(),
            crate::collect::resolver::ResolverTables::default(),
            BTreeMap::new(),
        ),
        bodies,
        ExternCrateLink::metadata_only(BTreeMap::new()),
    ))
    .unwrap();

    let validator = super::ProductChildLocationValidator::from_products_with_context(
        &app_products,
        &remap,
        &ctx,
    );
    let type_validator =
        super::ProductNominalTypeValidator::from_products_with_context(&app_products, &remap, &ctx);
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
                DefId::new(CrateId(1), LocalDefId(10)),
                DefId::new(CrateId(1), LocalDefId(11)),
                None,
            ),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    let err = super::remap_expr_child_locations(&mut expr, &remap, &validator, &type_validator, 0)
        .expect_err("dependency body providers must not authorize method targets");

    assert!(
        err.contains("unknown dependency impl target"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_accepts_resolved_var_extern_display_name_mismatch() {
    let (base, _cleanup) = temp_test_dir("resolved_var_extern_name_mismatch");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let extern_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(7),
    };
    let extern_def = DefId::new(CrateId(0), LocalDefId(7));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    insert_interface_extern(
        &mut products,
        extern_id,
        HirExtern {
            id: extern_def,
            name: "dep::foreign".to_string(),
            params: Vec::new(),
            ret: Type::I64,
            variadic: false,
            is_unsafe: false,
        },
    );
    let mut function = products.bodies.functions[&function_id].clone();
    function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "answer".to_string(),
                target: HirVarTarget::Extern(extern_def),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    products.bodies.functions.insert(function_id, function);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();
}

#[test]
fn load_product_artifact_accepts_dependency_resolved_var_function_display_name_mismatch() {
    let (base, _cleanup) = temp_test_dir("dependency_resolved_var_function_name_mismatch");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
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
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "app::answer".to_string(),
                target: HirVarTarget::Function(DefId::new(CrateId(1), LocalDefId(0))),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };
    app_products
        .bodies
        .functions
        .insert(app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    ctx.load_product_artifact_from_path(app_artifact).unwrap();
}

#[test]
fn product_artifact_accepts_ambiguous_display_name_with_exact_function_target() {
    let left_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(10),
    };
    let right_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(11),
    };
    let left_def = DefId::new(CrateId(0), LocalDefId(10));
    let right_def = DefId::new(CrateId(0), LocalDefId(11));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    products
        .identity_table
        .display_names
        .insert(left_id, "left::value".to_string());
    products
        .identity_table
        .display_names
        .insert(right_id, "right::value".to_string());
    insert_interface_function(&mut products, left_id, &test_function(left_def, "value"));
    insert_interface_function(&mut products, right_id, &test_function(right_def, "value"));
    let mut expr = HirExpr {
        kind: HirExprKindFor::ResolvedVar(HirVarRef {
            name: "value".to_string(),
            target: HirVarTarget::Function(right_def),
        }),
        ty: Type::function(Vec::new(), Type::I64),
        span: Span::test(),
    };
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
        .unwrap();
}
