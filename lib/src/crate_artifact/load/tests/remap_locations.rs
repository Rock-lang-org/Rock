use super::*;

#[test]
fn remap_var_target_child_location_rejects_serialized_instance_target() {
    let product_crate = ProductCrateId(1);
    let remap = super::super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table: ProductIdentityTable::default(),
        interface: crate::products::ProductInterface::default(),
        bodies: ProductBodies::default(),
        link: ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: BTreeMap::new(),
        proc_macros: Vec::new(),
    };
    let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut reference = HirVarRef {
        name: "identity".to_string(),
        target: HirVarTarget::Instance(crate::ids::InstanceId(5)),
    };

    let err = super::remap_var_target_child_location(
        &mut reference,
        &remap,
        &validator,
        &BTreeSet::new(),
    )
    .unwrap_err();

    assert!(
        err.contains("local instance target"),
        "unexpected error: {err}"
    );
}

#[test]
fn remap_pattern_child_locations_remaps_new_pattern_sidecars() {
    let product_crate = ProductCrateId(1);
    let old_struct = product_def_id(1, 10);
    let old_enum = product_def_id(1, 11);
    let remap = super::super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let mut products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table: ProductIdentityTable::default(),
        interface: crate::products::ProductInterface::default(),
        bodies: ProductBodies::default(),
        link: ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: BTreeMap::new(),
        proc_macros: Vec::new(),
    };
    let struct_def = HirStruct {
        id: product_hir_def_id(old_struct),
        name: "dep::Box".to_string(),
        generic_params: Vec::new(),
        fields: vec![HirField {
            id: FieldId(0),
            name: "value".to_string(),
            ty: Type::I64,
            public: true,
        }],
    };
    products
        .interface
        .structs
        .insert(old_struct, ProductStructInterface::from(&struct_def));
    let enum_def = HirEnum {
        id: product_hir_def_id(old_enum),
        name: "dep::Option".to_string(),
        generic_params: Vec::new(),
        variants: vec![HirVariant {
            id: VariantId(0),
            name: "Some".to_string(),
            fields: HirVariantFields::Positional(vec![Type::I64]),
        }],
    };
    products
        .interface
        .enums
        .insert(old_enum, ProductEnumInterface::from(&enum_def));
    let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let mut pattern = HirPattern::Enum(
        "dep::Option".to_string(),
        "Some".to_string(),
        Some(HirVariantLocation {
            owner: product_hir_def_id(old_enum),
            variant_id: VariantId(0),
            name: "Some".to_string(),
        }),
        vec![HirPattern::Struct(
            "dep::Box".to_string(),
            Some(product_hir_def_id(old_struct)),
            vec![Type::Struct {
                id: product_hir_def_id(old_struct),
                args: Vec::new(),
            }],
            vec![HirStructPatternField {
                name: "value".to_string(),
                field: Some(HirFieldLocation {
                    owner: product_hir_def_id(old_struct),
                    field_id: FieldId(0),
                    name: "value".to_string(),
                }),
                pattern: HirPattern::Wildcard,
            }],
        )],
    );

    super::remap_pattern_child_locations(&mut pattern, &remap, &child_validator, &validator, 0)
        .unwrap();

    let expected_struct = remap.def_id(old_struct).unwrap();
    let expected_enum = remap.def_id(old_enum).unwrap();
    let HirPattern::Enum(_, _, Some(location), payloads) = pattern else {
        panic!("expected enum pattern");
    };
    assert_eq!(location.owner, expected_enum);
    let HirPattern::Struct(_, Some(struct_id), args, fields) = &payloads[0] else {
        panic!("expected struct payload pattern");
    };
    assert_eq!(*struct_id, expected_struct);
    assert_eq!(
        args[0],
        Type::Struct {
            id: expected_struct,
            args: Vec::new(),
        }
    );
    assert_eq!(fields[0].field.as_ref().unwrap().owner, expected_struct);
}

#[test]
fn remap_expr_child_locations_remaps_call_target_sidecars() {
    let product_crate = ProductCrateId(1);
    let function_product_id = product_def_id(1, 10);
    let extern_product_id = product_def_id(1, 11);
    let function_id = product_hir_def_id(function_product_id);
    let extern_id = product_hir_def_id(extern_product_id);
    let remap = super::super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let mut products = product_with_function(
        "dep",
        product_crate,
        function_product_id.local_id.0,
        PathBuf::from("dep.o"),
    );
    insert_interface_extern(
        &mut products,
        extern_product_id,
        HirExtern {
            id: extern_id,
            name: "foreign".to_string(),
            params: Vec::new(),
            ret: Type::I64,
            variadic: false,
            is_unsafe: false,
        },
    );
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut expr = HirExpr {
        kind: HirExprKindFor::TupleLiteral(vec![
            HirExpr {
                kind: HirExprKindFor::Call(
                    Box::new(HirExpr {
                        kind: HirExprKindFor::Var("dep::answer".to_string()),
                        ty: Type::function(Vec::new(), Type::I64),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    Some(HirCallTarget::Function(function_id)),
                ),
                ty: Type::I64,
                span: Span::test(),
            },
            HirExpr {
                kind: HirExprKindFor::Call(
                    Box::new(HirExpr {
                        kind: HirExprKindFor::Var("foreign".to_string()),
                        ty: Type::function(Vec::new(), Type::I64),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    Some(HirCallTarget::Extern(extern_id)),
                ),
                ty: Type::I64,
                span: Span::test(),
            },
        ]),
        ty: Type::Tuple(vec![Type::I64, Type::I64]),
        span: Span::test(),
    };

    super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
        .unwrap();

    let HirExprKindFor::TupleLiteral(items) = &expr.kind else {
        panic!("expected tuple literal");
    };
    let HirExprKindFor::Call(_, _, Some(function_target)) = &items[0].kind else {
        panic!("expected function call target");
    };
    assert_eq!(
        function_target,
        &HirCallTarget::Function(remap.def_id(function_product_id).unwrap())
    );
    let HirExprKindFor::Call(_, _, Some(extern_target)) = &items[1].kind else {
        panic!("expected extern call target");
    };
    assert_eq!(
        extern_target,
        &HirCallTarget::Extern(remap.def_id(extern_product_id).unwrap())
    );
}

#[test]
fn remap_function_child_locations_rejects_unknown_local_call_target() {
    let product_crate = ProductCrateId(1);
    let function_product_id = product_def_id(1, 10);
    let remap = super::super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let products = product_with_function(
        "dep",
        product_crate,
        function_product_id.local_id.0,
        PathBuf::from("dep.o"),
    );
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut function = products.bodies.functions[&function_product_id].clone();
    function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::Call(
                Box::new(HirExpr {
                    kind: HirExprKindFor::Var("local_fn".to_string()),
                    ty: Type::function(Vec::new(), Type::I64),
                    span: Span::test(),
                }),
                Vec::new(),
                Some(HirCallTarget::Local(crate::ids::HirLocalId(99))),
            ),
            ty: Type::I64,
            span: Span::test(),
        })],
        ty: Type::I64,
    };

    let err = super::remap_function_child_locations(
        &mut function,
        &remap,
        &child_validator,
        &type_validator,
    )
    .unwrap_err();

    assert!(
        err.contains("unknown local call target"),
        "unexpected error: {err}"
    );
}

#[test]
fn remap_function_child_locations_rejects_unknown_local_resolved_var_target() {
    let product_crate = ProductCrateId(1);
    let function_product_id = product_def_id(1, 10);
    let remap = super::super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let products = product_with_function(
        "dep",
        product_crate,
        function_product_id.local_id.0,
        PathBuf::from("dep.o"),
    );
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut function = products.bodies.functions[&function_product_id].clone();
    function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::ResolvedVar(HirVarRef {
                name: "local_fn".to_string(),
                target: HirVarTarget::Local(crate::ids::HirLocalId(99)),
            }),
            ty: Type::function(Vec::new(), Type::I64),
            span: Span::test(),
        })],
        ty: Type::function(Vec::new(), Type::I64),
    };

    let err = super::remap_function_child_locations(
        &mut function,
        &remap,
        &child_validator,
        &type_validator,
    )
    .unwrap_err();

    assert!(
        err.contains("unknown local var target"),
        "unexpected error: {err}"
    );
}

#[test]
fn remap_function_child_locations_rejects_call_target_to_lambda_local() {
    let product_crate = ProductCrateId(1);
    let function_product_id = product_def_id(1, 10);
    let lambda_local = crate::ids::HirLocalId(42);
    let remap = super::super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let products = product_with_function(
        "dep",
        product_crate,
        function_product_id.local_id.0,
        PathBuf::from("dep.o"),
    );
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut function = products.bodies.functions[&function_product_id].clone();
    function.body = HirBlock {
        stmts: vec![
            HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::Lambda {
                    params: vec![HirParam {
                        name: "callback".to_string(),
                        local_id: lambda_local,
                        ty: Type::function(Vec::new(), Type::I64),
                        mutable: false,
                        is_ref: false,
                    }],
                    body: HirBlock {
                        stmts: Vec::new(),
                        ty: Type::Unit,
                    },
                    captures: Vec::new(),
                },
                ty: Type::function(Vec::new(), Type::Unit),
                span: Span::test(),
            }),
            HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::Call(
                    Box::new(HirExpr {
                        kind: HirExprKindFor::Var("callback".to_string()),
                        ty: Type::function(Vec::new(), Type::I64),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    Some(HirCallTarget::Local(lambda_local)),
                ),
                ty: Type::I64,
                span: Span::test(),
            }),
        ],
        ty: Type::I64,
    };

    let err = super::remap_function_child_locations(
        &mut function,
        &remap,
        &child_validator,
        &type_validator,
    )
    .unwrap_err();

    assert!(
        err.contains("unknown local call target"),
        "unexpected error: {err}"
    );
}

#[test]
fn remap_function_child_locations_rejects_call_target_to_current_let_local() {
    let product_crate = ProductCrateId(1);
    let function_product_id = product_def_id(1, 10);
    let let_local = crate::ids::HirLocalId(7);
    let remap = super::super::ProductIdentityRemap {
        local_crate: product_crate,
        crate_ids: BTreeMap::from([(product_crate, CrateId(9))]),
    };
    let products = product_with_function(
        "dep",
        product_crate,
        function_product_id.local_id.0,
        PathBuf::from("dep.o"),
    );
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut function = products.bodies.functions[&function_product_id].clone();
    function.body = HirBlock {
        stmts: vec![HirStmt::Let {
            name: "local_fn".to_string(),
            local_id: let_local,
            ty: Type::I64,
            value: HirExpr {
                kind: HirExprKindFor::Call(
                    Box::new(HirExpr {
                        kind: HirExprKindFor::Var("local_fn".to_string()),
                        ty: Type::function(Vec::new(), Type::I64),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    Some(HirCallTarget::Local(let_local)),
                ),
                ty: Type::I64,
                span: Span::test(),
            },
            mutable: false,
        }],
        ty: Type::I64,
    };

    let err = super::remap_function_child_locations(
        &mut function,
        &remap,
        &child_validator,
        &type_validator,
    )
    .unwrap_err();

    assert!(
        err.contains("unknown local call target"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_remaps_child_locations_in_generic_body() {
    let (base, _cleanup) = temp_test_dir("generic_body_child_locations");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let struct_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(5),
    };
    let enum_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(6),
    };
    let struct_def = DefId::new(CrateId(0), LocalDefId(5));
    let enum_def = DefId::new(CrateId(0), LocalDefId(6));

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut function = products.bodies.functions[&function_id].clone();
    function.body.stmts = vec![
        HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::FieldAccess(
                Box::new(HirExpr {
                    kind: HirExprKindFor::Var("widget".to_string()),
                    ty: Type::Struct {
                        id: struct_def,
                        args: Vec::new(),
                    },
                    span: Span::test(),
                }),
                "value".to_string(),
                Some(HirFieldLocation {
                    owner: struct_def,
                    field_id: FieldId(0),
                    name: "value".to_string(),
                }),
            ),
            ty: Type::I64,
            span: Span::test(),
        }),
        HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::EnumVariant(
                "dep::Choice".to_string(),
                "Some".to_string(),
                Vec::new(),
                Some(HirVariantLocation {
                    owner: enum_def,
                    variant_id: VariantId(0),
                    name: "Some".to_string(),
                }),
            ),
            ty: Type::Enum {
                id: enum_def,
                args: Vec::new(),
            },
            span: Span::test(),
        }),
    ];
    products.bodies.functions.insert(function_id, function);
    products
        .identity_table
        .display_names
        .insert(struct_product_id, "dep::Widget".to_string());
    products
        .identity_table
        .display_names
        .insert(enum_product_id, "dep::Choice".to_string());
    insert_interface_struct(
        &mut products,
        struct_product_id,
        HirStruct {
            id: struct_def,
            name: "dep::Widget".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );
    insert_interface_enum(
        &mut products,
        enum_product_id,
        HirEnum {
            id: enum_def,
            name: "dep::Choice".to_string(),
            generic_params: Vec::new(),
            variants: vec![HirVariant {
                id: VariantId(0),
                name: "Some".to_string(),
                fields: HirVariantFields::Unit,
            }],
        },
    );
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let interface = dep.metadata().interface();
    let function_id = interface
        .function_by_canonical_name("dep::answer")
        .expect("function interface should exist")
        .id;
    let body = dep
        .body_providers()
        .generic_function(function_id)
        .expect("generic function body should exist");
    let struct_owner = interface
        .struct_by_canonical_name("dep::Widget")
        .expect("struct interface should exist")
        .id;
    let enum_owner = interface
        .enum_by_canonical_name("dep::Choice")
        .expect("enum interface should exist")
        .id;
    assert_ne!(struct_owner.crate_id, CrateId(0));
    assert_ne!(enum_owner.crate_id, CrateId(0));

    match &body.body.stmts[0] {
        HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::FieldAccess(_, _, Some(location)),
            ..
        }) => assert_eq!(location.owner, struct_owner),
        other => panic!("expected remapped field location, got {other:?}"),
    }
    match &body.body.stmts[1] {
        HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::EnumVariant(_, _, _, Some(location)),
            ..
        }) => assert_eq!(location.owner, enum_owner),
        other => panic!("expected remapped variant location, got {other:?}"),
    }
}

#[test]
fn product_child_location_validator_rejects_mismatched_field_sidecar() {
    let struct_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(5),
    };
    let struct_def = DefId::new(CrateId(0), LocalDefId(5));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_struct(
        &mut products,
        struct_product_id,
        HirStruct {
            id: struct_def,
            name: "dep::Widget".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );

    let validator = super::ProductChildLocationValidator::from_products(&products);
    let err = validator
        .validate_field(struct_product_id, FieldId(0), "other", "value")
        .unwrap_err();

    assert!(
        err.contains("field sidecar mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_child_location_remap_rejects_excessive_depth() {
    let mut expr = HirExpr {
        kind: HirExprKindFor::IntLiteral(1),
        ty: Type::I64,
        span: Span::test(),
    };
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::new(),
    };
    let validator = super::ProductChildLocationValidator {
        fields_by_id: HashMap::new(),
        variants_by_id: HashMap::new(),
    };
    let type_validator = super::ProductNominalTypeValidator {
        normalization_env: Default::default(),
        local_crate: ProductCrateId(0),
        structs: BTreeSet::new(),
        enums: BTreeSet::new(),
        type_aliases: BTreeSet::new(),
        traits: BTreeMap::new(),
        trait_generic_counts: BTreeMap::new(),
        trait_methods: BTreeMap::new(),
        trait_signatures: BTreeMap::new(),
        impls: BTreeSet::new(),
        impl_traits: BTreeMap::new(),
        impl_trait_args: BTreeMap::new(),
        impl_methods: BTreeMap::new(),
        effective_trait_methods: BTreeMap::new(),
        methods: BTreeSet::new(),
        static_methods: BTreeSet::new(),
        method_receiver_modes: BTreeMap::new(),
        functions: BTreeSet::new(),
        externs: BTreeSet::new(),
        dependency_structs: BTreeSet::new(),
        dependency_enums: BTreeSet::new(),
        dependency_type_aliases: BTreeSet::new(),
        dependency_traits: BTreeMap::new(),
        dependency_trait_generic_counts: BTreeMap::new(),
        dependency_trait_methods: BTreeMap::new(),
        dependency_trait_signatures: BTreeMap::new(),
        dependency_impls: BTreeSet::new(),
        dependency_impl_traits: BTreeMap::new(),
        dependency_impl_trait_args: BTreeMap::new(),
        dependency_impl_methods: BTreeMap::new(),
        dependency_effective_trait_methods: BTreeMap::new(),
        dependency_methods: BTreeSet::new(),
        dependency_static_methods: BTreeSet::new(),
        dependency_method_receiver_modes: BTreeMap::new(),
        dependency_functions: BTreeSet::new(),
        dependency_externs: BTreeSet::new(),
    };

    let err = super::remap_expr_child_locations(
        &mut expr,
        &remap,
        &validator,
        &type_validator,
        super::MAX_ARTIFACT_HIR_REMAP_DEPTH + 1,
    )
    .unwrap_err();

    assert!(err.contains("maximum remap depth"));
}
