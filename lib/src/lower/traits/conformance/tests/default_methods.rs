use super::*;

#[test]
fn conformance_preserves_empty_override_without_default_injection() {
    let mut lowerer = Lowerer::new_for_test();

    let trait_id = def_id(6);
    let impl_id = def_id(7);
    let default_method_id = def_id(8);
    let override_method_id = def_id(9);

    let default_method = HirFunction {
        id: default_method_id,
        name: "print".to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(GenericParamId {
                owner: trait_id,
                index: 0,
            }),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I32,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::IntLiteral(42),
                ty: Type::I32,
                span: crate::lexer::Span::test(),
            })],
            ty: Type::I32,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Shared),
        is_unsafe: false,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Printable".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::from([("print".to_string(), default_method.clone())]),
        signatures: HashMap::from([(
            "print".to_string(),
            HirFunctionSig {
                id: def_id(41),
                name: "print".to_string(),
                generic_params: vec![],
                params: vec![Type::Generic(GenericParamId {
                    owner: trait_id,
                    index: 0,
                })],
                ret: Type::I32,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Printable".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "print".to_string(),
                HirFunction {
                    id: override_method_id,
                    name: "print".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I64,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::I32,
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Unit,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["print"];
    assert_eq!(method.id, override_method_id);
    assert_eq!(method.params[0].ty, Type::I64);
    assert_eq!(method.ret_type, Type::I32);
    assert!(method.body.stmts.is_empty());
}

#[test]
fn conformance_resolves_signatureless_default_method_return_type_from_body() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(50);
    let impl_id = def_id(51);
    let default_method_id = def_id(52);
    let ret_ty = lowerer.engine.fresh_type_var();

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Animal".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::from([(
            "legs".to_string(),
            HirFunction {
                id: default_method_id,
                name: "legs".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(GenericParamId {
                        owner: trait_id,
                        index: 0,
                    }),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: ret_ty,
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::IntLiteral(4),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::I64,
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Dog".to_string()),
            type_name: "Dog".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Animal".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    assert_eq!(
        lowerer.items.impl_def(impl_id).unwrap().methods["legs"].ret_type,
        Type::I64
    );
}

#[test]
fn conformance_does_not_retarget_default_method_bound_non_self_receiver() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(113);
    let impl_id = def_id(114);
    let score_sig_id = def_id(115);
    let default_method_id = def_id(116);
    let impl_score_method_id = def_id(117);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let method_generic = GenericParamId {
        owner: default_method_id,
        index: 0,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Metric".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::from([(
            "score_other".to_string(),
            HirFunction {
                id: default_method_id,
                name: "score_other".to_string(),
                generic_params: vec![GenericParamDecl::type_param(method_generic, "T")],
                generic_bounds: HashMap::from([(
                    method_generic,
                    vec![TraitBound {
                        trait_id,
                        type_args: vec![],
                    }],
                )])
                .into(),
                params: vec![
                    HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    },
                    HirParam {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(1),
                        ty: Type::Generic(method_generic),
                        mutable: false,
                        is_ref: false,
                    },
                ],
                ret_type: Type::I64,
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::MethodCall(
                            Box::new(HirExpr {
                                kind: HirExprKind::Var("value".to_string()),
                                ty: Type::Generic(method_generic),
                                span: crate::lexer::Span::test(),
                            }),
                            "score".to_string(),
                            Vec::new(),
                            Some(ReceiverMode::Shared),
                            Some(HirMethodCallTarget::trait_method(
                                trait_id,
                                score_sig_id,
                                Vec::new(),
                                crate::hir::HirTraitDispatchKind::CurrentTrait,
                            )),
                        ),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::I64,
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::from([(
            "score".to_string(),
            HirFunctionSig {
                id: score_sig_id,
                name: "score".to_string(),
                generic_params: vec![],
                params: vec![Type::Generic(trait_self)],
                ret: Type::I64,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Metric".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "score".to_string(),
                HirFunction {
                    id: impl_score_method_id,
                    name: "score".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I32,
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::I64,
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::IntLiteral(1),
                            ty: Type::I64,
                            span: crate::lexer::Span::test(),
                        })],
                        ty: Type::I64,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let HirStmt::Expr(expr) = &lowerer.items.impl_def(impl_id).unwrap().methods["score_other"]
        .body
        .stmts[0]
    else {
        panic!("default body should contain method call");
    };
    let HirExprKind::MethodCall(_, _, _, _, Some(target)) = &expr.kind else {
        panic!("default body should contain method target");
    };
    assert_eq!(target.impl_id(), None);
    assert_eq!(target.method_id(), Some(score_sig_id));
}

#[test]
fn conformance_default_body_generic_ids_match_trait_header_order() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(95);
    let impl_id = def_id(96);
    let method_id = def_id(97);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Identity".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "identity".to_string(),
            HirFunction {
                id: method_id,
                name: "identity".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![
                    HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    },
                    HirParam {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_generic),
                        mutable: false,
                        is_ref: false,
                    },
                ],
                ret_type: Type::Generic(trait_generic),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("value".to_string()),
                        ty: Type::Generic(trait_generic),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::Generic(trait_generic),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::I32),
            trait_name: Some("Identity".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["identity"];
    assert_eq!(method.params[0].ty, Type::I32);
    assert_eq!(method.params[1].ty, Type::I64);
    assert_eq!(method.ret_type, Type::I64);
    assert_eq!(method.body.ty, Type::I64);
    let HirStmt::Expr(expr) = &method.body.stmts[0] else {
        panic!("default body should contain expression");
    };
    assert_eq!(expr.ty, Type::I64);
}

#[test]
fn generated_default_method_is_attached_to_exact_impl_owner_and_remaps_generics() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(110);
    let impl_id = def_id(111);
    let default_method_id = def_id(112);
    let bound_trait_id = def_id(114);
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let default_method_generic = GenericParamId {
        owner: default_method_id,
        index: 0,
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Factory".to_string(),
        generic_params: vec![],
        associated_types: vec![],
        methods: HashMap::from([(
            "id".to_string(),
            HirFunction {
                id: default_method_id,
                name: "id".to_string(),
                generic_params: vec![GenericParamDecl::type_param(default_method_generic, "U")],
                generic_bounds: HashMap::from([(
                    default_method_generic,
                    vec![TraitBound {
                        trait_id: bound_trait_id,
                        type_args: vec![Type::Generic(default_method_generic)],
                    }],
                )])
                .into(),
                params: vec![
                    HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    },
                    HirParam {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(default_method_generic),
                        mutable: false,
                        is_ref: false,
                    },
                ],
                ret_type: Type::Generic(default_method_generic),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("value".to_string()),
                        ty: Type::Generic(default_method_generic),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: Type::Generic(default_method_generic),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I64".to_string()),
            type_name: "I64".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Factory".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["id"];
    assert_ne!(method.id, default_method_id);
    assert!(lowerer.current_def_ids.contains(&method.id));
    let remapped_method_generic = GenericParamId {
        owner: method.id,
        index: 0,
    };
    assert_eq!(method.generic_params[0].id, remapped_method_generic);
    assert!(method.generic_bounds.contains_key(&remapped_method_generic));
    assert!(!method.generic_bounds.contains_key(&default_method_generic));
    assert_eq!(method.params[1].ty, Type::Generic(remapped_method_generic));
    assert_eq!(method.ret_type, Type::Generic(remapped_method_generic));
    assert_eq!(method.body.ty, Type::Generic(remapped_method_generic));
    assert_eq!(
        method.generic_bounds[&remapped_method_generic][0].type_args,
        vec![Type::Generic(remapped_method_generic)]
    );
    let HirStmt::Expr(expr) = &method.body.stmts[0] else {
        panic!("default body should contain expression");
    };
    assert_eq!(expr.ty, Type::Generic(remapped_method_generic));
}
