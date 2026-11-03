use super::*;

#[test]
fn product_artifact_rejects_unknown_struct_literal_sidecar_without_fields() {
    let missing_struct = DefId::new(CrateId(0), LocalDefId(99));
    let mut expr = HirExpr {
        kind: HirExprKindFor::StructLiteral("Ghost".to_string(), Some(missing_struct), Vec::new()),
        ty: Type::I64,
        span: Span::test(),
    };
    let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::super::ProductChildLocationValidator::from_products(&products);
    let type_validator =
        super::super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err = super::super::remap_expr_child_locations(
        &mut expr,
        &remap,
        &child_validator,
        &type_validator,
        0,
    )
    .unwrap_err();

    assert!(
        err.contains("unknown nominal type definition") && err.contains("as struct"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_unknown_struct_pattern_sidecar_without_fields() {
    let missing_struct = DefId::new(CrateId(0), LocalDefId(99));
    let mut pattern = HirPattern::Struct(
        "Ghost".to_string(),
        Some(missing_struct),
        Vec::new(),
        Vec::new(),
    );
    let products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::super::ProductChildLocationValidator::from_products(&products);
    let type_validator =
        super::super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err = super::super::remap_pattern_child_locations(
        &mut pattern,
        &remap,
        &child_validator,
        &type_validator,
        0,
    )
    .unwrap_err();

    assert!(
        err.contains("unknown nominal type definition") && err.contains("as struct"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_struct_literal_field_owner_mismatch() {
    let first_struct_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(5),
    };
    let second_struct_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(6),
    };
    let first_struct_def = DefId::new(CrateId(0), LocalDefId(5));
    let second_struct_def = DefId::new(CrateId(0), LocalDefId(6));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_struct(
        &mut products,
        first_struct_id,
        HirStruct {
            id: first_struct_def,
            name: "dep::First".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );
    insert_interface_struct(
        &mut products,
        second_struct_id,
        HirStruct {
            id: second_struct_def,
            name: "dep::Second".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );
    let mut expr = HirExpr {
        kind: HirExprKindFor::StructLiteral(
            "First".to_string(),
            Some(first_struct_def),
            vec![crate::hir::HirStructLiteralFieldFor::<AcceptedHir> {
                name: "value".to_string(),
                value: HirExpr {
                    kind: HirExprKindFor::IntLiteral(1),
                    ty: Type::I64,
                    span: Span::test(),
                },
                field: Some(HirFieldLocation {
                    owner: second_struct_def,
                    field_id: FieldId(0),
                    name: "value".to_string(),
                }),
            }],
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::super::ProductChildLocationValidator::from_products(&products);
    let type_validator =
        super::super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err = super::super::remap_expr_child_locations(
        &mut expr,
        &remap,
        &child_validator,
        &type_validator,
        0,
    )
    .unwrap_err();

    assert!(
        err.contains("struct literal field owner mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_artifact_rejects_struct_pattern_field_owner_mismatch() {
    let first_struct_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(5),
    };
    let second_struct_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(6),
    };
    let first_struct_def = DefId::new(CrateId(0), LocalDefId(5));
    let second_struct_def = DefId::new(CrateId(0), LocalDefId(6));
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_struct(
        &mut products,
        first_struct_id,
        HirStruct {
            id: first_struct_def,
            name: "dep::First".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );
    insert_interface_struct(
        &mut products,
        second_struct_id,
        HirStruct {
            id: second_struct_def,
            name: "dep::Second".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        },
    );
    let mut pattern = HirPattern::Struct(
        "First".to_string(),
        Some(first_struct_def),
        Vec::new(),
        vec![HirStructPatternField {
            name: "value".to_string(),
            field: Some(HirFieldLocation {
                owner: second_struct_def,
                field_id: FieldId(0),
                name: "value".to_string(),
            }),
            pattern: HirPattern::Wildcard,
        }],
    );
    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = super::super::ProductChildLocationValidator::from_products(&products);
    let type_validator =
        super::super::ProductNominalTypeValidator::from_products(&products, &remap);

    let err = super::super::remap_pattern_child_locations(
        &mut pattern,
        &remap,
        &child_validator,
        &type_validator,
        0,
    )
    .unwrap_err();

    assert!(
        err.contains("struct pattern field owner mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_dependency_field_sidecar_mismatch() {
    let (base, _cleanup) = temp_test_dir("dependency_field_sidecar_mismatch");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_struct_id = DefId::new(CrateId(0), LocalDefId(5));
    let dep_struct_product_id = ProductDefId::from(dep_struct_id);
    let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    dep_products
        .identity_table
        .display_names
        .insert(dep_struct_product_id, "dep::Widget".to_string());
    insert_interface_struct(
        &mut dep_products,
        dep_struct_product_id,
        HirStruct {
            id: dep_struct_id,
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
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let app_function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let dep_struct_ref = DefId::new(CrateId(1), LocalDefId(5));
    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    app_products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("dep".to_string()),
    );
    let mut app_function = app_products.bodies.functions[&app_function_id].clone();
    app_function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::FieldAccess(
                Box::new(HirExpr {
                    kind: HirExprKindFor::Var("widget".to_string()),
                    ty: Type::Struct {
                        id: dep_struct_ref,
                        args: Vec::new(),
                    },
                    span: Span::test(),
                }),
                "value".to_string(),
                Some(HirFieldLocation {
                    owner: dep_struct_ref,
                    field_id: FieldId(0),
                    name: "other".to_string(),
                }),
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
    let err = ctx
        .load_product_artifact_from_path(app_artifact)
        .expect_err("dependency field sidecar mismatch should be rejected");

    assert!(
        err.contains("field sidecar mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_dependency_variant_sidecar_mismatch() {
    let (base, _cleanup) = temp_test_dir("dependency_variant_sidecar_mismatch");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_enum_id = DefId::new(CrateId(0), LocalDefId(6));
    let dep_enum_product_id = ProductDefId::from(dep_enum_id);
    let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    dep_products
        .identity_table
        .display_names
        .insert(dep_enum_product_id, "dep::Choice".to_string());
    insert_interface_enum(
        &mut dep_products,
        dep_enum_product_id,
        HirEnum {
            id: dep_enum_id,
            name: "dep::Choice".to_string(),
            generic_params: Vec::new(),
            variants: vec![HirVariant {
                id: VariantId(0),
                name: "Some".to_string(),
                fields: HirVariantFields::Unit,
            }],
        },
    );
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let app_function_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(0),
    };
    let dep_enum_ref = DefId::new(CrateId(1), LocalDefId(6));
    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    app_products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("dep".to_string()),
    );
    let mut app_function = app_products.bodies.functions[&app_function_id].clone();
    app_function.body = HirBlock {
        stmts: vec![HirStmt::Expr(HirExpr {
            kind: HirExprKindFor::EnumVariant(
                "Choice".to_string(),
                "Some".to_string(),
                Vec::new(),
                Some(HirVariantLocation {
                    owner: dep_enum_ref,
                    variant_id: VariantId(0),
                    name: "None".to_string(),
                }),
            ),
            ty: Type::Enum {
                id: dep_enum_ref,
                args: Vec::new(),
            },
            span: Span::test(),
        })],
        ty: Type::Enum {
            id: dep_enum_ref,
            args: Vec::new(),
        },
    };
    app_function.ret_type = Type::Enum {
        id: dep_enum_ref,
        args: Vec::new(),
    };
    insert_function_body(&mut app_products, app_function_id, app_function);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    let err = ctx
        .load_product_artifact_from_path(app_artifact)
        .expect_err("dependency variant sidecar mismatch should be rejected");

    assert!(
        err.contains("variant sidecar mismatch"),
        "unexpected error: {err}"
    );
}
