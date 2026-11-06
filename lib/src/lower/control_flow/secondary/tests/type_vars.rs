use super::*;

fn lowerer_with_type_var_method_bound() -> (Lowerer, Type, DefId, DefId) {
    let mut lowerer = Lowerer::new_for_test();

    let trait_id = DefId::new(CrateId(0), LocalDefId(10));
    let method_id = DefId::new(CrateId(0), LocalDefId(11));
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Wanted".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([(
            "value".to_string(),
            test_function(method_id, "value", Type::I64),
        )]),
        signatures: HashMap::new(),
    });

    let receiver_ty = lowerer.engine.fresh_type_var();
    let Type::TypeVar(var_id) = receiver_ty else {
        panic!("expected fresh type variable");
    };
    lowerer.engine.add_bound(
        var_id,
        TraitBound {
            trait_id,
            type_args: Vec::new(),
        },
    );

    (lowerer, Type::TypeVar(var_id), trait_id, method_id)
}

fn lowerer_with_type_var_signature_bound() -> (Lowerer, Type, DefId, DefId) {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = DefId::new(CrateId(0), LocalDefId(30));
    let signature_id = DefId::new(CrateId(0), LocalDefId(31));
    let self_param = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Readable".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "read".to_string(),
            HirFunctionSig {
                id: signature_id,
                name: "read".to_string(),
                generic_params: vec![GenericParamDecl::type_param(self_param, "Self")],
                params: vec![Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Generic(self_param)),
                }],
                ret: Type::I64,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(crate::types::ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    });

    let receiver_ty = lowerer.engine.fresh_type_var();
    let Type::TypeVar(var_id) = receiver_ty else {
        panic!("expected fresh type variable");
    };
    lowerer.engine.add_bound(
        var_id,
        TraitBound {
            trait_id,
            type_args: Vec::new(),
        },
    );

    (lowerer, Type::TypeVar(var_id), trait_id, signature_id)
}

#[test]
fn type_var_method_call_uses_trait_bound_target_before_name_candidates() {
    let (mut lowerer, receiver_ty, trait_id, method_id) = lowerer_with_type_var_method_bound();
    let field = HirExpr {
        kind: HirExprKind::FieldAccess(
            Box::new(receiver_expr(receiver_ty)),
            "value".to_string(),
            None,
        ),
        ty: lowerer.engine.fresh_type_var(),
        span: Span::test(),
    };

    let lowered = lowerer.apply_secondary(
        field,
        &SecondaryExpr::Arguments(Vec::new()),
        ExprUse::Value,
        false,
    );

    assert!(matches!(lowered.ty, Type::I64));
    match lowered.kind {
        HirExprKind::MethodCall(_, _, _, _, Some(target)) => {
            assert_eq!(target.trait_id(), Some(trait_id));
            assert_eq!(target.method_id(), Some(method_id));
        }
        other => panic!("expected targeted method call, got {other:?}"),
    }
}

#[test]
fn type_var_without_bounds_does_not_select_unowned_method() {
    let mut lowerer = Lowerer::new_for_test();
    let receiver_ty = lowerer.engine.fresh_type_var();
    let field = HirExpr {
        kind: HirExprKind::FieldAccess(
            Box::new(receiver_expr(receiver_ty)),
            "value".to_string(),
            None,
        ),
        ty: lowerer.engine.fresh_type_var(),
        span: Span::test(),
    };

    let lowered = lowerer.apply_secondary(
        field,
        &SecondaryExpr::Arguments(Vec::new()),
        ExprUse::Value,
        false,
    );

    assert!(
        !matches!(lowered.kind, HirExprKind::MethodCall(..)),
        "unconstrained type variable used an unowned method candidate: {:?}",
        lowered.kind
    );
}

#[test]
fn concrete_receiver_does_not_select_method_without_impl_identity() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = DefId::new(CrateId(0), LocalDefId(10));
    lowerer.items.insert_structure(HirStruct {
        id: struct_id,
        name: "Box".to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    });
    let field = HirExpr {
        kind: HirExprKind::FieldAccess(
            Box::new(receiver_expr(Type::Struct {
                id: struct_id,
                args: Vec::new(),
            })),
            "value".to_string(),
            None,
        ),
        ty: lowerer.engine.fresh_type_var(),
        span: Span::test(),
    };

    let lowered = lowerer.apply_secondary(
        field,
        &SecondaryExpr::Arguments(Vec::new()),
        ExprUse::Value,
        false,
    );

    assert!(
        !matches!(lowered.kind, HirExprKind::MethodCall(..)),
        "concrete receiver used a method without impl identity: {:?}",
        lowered.kind
    );
}

#[test]
fn type_var_signature_call_uses_signature_id_target() {
    let (mut lowerer, receiver_ty, trait_id, signature_id) =
        lowerer_with_type_var_signature_bound();
    let field = HirExpr {
        kind: HirExprKind::FieldAccess(
            Box::new(receiver_expr(receiver_ty)),
            "read".to_string(),
            None,
        ),
        ty: lowerer.engine.fresh_type_var(),
        span: Span::test(),
    };

    let lowered = lowerer.apply_secondary(
        field,
        &SecondaryExpr::Arguments(Vec::new()),
        ExprUse::Value,
        false,
    );

    match lowered.kind {
        HirExprKind::MethodCall(_, _, _, _, Some(target)) => {
            assert_eq!(target.trait_id(), Some(trait_id));
            assert_eq!(target.method_id(), Some(signature_id));
            assert_ne!(target.method_id(), Some(trait_id));
        }
        other => panic!("expected signature-backed targeted method call, got {other:?}"),
    }
}

#[test]
fn current_trait_self_signature_call_uses_signature_return_type() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = DefId::new(CrateId(0), LocalDefId(40));
    let signature_id = DefId::new(CrateId(0), LocalDefId(41));
    let invalid_signature_id = DefId::new(CrateId(u32::MAX), LocalDefId(u32::MAX - 1));
    let string_id = DefId::new(CrateId(0), LocalDefId(42));
    let self_param = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let string_ty = Type::Struct {
        id: string_id,
        args: Vec::new(),
    };
    let _trait_frame = lowerer.item_scope.enter_trait(
        "Show".to_string(),
        trait_id,
        trait_id,
        vec!["Self".to_string()],
    );
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Show".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "show".to_string(),
            HirFunctionSig {
                id: invalid_signature_id,
                name: "show".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(self_param)],
                ret: string_ty.clone(),
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(crate::types::ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    });
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Show".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "show".to_string(),
            HirFunctionSig {
                id: signature_id,
                name: "show".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Generic(self_param)],
                ret: string_ty.clone(),
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(crate::types::ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    });
    let field = HirExpr {
        kind: HirExprKind::FieldAccess(
            Box::new(receiver_expr(Type::Generic(self_param))),
            "show".to_string(),
            None,
        ),
        ty: lowerer.engine.fresh_type_var(),
        span: Span::test(),
    };

    let lowered = lowerer.apply_secondary(
        field,
        &SecondaryExpr::Arguments(Vec::new()),
        ExprUse::Value,
        false,
    );

    assert_eq!(lowered.ty, string_ty);
    match lowered.kind {
        HirExprKind::MethodCall(_, _, _, _, Some(target)) => {
            assert_eq!(target.trait_id(), Some(trait_id));
            assert_eq!(target.method_id(), Some(signature_id));
        }
        other => panic!("expected current-trait signature method call, got {other:?}"),
    }
}

#[test]
fn type_var_method_value_uses_trait_bound_target_before_name_candidates() {
    let (mut lowerer, receiver_ty, trait_id, method_id) = lowerer_with_type_var_method_bound();

    let lowered = lowerer.apply_secondary(
        receiver_expr(receiver_ty),
        &SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
            name: "value".to_string(),
            span: Span::test(),
        })),
        ExprUse::Value,
        true,
    );

    assert!(matches!(lowered.ty, Type::Function { ret, .. } if matches!(*ret, Type::I64)));
    match lowered.kind {
        HirExprKind::Lambda { body, .. } => match body.stmts.as_slice() {
            [crate::hir::HirStmt::Expr(HirExpr {
                kind: HirExprKind::MethodCall(_, _, _, _, Some(target)),
                ..
            })] => {
                assert_eq!(target.trait_id(), Some(trait_id));
                assert_eq!(target.method_id(), Some(method_id));
            }
            other => panic!("expected targeted method call body, got {other:?}"),
        },
        other => panic!("expected method-value lambda, got {other:?}"),
    }
}

#[test]
fn type_var_field_access_uses_canonical_struct_name() {
    let mut lowerer = Lowerer::new_for_test();
    let point_id = DefId::new(CrateId(0), LocalDefId(1));
    lowerer.current_def_ids.insert(point_id);
    lowerer.items.insert_structure(HirStruct {
        id: point_id,
        name: "Point".to_string(),
        generic_params: Vec::new(),
        fields: vec![HirField {
            id: FieldId(0),
            name: "x".to_string(),
            ty: Type::I64,
            public: true,
        }],
    });
    lowerer
        .resolver
        .item_names_by_id
        .insert(point_id, "Point".to_string());
    lowerer
        .resolver
        .item_paths
        .insert("Point".to_string(), point_id);

    let base = HirExpr {
        kind: HirExprKind::Var("point".to_string()),
        ty: lowerer.engine.fresh_type_var(),
        span: Span::test(),
    };
    let lowered = lowerer.apply_secondary(
        base,
        &SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
            name: "x".to_string(),
            span: Span::test(),
        })),
        ExprUse::Value,
        false,
    );

    assert_eq!(lowered.ty, Type::I64);
    assert!(lowerer.errors().is_empty());
}

#[test]
fn type_var_field_access_remains_unresolved_when_multiple_structs_match() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.items.insert_structure(HirStruct {
        id: DefId::new(CrateId(0), LocalDefId(1)),
        name: "First".to_string(),
        generic_params: Vec::new(),
        fields: vec![HirField {
            id: FieldId(0),
            name: "value".to_string(),
            ty: Type::I64,
            public: true,
        }],
    });
    lowerer.items.insert_structure(HirStruct {
        id: DefId::new(CrateId(0), LocalDefId(2)),
        name: "Second".to_string(),
        generic_params: Vec::new(),
        fields: vec![HirField {
            id: FieldId(0),
            name: "value".to_string(),
            ty: Type::I64,
            public: true,
        }],
    });

    let base = HirExpr {
        kind: HirExprKind::Var("receiver".to_string()),
        ty: lowerer.engine.fresh_type_var(),
        span: Span::test(),
    };
    let lowered = lowerer.apply_secondary(
        base,
        &SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
            name: "value".to_string(),
            span: Span::test(),
        })),
        ExprUse::Value,
        false,
    );

    assert!(matches!(lowered.ty, Type::TypeVar(_)));
    assert!(lowerer.errors().is_empty());
    match lowered.kind {
        HirExprKind::FieldAccess(_, _, None) => {}
        other => panic!("expected resolved field access, got {other:?}"),
    }
}

#[test]
fn test_borrowed_slice_type_name_match_rejects_concrete_element_mismatch() {
    assert!(!Lowerer::receiver_type_name_matches("&[U8]", "&[I64]"));
    assert!(!Lowerer::receiver_type_name_matches(
        "&mut [U8]",
        "&mut [I64]"
    ));
}

#[test]
fn test_borrowed_slice_type_name_match_allows_generic_candidate_shape() {
    assert!(Lowerer::receiver_type_name_matches("&[T]", "&[U8]"));
    assert!(Lowerer::receiver_type_name_matches("&mut [T]", "&mut [U8]"));
}
