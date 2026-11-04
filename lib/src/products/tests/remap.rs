use super::*;

#[test]
fn remap_expr_location_product_ids_remaps_new_reference_sidecars() {
    let old_struct = DefId::new(CrateId(0), LocalDefId(1));
    let old_function = DefId::new(CrateId(0), LocalDefId(2));
    let old_extern = DefId::new(CrateId(0), LocalDefId(3));
    let new_struct = DefId::new(CrateId(7), LocalDefId(11));
    let new_function = DefId::new(CrateId(7), LocalDefId(12));
    let new_extern = DefId::new(CrateId(7), LocalDefId(13));
    let mut remap = BTreeMap::new();
    remap.insert(
        ProductDefId::from(old_struct),
        BTreeSet::from([ProductDefId::from(new_struct)]),
    );
    remap.insert(
        ProductDefId::from(old_function),
        BTreeSet::from([ProductDefId::from(new_function)]),
    );
    remap.insert(
        ProductDefId::from(old_extern),
        BTreeSet::from([ProductDefId::from(new_extern)]),
    );

    let mut struct_expr = HirExpr {
        kind: crate::hir::HirExprKindFor::StructLiteral(
            "Box".to_string(),
            Some(old_struct),
            vec![HirStructLiteralField {
                name: "value".to_string(),
                value: HirExpr {
                    kind: crate::hir::HirExprKindFor::ResolvedVar(HirVarRef {
                        name: "make".to_string(),
                        target: HirVarTarget::Function(old_function),
                    }),
                    ty: Type::I64,
                    span: Span::test(),
                },
                field: Some(HirFieldLocation {
                    owner: old_struct,
                    field_id: FieldId(0),
                    name: "value".to_string(),
                }),
            }],
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let mut extern_expr = HirExpr {
        kind: crate::hir::HirExprKindFor::ResolvedVar(HirVarRef {
            name: "puts".to_string(),
            target: HirVarTarget::Extern(old_extern),
        }),
        ty: Type::I64,
        span: Span::test(),
    };

    super::remap_expr_location_product_ids(&mut struct_expr, &remap);
    super::remap_expr_location_product_ids(&mut extern_expr, &remap);

    let crate::hir::HirExprKindFor::StructLiteral(_, Some(struct_id), fields) = &struct_expr.kind
    else {
        panic!("expected struct literal");
    };
    assert_eq!(*struct_id, new_struct);
    assert_eq!(fields[0].field.as_ref().unwrap().owner, new_struct);
    let crate::hir::HirExprKindFor::ResolvedVar(reference) = &fields[0].value.kind else {
        panic!("expected resolved var field value");
    };
    assert_eq!(reference.target, HirVarTarget::Function(new_function));
    let crate::hir::HirExprKindFor::ResolvedVar(reference) = &extern_expr.kind else {
        panic!("expected resolved extern var");
    };
    assert_eq!(reference.target, HirVarTarget::Extern(new_extern));
}

#[test]
fn remap_expr_location_product_ids_remaps_call_target_sidecars() {
    let old_function = DefId::new(CrateId(0), LocalDefId(2));
    let old_extern = DefId::new(CrateId(0), LocalDefId(3));
    let new_function = DefId::new(CrateId(7), LocalDefId(12));
    let new_extern = DefId::new(CrateId(7), LocalDefId(13));
    let mut remap = BTreeMap::new();
    remap.insert(
        ProductDefId::from(old_function),
        BTreeSet::from([ProductDefId::from(new_function)]),
    );
    remap.insert(
        ProductDefId::from(old_extern),
        BTreeSet::from([ProductDefId::from(new_extern)]),
    );

    let mut function_call = HirExpr {
        kind: crate::hir::HirExprKindFor::Call(
            Box::new(HirExpr {
                kind: crate::hir::HirExprKindFor::Var("make".to_string()),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            }),
            Vec::new(),
            Some(HirCallTarget::Function(old_function)),
        ),
        ty: Type::I64,
        span: Span::test(),
    };
    let mut extern_call = HirExpr {
        kind: crate::hir::HirExprKindFor::Call(
            Box::new(HirExpr {
                kind: crate::hir::HirExprKindFor::Var("puts".to_string()),
                ty: Type::function(Vec::new(), Type::I64),
                span: Span::test(),
            }),
            Vec::new(),
            Some(HirCallTarget::Extern(old_extern)),
        ),
        ty: Type::I64,
        span: Span::test(),
    };

    super::remap_expr_location_product_ids(&mut function_call, &remap);
    super::remap_expr_location_product_ids(&mut extern_call, &remap);

    let crate::hir::HirExprKindFor::Call(_, _, Some(function_target)) = &function_call.kind else {
        panic!("expected function call target");
    };
    assert_eq!(function_target, &HirCallTarget::Function(new_function));
    let crate::hir::HirExprKindFor::Call(_, _, Some(extern_target)) = &extern_call.kind else {
        panic!("expected extern call target");
    };
    assert_eq!(extern_target, &HirCallTarget::Extern(new_extern));
}

#[test]
fn remap_pattern_owned_type_ids_remaps_new_pattern_sidecars() {
    let old_id = DefId::new(CrateId(0), LocalDefId(1));
    let new_id = DefId::new(CrateId(0), LocalDefId(2));
    let mut pattern = HirPattern::Enum(
        "Option".to_string(),
        "Some".to_string(),
        Some(HirVariantLocation {
            owner: old_id,
            variant_id: VariantId(0),
            name: "Some".to_string(),
        }),
        vec![HirPattern::Struct(
            "Box".to_string(),
            Some(old_id),
            vec![Type::Struct {
                id: old_id,
                args: Vec::new(),
            }],
            vec![HirStructPatternField {
                name: "value".to_string(),
                field: Some(HirFieldLocation {
                    owner: old_id,
                    field_id: FieldId(0),
                    name: "value".to_string(),
                }),
                pattern: HirPattern::Wildcard,
            }],
        )],
    );

    super::remap_pattern_owned_type_ids(&mut pattern, old_id, new_id);

    let HirPattern::Enum(_, _, Some(location), payloads) = pattern else {
        panic!("expected enum pattern");
    };
    assert_eq!(location.owner, new_id);
    let HirPattern::Struct(_, Some(struct_id), args, fields) = &payloads[0] else {
        panic!("expected struct payload pattern");
    };
    assert_eq!(*struct_id, new_id);
    assert_eq!(
        args[0],
        Type::Struct {
            id: old_id,
            args: Vec::new(),
        }
    );
    assert_eq!(fields[0].field.as_ref().unwrap().owner, new_id);
}

#[test]
fn remap_expr_location_product_ids_remaps_match_pattern_sidecars() {
    let old_id = DefId::new(CrateId(0), LocalDefId(1));
    let new_id = DefId::new(CrateId(7), LocalDefId(2));
    let mut remap = BTreeMap::new();
    remap.insert(
        ProductDefId::from(old_id),
        BTreeSet::from([ProductDefId::from(new_id)]),
    );
    let mut expr = HirExpr {
        kind: crate::hir::HirExprKindFor::Match {
            scrutinee: Box::new(HirExpr {
                kind: crate::hir::HirExprKindFor::Var("value".to_string()),
                ty: Type::I64,
                span: Span::test(),
            }),
            arms: vec![crate::hir::HirMatchArm {
                pattern: HirPattern::Enum(
                    "Option".to_string(),
                    "Some".to_string(),
                    Some(HirVariantLocation {
                        owner: old_id,
                        variant_id: VariantId(0),
                        name: "Some".to_string(),
                    }),
                    vec![HirPattern::Struct(
                        "Box".to_string(),
                        Some(old_id),
                        Vec::new(),
                        vec![HirStructPatternField {
                            name: "value".to_string(),
                            field: Some(HirFieldLocation {
                                owner: old_id,
                                field_id: FieldId(0),
                                name: "value".to_string(),
                            }),
                            pattern: HirPattern::Wildcard,
                        }],
                    )],
                ),
                guard: None,
                body: HirBlock {
                    stmts: Vec::new(),
                    ty: Type::Unit,
                },
            }],
        },
        ty: Type::Unit,
        span: Span::test(),
    };

    super::remap_expr_location_product_ids(&mut expr, &remap);

    let crate::hir::HirExprKindFor::Match { arms, .. } = &expr.kind else {
        panic!("expected match expression");
    };
    let HirPattern::Enum(_, _, Some(location), payloads) = &arms[0].pattern else {
        panic!("expected enum pattern");
    };
    assert_eq!(location.owner, new_id);
    let HirPattern::Struct(_, Some(struct_id), _, fields) = &payloads[0] else {
        panic!("expected struct pattern");
    };
    assert_eq!(*struct_id, new_id);
    assert_eq!(fields[0].field.as_ref().unwrap().owner, new_id);
}
