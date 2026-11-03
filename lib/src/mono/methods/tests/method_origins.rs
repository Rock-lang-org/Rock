use super::*;

#[test]
fn monomorphize_same_named_methods_on_different_impls_use_distinct_method_origins() {
    let mut mono = Monomorphizer::new();
    let option_impl_id = DefId::new(CrateId(0), LocalDefId(10));
    let result_impl_id = DefId::new(CrateId(0), LocalDefId(11));
    let option_method_id = DefId::new(CrateId(0), LocalDefId(12));
    let result_method_id = DefId::new(CrateId(0), LocalDefId(13));
    mono.generic_impls.insert(
        option_impl_id,
        generic_map_impl("Option", option_impl_id, option_method_id),
    );
    mono.generic_impls.insert(
        result_impl_id,
        generic_map_impl("Result", result_impl_id, result_method_id),
    );
    seed_generic_enum_receiver_pattern(&mut mono, option_impl_id, "Option");
    seed_generic_enum_receiver_pattern(&mut mono, result_impl_id, "Result");

    let (option_recv, option_func_arg, mut option_expr) = map_expr_for_type("Option");
    if let HirExprKind::MethodCall(_, _, _, _, target) = &mut option_expr.kind {
        *target = selected_map_target(option_impl_id, option_method_id, Type::I64, Type::I64);
    }
    let _ = mono.monomorphize_standalone_method_call(
        "map",
        &[option_recv, option_func_arg],
        &mut option_expr,
    );
    let (result_recv, result_func_arg, mut result_expr) = map_expr_for_type("Result");
    if let HirExprKind::MethodCall(_, _, _, _, target) = &mut result_expr.kind {
        *target = selected_map_target(result_impl_id, result_method_id, Type::I64, Type::I64);
    }
    let _ = mono.monomorphize_standalone_method_call(
        "map",
        &[result_recv, result_func_arg],
        &mut result_expr,
    );

    let origins = mono
        .instances
        .records()
        .map(|record| record.origin.clone())
        .collect::<Vec<_>>();
    assert!(origins.contains(&InstanceOrigin::ImplMethod {
        owner: InstanceImplOwner::Named(option_impl_id),
        method: option_method_id,
    }));
    assert!(origins.contains(&InstanceOrigin::ImplMethod {
        owner: InstanceImplOwner::Named(result_impl_id),
        method: result_method_id,
    }));
    assert_eq!(origins.len(), 2);
}

#[test]
fn monomorphize_same_owner_distinct_methods_use_distinct_method_origins() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(20));
    let map_method_id = DefId::new(CrateId(0), LocalDefId(21));
    let fold_method_id = DefId::new(CrateId(0), LocalDefId(22));
    let mut fold_method = map_method_for_type("Option", impl_id, fold_method_id);
    fold_method.name = "fold".to_string();
    mono.generic_impls.insert(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Option".to_string()),
            type_name: "Option".to_string(),
            type_generics: type_generic(impl_id, "T"),
            receiver_pattern: vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })]
            .into(),
            trait_name: None,
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([
                (
                    "map".to_string(),
                    map_method_for_type("Option", impl_id, map_method_id),
                ),
                ("fold".to_string(), fold_method),
            ]),
        },
    );
    seed_generic_enum_receiver_pattern(&mut mono, impl_id, "Option");

    let (map_recv, map_func_arg, mut map_expr) = map_expr_for_type("Option");
    if let HirExprKind::MethodCall(_, _, _, _, target) = &mut map_expr.kind {
        *target = selected_map_target(impl_id, map_method_id, Type::I64, Type::I64);
    }
    let _ =
        mono.monomorphize_standalone_method_call("map", &[map_recv, map_func_arg], &mut map_expr);
    let (fold_recv, fold_func_arg, mut fold_expr) = map_expr_for_type("Option");
    if let HirExprKind::MethodCall(_, _, _, _, target) = &mut fold_expr.kind {
        *target = selected_map_target(impl_id, fold_method_id, Type::I64, Type::I64);
    }
    let _ = mono.monomorphize_standalone_method_call(
        "fold",
        &[fold_recv, fold_func_arg],
        &mut fold_expr,
    );

    let origins = mono
        .instances
        .records()
        .map(|record| record.origin.clone())
        .collect::<Vec<_>>();
    assert!(origins.contains(&InstanceOrigin::ImplMethod {
        owner: InstanceImplOwner::Named(impl_id),
        method: map_method_id,
    }));
    assert!(origins.contains(&InstanceOrigin::ImplMethod {
        owner: InstanceImplOwner::Named(impl_id),
        method: fold_method_id,
    }));
    assert_eq!(origins.len(), 2);
}

#[test]
fn standalone_method_call_uses_canonical_receiver_pattern_for_qualified_receiver() {
    let mut mono = Monomorphizer::new();
    seed_owner(&mut mono, "Option");
    let impl_id = DefId::new(CrateId(0), LocalDefId(60));
    let method_id = DefId::new(CrateId(0), LocalDefId(61));
    let method = option_map_method(impl_id, method_id);
    mono.generic_impls.insert(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Option".to_string()),
            type_name: "Option".to_string(),
            type_generics: type_generic(impl_id, "T"),
            receiver_pattern: vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })]
            .into(),
            trait_name: None,
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("map".to_string(), method)]),
        },
    );
    seed_generic_enum_receiver_pattern(&mut mono, impl_id, "Option");

    let recv = qualified_option_receiver();
    let func_arg = HirExpr {
        kind: HirExprKind::Var("inc".to_string()),
        ty: Type::function(vec![Type::I64], Type::I64),
        span: Span::test(),
    };
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "map".to_string(),
            vec![func_arg.clone()],
            Some(ReceiverMode::Move),
            selected_map_target(impl_id, method_id, Type::I64, Type::I64),
        ),
        ty: enum_ty("std::option::Option", vec![Type::I64]),
        span: Span::test(),
    };

    let _ = mono.monomorphize_standalone_method_call("map", &[recv, func_arg], &mut expr);

    assert!(matches!(expr.kind, HirExprKind::Call(_, _, _)));
}

#[test]
fn trait_method_call_uses_canonical_receiver_pattern_for_qualified_receiver() {
    let mut mono = Monomorphizer::new();
    seed_owner(&mut mono, "Option");
    let impl_id = DefId::new(CrateId(0), LocalDefId(62));
    let trait_id = DefId::new(CrateId(0), LocalDefId(63));
    let method_id = DefId::new(CrateId(0), LocalDefId(64));
    let method = option_println_method(impl_id, method_id);
    mono.trait_impls.insert(
        trait_id,
        vec![HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Option".to_string()),
            type_name: "Option".to_string(),
            type_generics: type_generic(impl_id, "T"),
            receiver_pattern: vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })]
            .into(),
            trait_name: Some("Show".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("println".to_string(), method)]),
        }],
    );
    seed_generic_enum_receiver_pattern(&mut mono, impl_id, "Option");

    let recv = qualified_option_receiver();
    let mut selected_target = selected_impl_target(impl_id, method_id, Type::I64);
    if let crate::hir::HirSelectedMethodTarget::ImplMethod { selected_trait, .. } =
        &mut selected_target.target
    {
        *selected_trait = Some(crate::hir::HirSelectedTraitMember {
            trait_id,
            member_id: method_id,
            trait_args: Vec::new(),
        });
    }
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "println".to_string(),
            vec![],
            Some(ReceiverMode::Move),
            selected_target,
        ),
        ty: Type::I32,
        span: Span::test(),
    };

    let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

    assert!(matches!(expr.kind, HirExprKind::Call(_, _, _)));
}

#[test]
fn specialize_impl_method_preserves_concrete_nominal_self_type_without_impl_args() {
    let mut mono = Monomorphizer::new();
    let foo_id = def_id(77);
    let method = HirFunction {
        id: def_id(78),
        name: "id".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(0),
                index: 0,
            },
            "U",
        )],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Struct {
                id: foo_id,
                args: Vec::new(),
            },
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::Var("self".to_string()),
                ty: Type::Generic(crate::types::GenericParamId {
                    owner: crate::ids::DefId::new(
                        crate::ids::CrateId(0),
                        crate::ids::LocalDefId(0),
                    ),
                    index: 0,
                }),
                span: Span::test(),
            })],
            ty: Type::I64,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };

    let i64_id = mono.intern_type(&Type::I64);
    let specialized = mono.specialize_impl_method(
        &method,
        &[i64_id],
        &[],
        "Foo_id_I64",
        DefId::new(CrateId(0), LocalDefId(0)),
        None,
    );

    let HirStmt::Expr(HirExpr { ty, .. }) = &specialized.body.stmts[0] else {
        panic!("expected self expression");
    };
    assert_eq!(
        ty,
        &Type::Struct {
            id: foo_id,
            args: Vec::new(),
        }
    );
}

#[test]
fn specialize_impl_method_preserves_nested_nominal_receiver_shape() {
    let mut mono = Monomorphizer::new();
    let impl_id = def_id(79);
    let option_id = def_id(80);
    let generic = Type::Generic(crate::types::GenericParamId {
        owner: impl_id,
        index: 0,
    });
    let receiver = Type::Enum {
        id: option_id,
        args: vec![Type::Enum {
            id: option_id,
            args: vec![generic],
        }],
    };
    let method = HirFunction {
        id: def_id(81),
        name: "flatten".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: receiver.clone(),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::Var("self".to_string()),
                ty: receiver,
                span: Span::test(),
            })],
            ty: Type::I64,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };
    let i64_id = mono.intern_type(&Type::I64);

    let specialized = mono.specialize_impl_method(
        &method,
        &[i64_id],
        &[GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        "Option_Option_I64_flatten",
        impl_id,
        None,
    );
    let expected = Type::Enum {
        id: option_id,
        args: vec![Type::Enum {
            id: option_id,
            args: vec![Type::I64],
        }],
    };
    let HirStmt::Expr(HirExpr { ty, .. }) = &specialized.body.stmts[0] else {
        panic!("expected self expression");
    };

    assert_eq!(specialized.params[0].ty, expected);
    assert_eq!(ty, &expected);
}
