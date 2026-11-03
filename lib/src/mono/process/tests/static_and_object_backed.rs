use super::*;

fn generic_static_impl_for_process_test(
    type_name: &str,
    impl_id: DefId,
    method_id: DefId,
) -> HirImpl {
    let generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named(type_name.to_string()),
        type_name: type_name.to_string(),
        type_generics: vec![GenericParamDecl::type_param(generic, "T")],
        receiver_pattern: vec![Type::Generic(generic)].into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "make".to_string(),
            HirFunction {
                id: method_id,
                name: "make".to_string(),
                generic_params: Vec::new(),
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "value".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(generic),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: Type::Generic(generic),
                body: HirBlock {
                    stmts: Vec::new(),
                    ty: Type::Generic(generic),
                },
                is_curried: false,
                is_method: false,
                self_receiver: None,
                is_unsafe: false,
            },
        )]),
    }
}

#[test]
fn process_object_backed_concrete_function_call_uses_registered_instance() {
    let mut mono = Monomorphizer::new();
    let answer_id = DefId::new(CrateId(1), LocalDefId(994));
    let answer = callback_acceptor_for_process_test(answer_id, "answer", Type::Unit);
    let key = InstanceKey::new(InstanceOrigin::Function(answer_id), Vec::new());
    let instance_id = mono.instances.intern(key.clone(), |id| InstanceRecord {
        id,
        origin: key.origin.clone(),
        substitution: key.substitution.clone(),
        symbols: InstanceSymbols::new("dep::answer", "dep::answer"),
        declared: Some(answer.clone()),
        provided_by_object: true,
        is_specialization: false,
    });
    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "answer".to_string(),
                    target: HirVarTarget::Function(answer_id),
                }),
                ty: Type::function(Vec::new(), Type::I64),
                span: crate::lexer::Span::test(),
            }),
            Vec::new(),
            Some(HirCallTarget::Function(answer_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("callee should remain resolved");
    };
    assert_eq!(reference.target, HirVarTarget::Instance(instance_id));
    assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
}

#[test]
fn process_object_backed_static_impl_method_call_uses_registered_instance() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(1), LocalDefId(995));
    let method_id = DefId::new(CrateId(1), LocalDefId(996));
    let method = callback_acceptor_for_process_test(method_id, "dealloc", Type::Unit);
    let key = InstanceKey::new(
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: method_id,
        },
        Vec::new(),
    );
    let instance_id = mono.instances.intern(key.clone(), |id| InstanceRecord {
        id,
        origin: key.origin.clone(),
        substitution: key.substitution.clone(),
        symbols: InstanceSymbols::new("__rock_Global_dealloc", "__rock_Global_dealloc"),
        declared: Some(method.clone()),
        provided_by_object: true,
        is_specialization: false,
    });
    mono.generic_impls.insert(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: crate::hir::HirImplOwner::Named("Global".to_string()),
            type_name: "Global".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("dealloc".to_string(), method)]),
        },
    );
    mono.set_impl_receiver_pattern_for_test(
        impl_id,
        crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
    );

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "dealloc".to_string(),
                    target: HirVarTarget::Function(method_id),
                }),
                ty: Type::function(Vec::new(), Type::I64),
                span: crate::lexer::Span::test(),
            }),
            Vec::new(),
            Some(HirCallTarget::StaticMethod(
                crate::hir::HirStaticMethodTarget {
                    owner_ty: Type::Unit,
                    method: HirMethodCallTarget::impl_method(impl_id, method_id, None),
                },
            )),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("callee should remain resolved");
    };
    assert_eq!(reference.target, HirVarTarget::Instance(instance_id));
    assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
}

#[test]
fn process_instance_callee_does_not_fall_back_to_static_impl_name_lookup() {
    let mut mono = Monomorphizer::new();
    let free_id = DefId::new(CrateId(0), LocalDefId(974));
    let impl_id = DefId::new(CrateId(0), LocalDefId(975));
    let method_id = DefId::new(CrateId(0), LocalDefId(976));
    let free_function = generic_identity_for_process_test(free_id, "Box_make");

    mono.register_generic_function("Box_make".to_string(), free_function);
    mono.generic_impls.insert(
        impl_id,
        generic_static_impl_for_process_test("Box", impl_id, method_id),
    );
    mono.resolver
        .item_paths
        .insert("Box_make".to_string(), free_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "Box_make".to_string(),
                    target: HirVarTarget::Function(free_id),
                }),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::IntLiteral(42),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(free_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("callee should be instance-backed");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("callee should target a monomorphized instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
    assert_eq!(record.origin, InstanceOrigin::Function(free_id));
    assert_ne!(
        record.origin,
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: method_id,
        }
    );
}

#[test]
fn process_static_impl_function_target_is_not_materialized_without_authority() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(950));
    let method_id = DefId::new(CrateId(0), LocalDefId(951));
    let colliding_function_id = DefId::new(CrateId(0), LocalDefId(952));
    let generic_id = GenericParamId {
        owner: colliding_function_id,
        index: 0,
    };
    mono.generic_impls.insert(
        impl_id,
        generic_static_impl_for_process_test("Box", impl_id, method_id),
    );
    mono.generic_functions.insert(
        colliding_function_id,
        HirFunction {
            id: colliding_function_id,
            name: "wrong_alias".to_string(),
            generic_params: vec![GenericParamDecl::type_param(generic_id, "T")],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(generic_id),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Generic(generic_id),
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Generic(generic_id),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        },
    );
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), colliding_function_id);
    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Function(method_id),
                }),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::IntLiteral(7),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(method_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("static impl call should remain a call");
    };
    assert!(matches!(
        &callee.kind,
        HirExprKind::ResolvedVar(HirVarRef {
            target: HirVarTarget::Function(id),
            ..
        }) if *id == method_id
    ));
    assert_eq!(target, &Some(HirCallTarget::Function(method_id)));
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn process_static_impl_function_value_is_not_materialized_without_authority() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(960));
    let method_id = DefId::new(CrateId(0), LocalDefId(961));
    mono.generic_impls.insert(
        impl_id,
        generic_static_impl_for_process_test("Box", impl_id, method_id),
    );
    let mut expr = HirExpr {
        kind: HirExprKind::ResolvedVar(HirVarRef {
            name: "Box_make".to_string(),
            target: HirVarTarget::Function(method_id),
        }),
        ty: Type::function(vec![Type::I64], Type::I64),
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::ResolvedVar(reference) = &expr.kind else {
        panic!("static impl function value should remain a resolved var");
    };
    assert_eq!(reference.target, HirVarTarget::Function(method_id));
    assert!(mono.instances.records().next().is_none());
    assert_eq!(expr.ty, Type::function(vec![Type::I64], Type::I64));
}

#[test]
fn process_call_does_not_resolve_callee_by_backend_symbol() {
    let mut mono = Monomorphizer::new();
    let apply_id = DefId::new(CrateId(0), LocalDefId(915));
    let identity_id = DefId::new(CrateId(0), LocalDefId(916));
    let generic_id = GenericParamId {
        owner: identity_id,
        index: 0,
    };
    let generic_identity = HirFunction {
        id: identity_id,
        name: "identity".to_string(),
        generic_params: vec![GenericParamDecl::type_param(generic_id, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(generic_id),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(generic_id),
        body: HirBlock {
            stmts: vec![HirStmt::Return(Some(HirExpr {
                kind: HirExprKind::Var("value".to_string()),
                ty: Type::Generic(generic_id),
                span: crate::lexer::Span::test(),
            }))],
            ty: Type::Generic(generic_id),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    mono.register_generic_function("identity".to_string(), generic_identity);
    mono.resolver
        .item_paths
        .insert("identity".to_string(), identity_id);

    let apply_substitution = mono.intern_types(&[Type::I64]);
    let apply_key =
        crate::mono::InstanceKey::new(InstanceOrigin::Function(apply_id), apply_substitution);
    let apply_body = HirFunction {
        id: apply_id,
        name: "apply".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "callback".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::function(vec![Type::I64], Type::I64),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let apply_instance =
        mono.instances
            .intern(apply_key.clone(), |id| crate::mono::InstanceRecord {
                id,
                origin: apply_key.origin.clone(),
                substitution: apply_key.substitution.clone(),
                symbols: crate::mono::InstanceSymbols::new("apply", "apply_mono_0"),
                declared: None,
                provided_by_object: false,
                is_specialization: true,
            });
    mono.instances
        .insert_pre_mir_body(apply_instance, apply_body);
    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::Var("apply_mono_0".to_string()),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::Var("identity".to_string()),
                ty: Type::function(vec![Type::Generic(generic_id)], Type::Generic(generic_id)),
                span: crate::lexer::Span::test(),
            }],
            None,
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    assert_eq!(mono.instances.len(), 1);
    assert!(
        matches!(&args[0].kind, HirExprKind::Var(name) if name == "identity"),
        "backend symbols must not recover instance bodies for argument specialization"
    );
}
