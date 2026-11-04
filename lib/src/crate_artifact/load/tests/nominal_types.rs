use super::*;

type HirMatchArm = crate::hir::HirMatchArmFor<AcceptedHir>;

#[test]
fn product_artifact_rejects_nested_error_in_unexported_interface_type() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let struct_id = product_def_id(0, 7);
    products.interface.structs.insert(
        struct_id,
        ProductStructInterface {
            id: product_hir_def_id(struct_id),
            name: "dep::Private".to_string(),
            generic_params: Vec::new(),
            fields: vec![ProductFieldInterface {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::Tuple(vec![Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Error),
                }]),
                public: false,
            }],
        },
    );

    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let error =
        super::validate_product_type_fields(&products, &remap, &CrateContext::new()).unwrap_err();

    assert!(error.contains("Type::Error"), "unexpected error: {error}");
}

#[test]
fn product_artifact_rejects_nested_type_var_in_unexported_body_type() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let function_id = product_def_id(0, 0);
    products
        .bodies
        .functions
        .get_mut(&function_id)
        .unwrap()
        .body
        .ty = Type::Tuple(vec![Type::Reference {
        mutable: false,
        inner: Box::new(Type::TypeVar(crate::ids::TypeVarId(0))),
    }]);

    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let error =
        super::validate_product_type_fields(&products, &remap, &CrateContext::new()).unwrap_err();

    assert!(
        error.contains("unresolved type"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_remaps_nominal_type_ids_in_hir_types() {
    let (base, _cleanup) = temp_test_dir("nominal_type_ids");
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
    function.params.push(crate::hir::HirParam {
        name: "widget".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Struct {
            id: struct_def,
            args: vec![Type::Enum {
                id: enum_def,
                args: Vec::new(),
            }],
        },
        mutable: false,
        is_ref: false,
    });
    function.body.stmts = vec![
        HirStmt::Let {
            name: "local".to_string(),
            local_id: crate::ids::HirLocalId(1),
            ty: Type::Struct {
                id: struct_def,
                args: vec![Type::Enum {
                    id: enum_def,
                    args: Vec::new(),
                }],
            },
            value: HirExpr {
                kind: HirExprKindFor::Var("widget".to_string()),
                ty: Type::Struct {
                    id: struct_def,
                    args: vec![Type::Enum {
                        id: enum_def,
                        args: Vec::new(),
                    }],
                },
                span: Span::test(),
            },
            mutable: false,
        },
        HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::Match {
                scrutinee: Box::new(HirExpr {
                    kind: HirExprKindFor::Var("choice".to_string()),
                    ty: Type::Enum {
                        id: enum_def,
                        args: Vec::new(),
                    },
                    span: Span::test(),
                }),
                arms: vec![HirMatchArm {
                    pattern: HirPattern::Or(vec![
                        HirPattern::Struct(
                            "dep::Widget".to_string(),
                            None,
                            vec![Type::Enum {
                                id: enum_def,
                                args: Vec::new(),
                            }],
                            vec![HirStructPatternField {
                                name: "field".to_string(),
                                field: None,
                                pattern: HirPattern::Struct(
                                    "dep::Nested".to_string(),
                                    None,
                                    vec![Type::Struct {
                                        id: struct_def,
                                        args: Vec::new(),
                                    }],
                                    Vec::new(),
                                ),
                            }],
                        ),
                        HirPattern::Enum(
                            "dep::Choice".to_string(),
                            "Some".to_string(),
                            None,
                            vec![HirPattern::Tuple(vec![HirPattern::Struct(
                                "dep::Widget".to_string(),
                                None,
                                vec![Type::Struct {
                                    id: struct_def,
                                    args: Vec::new(),
                                }],
                                Vec::new(),
                            )])],
                        ),
                    ]),
                    guard: None,
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKindFor::IntLiteral(1),
                            ty: Type::I64,
                            span: Span::test(),
                        })],
                        ty: Type::I64,
                    },
                }],
            },
            ty: Type::I64,
            span: Span::test(),
        }),
        HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::Lambda {
                params: vec![HirParam {
                    name: "captured_widget".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Struct {
                        id: struct_def,
                        args: Vec::new(),
                    },
                    mutable: false,
                    is_ref: false,
                }],
                body: HirBlock {
                    stmts: Vec::new(),
                    ty: Type::Enum {
                        id: enum_def,
                        args: Vec::new(),
                    },
                },
                captures: vec![HirClosureCapture {
                    name: "widget".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    kind: HirClosureCaptureKind::SharedBorrow,
                    mutable: false,
                    ty: Type::Struct {
                        id: struct_def,
                        args: vec![Type::Enum {
                            id: enum_def,
                            args: Vec::new(),
                        }],
                    },
                }],
            },
            ty: Type::function(
                vec![Type::Struct {
                    id: struct_def,
                    args: Vec::new(),
                }],
                Type::Enum {
                    id: enum_def,
                    args: Vec::new(),
                },
            ),
            span: Span::test(),
        }),
    ];
    function.ret_type = Type::Enum {
        id: enum_def,
        args: Vec::new(),
    };
    insert_function_body(&mut products, function_id, function);
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
            fields: Vec::new(),
        },
    );
    insert_interface_enum(
        &mut products,
        enum_product_id,
        HirEnum {
            id: enum_def,
            name: "dep::Choice".to_string(),
            generic_params: Vec::new(),
            variants: Vec::new(),
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

    assert_eq!(
        body.params[0].ty,
        Type::Struct {
            id: struct_owner,
            args: vec![Type::Enum {
                id: enum_owner,
                args: Vec::new(),
            }],
        }
    );
    assert_eq!(
        body.ret_type,
        Type::Enum {
            id: enum_owner,
            args: Vec::new(),
        }
    );
    match &body.body.stmts[0] {
        HirStmt::Let { ty, .. } => assert_eq!(
            ty,
            &Type::Struct {
                id: struct_owner,
                args: vec![Type::Enum {
                    id: enum_owner,
                    args: Vec::new(),
                }],
            }
        ),
        other => panic!("expected remapped let type, got {other:?}"),
    }
    let HirStmt::Expr(HirExpr {
        kind: HirExprKindFor::Match { arms, .. },
        ..
    }) = &body.body.stmts[1]
    else {
        panic!("expected match expression");
    };
    let HirPattern::Or(or_patterns) = &arms[0].pattern else {
        panic!("expected or pattern");
    };
    let HirPattern::Struct(_, _, struct_args, struct_fields) = &or_patterns[0] else {
        panic!("expected struct pattern");
    };
    assert_eq!(
        struct_args,
        &vec![Type::Enum {
            id: enum_owner,
            args: Vec::new(),
        }]
    );
    let HirPattern::Struct(_, _, nested_args, _) = &struct_fields[0].pattern else {
        panic!("expected nested struct pattern");
    };
    assert_eq!(
        nested_args,
        &vec![Type::Struct {
            id: struct_owner,
            args: Vec::new(),
        }]
    );
    let HirPattern::Enum(_, _, _, enum_payloads) = &or_patterns[1] else {
        panic!("expected enum pattern");
    };
    let HirPattern::Tuple(tuple_patterns) = &enum_payloads[0] else {
        panic!("expected tuple pattern");
    };
    let HirPattern::Struct(_, _, enum_nested_args, _) = &tuple_patterns[0] else {
        panic!("expected nested enum payload pattern");
    };
    assert_eq!(
        enum_nested_args,
        &vec![Type::Struct {
            id: struct_owner,
            args: Vec::new(),
        }]
    );
    let HirStmt::Expr(HirExpr {
        kind:
            HirExprKindFor::Lambda {
                params,
                body,
                captures,
            },
        ty,
        ..
    }) = &body.body.stmts[2]
    else {
        panic!("expected lambda expression");
    };
    assert_eq!(
        params[0].ty,
        Type::Struct {
            id: struct_owner,
            args: Vec::new(),
        }
    );
    assert_eq!(
        captures[0].ty,
        Type::Struct {
            id: struct_owner,
            args: vec![Type::Enum {
                id: enum_owner,
                args: Vec::new(),
            }],
        }
    );
    assert_eq!(
        body.ty,
        Type::Enum {
            id: enum_owner,
            args: Vec::new(),
        }
    );
    assert_eq!(
        *ty,
        Type::function(
            vec![Type::Struct {
                id: struct_owner,
                args: Vec::new(),
            }],
            Type::Enum {
                id: enum_owner,
                args: Vec::new(),
            },
        )
    );
}

#[test]
fn load_product_artifact_rejects_unknown_nominal_type_id() {
    let (base, _cleanup) = temp_test_dir("unknown_nominal_type_id");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut function = products.bodies.functions[&function_id].clone();
    function.params.push(HirParam {
        name: "ghost".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(999)),
            args: Vec::new(),
        },
        mutable: false,
        is_ref: false,
    });
    insert_function_body(&mut products, function_id, function);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("unknown nominal type ID should be rejected during artifact load");

    assert!(
        err.contains("unknown nominal type definition"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_invalid_projection_type_ids() {
    let (base, _cleanup) = temp_test_dir("invalid_projection_type_ids");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let trait_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(4),
    };
    let trait_id = DefId::new(CrateId(0), LocalDefId(4));
    insert_interface_trait(
        &mut products,
        trait_product_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "dep::Deref".to_string(),
            generic_params: Vec::new(),
            associated_types: vec![HirAssociatedTypeDecl {
                id: AssocTypeId(0),
                name: "Target".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let mut function = products.bodies.functions[&function_id].clone();
    function.params.push(HirParam {
        name: "bad".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Projection {
            ty: Box::new(Type::I64),
            trait_id,
            assoc_type: AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: AssocTypeId(99),
            },
            trait_args: Vec::new(),
        },
        mutable: false,
        is_ref: false,
    });
    insert_function_body(&mut products, function_id, function);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .unwrap_err();

    assert!(
        err.contains("unknown associated type ID"),
        "unexpected error: {err}"
    );
}

fn product_with_projection_trait(
    assoc_type_id: AssocTypeId,
) -> (
    CompilerProducts,
    super::super::ProductIdentityRemap,
    super::ProductNominalTypeValidator,
) {
    let trait_product_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(4),
    };
    let trait_def_id = DefId::new(CrateId(0), LocalDefId(4));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_trait(
        &mut products,
        trait_product_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_def_id,
            name: "dep::Deref".to_string(),
            generic_params: Vec::new(),
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_type_id,
                name: "Target".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);

    (products, remap, validator)
}

#[test]
fn product_artifact_remaps_projection_type_ids() {
    let (_products, remap, validator) = product_with_projection_trait(AssocTypeId(0));
    let mut ty = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id: DefId::new(CrateId(0), LocalDefId(4)),
        assoc_type: AssociatedTypeKey {
            owner: DefId::new(CrateId(0), LocalDefId(4)),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };

    super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap();

    assert_eq!(
        ty,
        Type::Projection {
            ty: Box::new(Type::I64),
            trait_id: DefId::new(CrateId(7), LocalDefId(4)),
            assoc_type: AssociatedTypeKey {
                owner: DefId::new(CrateId(7), LocalDefId(4)),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: Vec::new(),
        }
    );
}

#[test]
fn product_artifact_rejects_unknown_projection_trait_id() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    products.interface.traits.clear();
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut ty = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id: DefId::new(CrateId(0), LocalDefId(4)),
        assoc_type: AssociatedTypeKey {
            owner: DefId::new(CrateId(0), LocalDefId(4)),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };

    let err = super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap_err();

    assert!(
        err.contains("unknown trait definition"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_error_type_in_executable_body() {
    let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::ProductChildLocationValidator::from_products(&products);
    let type_validator = super::ProductNominalTypeValidator::from_products(&products, &remap);
    let mut expr = HirExpr {
        kind: HirExprKindFor::IntLiteral(1),
        ty: Type::Error,
        span: Span::test(),
    };

    let error =
        super::remap_expr_child_locations(&mut expr, &remap, &child_validator, &type_validator, 0)
            .unwrap_err();

    assert!(
        error.contains("unresolved type"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_error_type_in_impl_receiver_metadata() {
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let impl_id = product_def_id(0, 7);
    insert_interface_impl(
        &mut products,
        impl_id,
        HirImpl {
            id: product_hir_def_id(impl_id),
            owner: HirImplOwner::Named("Broken".to_string()),
            type_name: "Broken".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Error),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        },
    );

    let error = super::validate_product_method_authority_maps(
        &products,
        &super::ProductDependencyDefinitions::default(),
    )
    .unwrap_err();

    assert!(
        error.contains("unresolved type"),
        "unexpected error: {error}"
    );
}

#[test]
fn product_artifact_rejects_unknown_projection_assoc_type_id() {
    let (_products, remap, validator) = product_with_projection_trait(AssocTypeId(0));
    let mut ty = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id: DefId::new(CrateId(0), LocalDefId(4)),
        assoc_type: AssociatedTypeKey {
            owner: DefId::new(CrateId(0), LocalDefId(4)),
            assoc_type_id: AssocTypeId(9),
        },
        trait_args: Vec::new(),
    };

    let err = super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap_err();

    assert!(
        err.contains("unknown associated type ID"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_projection_assoc_owner_mismatch_across_crates() {
    let (_products, mut remap, validator) = product_with_projection_trait(AssocTypeId(0));
    remap.crate_ids.insert(ProductCrateId(1), CrateId(8));
    let mut ty = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id: DefId::new(CrateId(0), LocalDefId(4)),
        assoc_type: AssociatedTypeKey {
            owner: DefId::new(CrateId(1), LocalDefId(4)),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };

    let err = super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap_err();

    assert!(
        err.contains("does not match associated type owner"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_unknown_dependency_projection_trait_id() {
    let (_products, mut remap, validator) = product_with_projection_trait(AssocTypeId(0));
    remap.crate_ids.insert(ProductCrateId(1), CrateId(8));
    let mut ty = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id: DefId::new(CrateId(1), LocalDefId(44)),
        assoc_type: AssociatedTypeKey {
            owner: DefId::new(CrateId(1), LocalDefId(44)),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };

    let err = super::remap_type_def_ids(&mut ty, &remap, &validator).unwrap_err();

    assert!(
        err.contains("unknown dependency trait definition"),
        "unexpected error: {err}"
    );
}
