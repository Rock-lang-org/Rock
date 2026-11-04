use super::*;

fn refresh_resolved_type_ids(resolved: &mut ResolvedHirProgram) {
    resolved.type_context = crate::type_context::TypeContext::new();
    resolved.type_ids =
        crate::hir::collect_hir_type_ids(&resolved.program, &mut resolved.type_context);
}

#[test]
fn serialized_product_artifact_roundtrips_function_signature_types() {
    let resolved = resolved_hir_for_products();
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let roundtrip =
        CompilerProducts::from_artifact_bytes(&products.to_artifact_bytes().unwrap()).unwrap();
    let function = roundtrip
        .interface
        .functions
        .values()
        .find(|function| function.name == "identity")
        .unwrap();

    assert_eq!(function.params[0], Type::I64);
    assert_eq!(function.ret_type, Type::I64);
}

#[test]
fn serialized_product_artifact_roundtrips_type_alias_identity_and_body() {
    let mut resolved = resolved_hir_for_products();
    let alias_id = DefId::new(resolved.root_crate_id, LocalDefId(200));
    let alias = crate::hir::HirTypeAlias {
        id: alias_id,
        name: "Identity".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: alias_id,
                index: 0,
            },
            "T",
        )],
        ty: Type::Lambda {
            params: vec![crate::type_services::kind::Kind::Type],
            body: Box::new(Type::BoundVar {
                depth: 0,
                index: 0,
                kind: crate::type_services::kind::Kind::Type,
            }),
        },
    };
    mutate_program(&mut resolved.program, |program| {
        program.type_aliases.insert(alias_id, alias.clone());
        program.rebuild_indexes();
    });
    resolved.current_def_ids.insert(alias_id);

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .unwrap();
    let roundtrip =
        CompilerProducts::from_artifact_bytes(&products.to_artifact_bytes().unwrap()).unwrap();
    let product_id = ProductDefId::from(alias_id);
    let roundtrip_alias = roundtrip.interface.type_aliases.get(&product_id).unwrap();

    assert_eq!(roundtrip_alias.id, alias_id);
    assert_eq!(roundtrip_alias.name, "Identity");
    assert_eq!(roundtrip_alias.generic_params, alias.generic_params);
    assert_eq!(roundtrip_alias.ty, alias.ty);
}

#[test]
fn serialized_product_artifact_roundtrips_struct_enum_trait_impl_and_extern_types() {
    let mut resolved = resolved_hir_for_products();
    let field_ty = Type::Pointer(Box::new(Type::U32));
    let variant_ty = Type::Array(Box::new(Type::Bool), 3);
    mutate_program(&mut resolved.program, |program| {
        program
            .structs
            .values_mut()
            .find(|strukt| strukt.name == "Box")
            .unwrap()
            .fields
            .push(crate::hir::HirField {
                id: FieldId(99),
                name: "payload".to_string(),
                ty: field_ty.clone(),
                public: true,
            });
        program
            .enums
            .values_mut()
            .find(|enm| enm.name == "Maybe")
            .unwrap()
            .variants
            .push(crate::hir::HirVariant {
                id: VariantId(99),
                name: "Some".to_string(),
                fields: crate::hir::HirVariantFields::Positional(vec![variant_ty.clone()]),
            });
    });
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let bytes = products.to_artifact_bytes().unwrap();
    let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();

    let strukt = roundtrip
        .interface
        .structs
        .values()
        .find(|strukt| strukt.name == "Box")
        .unwrap();
    assert_eq!(strukt.fields[0].ty, field_ty);
    let enm = roundtrip
        .interface
        .enums
        .values()
        .find(|enm| enm.name == "Maybe")
        .unwrap();
    match &enm.variants[0].fields {
        crate::hir::HirVariantFields::Positional(types) => {
            assert_eq!(types.as_slice(), &[variant_ty.clone()]);
        }
        other => panic!("expected positional variant fields, got {other:?}"),
    }
    assert!(
        super::super::type_table::tests::artifact_type_table_contains_type_for_test(
            &bytes, &field_ty
        )
        .unwrap()
    );
    assert!(
        super::super::type_table::tests::artifact_type_table_contains_type_for_test(
            &bytes,
            &variant_ty
        )
        .unwrap()
    );
    assert!(roundtrip
        .interface
        .traits
        .values()
        .any(|trait_def| trait_def.name == "demo::Show"));
    assert!(roundtrip.interface.impls.values().any(|imp| !matches!(
        imp.receiver_pattern,
        crate::hir::HirImplReceiverPattern::Exact(Type::Unit)
    )));
    assert!(roundtrip
        .interface
        .externs
        .values()
        .any(|ext| ext.ret == crate::types::Type::I32));
}

#[test]
fn serialized_product_artifact_roundtrips_constructor_impl_target() {
    let mut resolved = resolved_hir_for_products();
    let mut expected = None;
    mutate_program(&mut resolved.program, |program| {
        let box_id = program
            .structs
            .values()
            .find(|structure| structure.name == "Box")
            .unwrap()
            .id;
        let impl_generic = program.impls.values().next().unwrap().type_generics[0].id;
        let target = Type::Lambda {
            params: vec![crate::type_services::kind::Kind::Type],
            body: Box::new(Type::Struct {
                id: box_id,
                args: vec![Type::Tuple(vec![
                    Type::Generic(impl_generic),
                    Type::BoundVar {
                        depth: 0,
                        index: 0,
                        kind: crate::type_services::kind::Kind::Type,
                    },
                ])],
            }),
        };
        program.impls.values_mut().next().unwrap().receiver_pattern =
            crate::hir::HirImplReceiverPattern::Constructor(target.clone());
        expected = Some(target);
    });
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let roundtrip =
        CompilerProducts::from_artifact_bytes(&products.to_artifact_bytes().unwrap()).unwrap();

    assert!(roundtrip.interface.impls.values().any(|imp| {
        imp.receiver_pattern
            == crate::hir::HirImplReceiverPattern::Constructor(expected.clone().unwrap())
    }));
}

#[test]
fn compiler_products_type_table_roundtrips_nested_body_type_locations() {
    let mut resolved = resolved_hir_for_products();
    let block_ty = Type::Array(Box::new(Type::U8), 4);
    let let_ty = Type::Tuple(vec![Type::U16, Type::Bool]);
    let nested_expr_ty = Type::Pointer(Box::new(Type::Bool));
    let cast_ty = Type::Array(Box::new(Type::U32), 2);
    let lambda_param_ty = Type::Pointer(Box::new(Type::F32));
    let lambda_capture_ty = Type::Reference {
        mutable: true,
        inner: Box::new(Type::U64),
    };
    let lambda_body_ty = Type::Slice(Box::new(Type::Char));
    let lambda_expr_ty = Type::function(vec![lambda_param_ty.clone()], lambda_body_ty.clone());
    mutate_program(&mut resolved.program, |program| {
        let identity = program
            .functions
            .values_mut()
            .find(|function| function.name == "identity")
            .unwrap();
        identity.body = crate::hir::HirBlockFor::<AcceptedHir> {
            stmts: vec![
                crate::hir::HirStmtFor::Let {
                    name: "converted".to_string(),
                    local_id: crate::ids::HirLocalId(11),
                    ty: let_ty.clone(),
                    value: crate::hir::HirExprFor::<AcceptedHir> {
                        kind: crate::hir::HirExprKindFor::Cast(
                            Box::new(crate::hir::HirExprFor::<AcceptedHir> {
                                kind: crate::hir::HirExprKindFor::IntLiteral(1),
                                ty: nested_expr_ty.clone(),
                                span: Span::test(),
                            }),
                            cast_ty.clone(),
                        ),
                        ty: cast_ty.clone(),
                        span: Span::test(),
                    },
                    mutable: false,
                },
                crate::hir::HirStmtFor::Expr(crate::hir::HirExprFor::<AcceptedHir> {
                    kind: crate::hir::HirExprKindFor::Lambda {
                        params: vec![HirParam {
                            name: "arg".to_string(),
                            local_id: crate::ids::HirLocalId(12),
                            ty: lambda_param_ty.clone(),
                            mutable: false,
                            is_ref: false,
                        }],
                        body: crate::hir::HirBlockFor::<AcceptedHir> {
                            stmts: vec![crate::hir::HirStmtFor::Expr(crate::hir::HirExprFor::<
                                AcceptedHir,
                            > {
                                kind: crate::hir::HirExprKindFor::Unit,
                                ty: lambda_body_ty.clone(),
                                span: Span::test(),
                            })],
                            ty: lambda_body_ty.clone(),
                        },
                        captures: vec![HirClosureCapture {
                            name: "captured".to_string(),
                            local_id: crate::ids::HirLocalId(13),
                            kind: HirClosureCaptureKind::MutableBorrow,
                            mutable: true,
                            ty: lambda_capture_ty.clone(),
                        }],
                    },
                    ty: lambda_expr_ty.clone(),
                    span: Span::test(),
                }),
            ],
            ty: block_ty.clone(),
        };
    });
    refresh_resolved_type_ids(&mut resolved);
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let bytes = products.to_artifact_bytes().unwrap();
    let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();
    let function = roundtrip
        .bodies
        .functions
        .values()
        .find(|function| function.name == "identity")
        .unwrap();

    assert_eq!(function.body.ty, block_ty);
    let crate::hir::HirStmtFor::Let { ty, value, .. } = &function.body.stmts[0] else {
        panic!("expected let statement");
    };
    assert_eq!(ty, &let_ty);
    assert_eq!(value.ty, cast_ty);
    let crate::hir::HirExprKindFor::Cast(inner, target_ty) = &value.kind else {
        panic!("expected cast expression");
    };
    assert_eq!(inner.ty, nested_expr_ty);
    assert_eq!(target_ty, &cast_ty);
    let crate::hir::HirStmtFor::Expr(lambda) = &function.body.stmts[1] else {
        panic!("expected lambda expression statement");
    };
    assert_eq!(lambda.ty, lambda_expr_ty);
    let crate::hir::HirExprKindFor::Lambda {
        params,
        body,
        captures,
    } = &lambda.kind
    else {
        panic!("expected lambda expression");
    };
    assert_eq!(params[0].ty, lambda_param_ty);
    assert_eq!(captures[0].ty, lambda_capture_ty);
    assert_eq!(body.ty, lambda_body_ty);

    for expected in [
        &let_ty,
        &cast_ty,
        &lambda_param_ty,
        &lambda_capture_ty,
        &lambda_body_ty,
    ] {
        assert!(
            super::super::type_table::tests::artifact_type_table_contains_type_for_test(
                &bytes, expected
            )
            .unwrap(),
            "artifact type table should contain body-only type {expected:?}"
        );
    }
}

#[test]
fn compiler_products_type_table_roundtrips_struct_pattern_type_args() {
    let mut resolved = resolved_hir_for_products();
    let pattern_arg_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::U16),
    };
    mutate_program(&mut resolved.program, |program| {
        let identity = program
            .functions
            .values_mut()
            .find(|function| function.name == "identity")
            .unwrap();
        identity.body = crate::hir::HirBlockFor::<AcceptedHir> {
            stmts: vec![crate::hir::HirStmtFor::Expr(crate::hir::HirExprFor::<
                AcceptedHir,
            > {
                kind: crate::hir::HirExprKindFor::Match {
                    scrutinee: Box::new(crate::hir::HirExprFor::<AcceptedHir> {
                        kind: crate::hir::HirExprKindFor::Var("value".to_string()),
                        ty: Type::I64,
                        span: Span::test(),
                    }),
                    arms: vec![crate::hir::HirMatchArmFor::<AcceptedHir> {
                        pattern: HirPattern::Struct(
                            "Box".to_string(),
                            None,
                            vec![pattern_arg_ty.clone()],
                            vec![HirStructPatternField {
                                name: "value".to_string(),
                                field: None,
                                pattern: HirPattern::Wildcard,
                            }],
                        ),
                        guard: None,
                        body: crate::hir::HirBlockFor::<AcceptedHir> {
                            stmts: Vec::new(),
                            ty: Type::Unit,
                        },
                    }],
                },
                ty: Type::Unit,
                span: Span::test(),
            })],
            ty: Type::Unit,
        };
    });
    refresh_resolved_type_ids(&mut resolved);
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let bytes = products.to_artifact_bytes().unwrap();
    let roundtrip = CompilerProducts::from_artifact_bytes(&bytes).unwrap();
    let function = roundtrip
        .bodies
        .functions
        .values()
        .find(|function| function.name == "identity")
        .unwrap();
    let crate::hir::HirStmtFor::Expr(expr) = &function.body.stmts[0] else {
        panic!("expected match expression statement");
    };
    let crate::hir::HirExprKindFor::Match { arms, .. } = &expr.kind else {
        panic!("expected match expression");
    };
    let HirPattern::Struct(_, _, type_args, _) = &arms[0].pattern else {
        panic!("expected struct pattern");
    };

    assert_eq!(type_args, &vec![pattern_arg_ty.clone()]);
    assert!(
        super::super::type_table::tests::artifact_type_table_contains_type_for_test(
            &bytes,
            &pattern_arg_ty
        )
        .unwrap()
    );
}

#[test]
fn compiler_products_serialize_structural_types_as_explicit_compatibility_boundary() {
    let resolved = resolved_hir_for_products();
    let identity_id = DefId::new(CrateId(0), LocalDefId(1));
    let ret_id = resolved
        .type_id_at(&crate::hir::HirTypeLocation::FunctionReturn {
            function: identity_id,
        })
        .unwrap();

    assert_eq!(resolved.type_at(ret_id), Type::I64);

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &resolved,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let roundtrip =
        CompilerProducts::from_artifact_bytes(&products.to_artifact_bytes().unwrap()).unwrap();
    let function = roundtrip
        .interface
        .functions
        .values()
        .find(|function| function.name == "identity")
        .unwrap();

    assert_eq!(function.ret_type, Type::I64);
    assert_eq!(function.params[0], Type::I64);
}
