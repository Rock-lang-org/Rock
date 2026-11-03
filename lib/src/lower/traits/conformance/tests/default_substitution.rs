use super::*;

#[test]
fn conformance_substitutes_default_method_nested_struct_trait_generic_types() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(73);
    let impl_id = def_id(74);
    let method_id = def_id(75);
    let box_id = def_id(76);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let boxed_trait_generic = Type::Struct {
        id: box_id,
        args: vec![Type::Generic(trait_generic)],
    };
    let boxed_i64 = Type::Struct {
        id: box_id,
        args: vec![Type::I64],
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "MakeBox".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "make".to_string(),
            HirFunction {
                id: method_id,
                name: "make".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(trait_self),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: boxed_trait_generic.clone(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("boxed".to_string()),
                        ty: boxed_trait_generic.clone(),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: boxed_trait_generic.clone(),
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
            receiver_pattern: vec![].into(),
            trait_name: Some("MakeBox".to_string()),
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

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["make"];
    assert_eq!(method.ret_type, boxed_i64);
    assert_eq!(method.body.ty, boxed_i64);
}

#[test]
fn conformance_substitutes_default_method_nested_enum_trait_generic_types() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(77);
    let impl_id = def_id(78);
    let method_id = def_id(79);
    let maybe_id = def_id(80);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let maybe_trait_generic = Type::Enum {
        id: maybe_id,
        args: vec![Type::Generic(trait_generic)],
    };
    let maybe_i64 = Type::Enum {
        id: maybe_id,
        args: vec![Type::I64],
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "MakeMaybe".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "make".to_string(),
            HirFunction {
                id: method_id,
                name: "make".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(trait_self),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: maybe_trait_generic.clone(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("maybe".to_string()),
                        ty: maybe_trait_generic.clone(),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: maybe_trait_generic.clone(),
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
            receiver_pattern: vec![].into(),
            trait_name: Some("MakeMaybe".to_string()),
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

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["make"];
    assert_eq!(method.ret_type, maybe_i64);
    assert_eq!(method.body.ty, maybe_i64);
}

#[test]
fn conformance_substitutes_default_method_call_target_trait_args() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(81);
    let impl_id = def_id(82);
    let method_id = def_id(83);
    let target_method_id = def_id(84);
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
        name: "Caller".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "call".to_string(),
            HirFunction {
                id: method_id,
                name: "call".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(trait_self),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: Type::I64,
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::MethodCall(
                            Box::new(HirExpr {
                                kind: HirExprKind::Var("self".to_string()),
                                ty: Type::Generic(trait_self),
                                span: crate::lexer::Span::test(),
                            }),
                            "target".to_string(),
                            Vec::new(),
                            Some(ReceiverMode::Shared),
                            Some(HirMethodCallTarget::impl_method(
                                impl_id,
                                target_method_id,
                                Some(crate::hir::HirSelectedTraitMember {
                                    trait_id,
                                    member_id: target_method_id,
                                    trait_args: vec![Type::Generic(trait_generic)],
                                }),
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
        signatures: HashMap::new(),
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("I32".to_string()),
            type_name: "I32".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Caller".to_string()),
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

    let HirStmt::Expr(expr) = &lowerer.items.impl_def(impl_id).unwrap().methods["call"]
        .body
        .stmts[0]
    else {
        panic!("default body should contain method call");
    };
    let HirExprKind::MethodCall(_, _, _, _, Some(target)) = &expr.kind else {
        panic!("default body should contain method target");
    };
    assert_eq!(target.trait_args(), &[Type::I64]);
}

#[test]
fn conformance_substitutes_default_method_match_guard_and_pattern_types() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(85);
    let impl_id = def_id(86);
    let method_id = def_id(87);
    let box_id = def_id(88);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let boxed_trait_generic = Type::Struct {
        id: box_id,
        args: vec![Type::Generic(trait_generic)],
    };

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Unwrap".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "unwrap".to_string(),
            HirFunction {
                id: method_id,
                name: "unwrap".to_string(),
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
                        name: "boxed".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: boxed_trait_generic.clone(),
                        mutable: false,
                        is_ref: false,
                    },
                ],
                ret_type: Type::Generic(trait_generic),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Match {
                            scrutinee: Box::new(HirExpr {
                                kind: HirExprKind::Var("boxed".to_string()),
                                ty: boxed_trait_generic.clone(),
                                span: crate::lexer::Span::test(),
                            }),
                            arms: vec![HirMatchArm {
                                pattern: HirPattern::Struct(
                                    "Box".to_string(),
                                    None,
                                    vec![Type::Generic(trait_generic)],
                                    vec![HirStructPatternField {
                                        name: "value".to_string(),
                                        field: None,
                                        pattern: HirPattern::Binding {
                                            name: "value".to_string(),
                                            local_id: crate::ids::HirLocalId(0),
                                            mutable: false,
                                        },
                                    }],
                                ),
                                guard: Some(HirExpr {
                                    kind: HirExprKind::Var("value".to_string()),
                                    ty: Type::Generic(trait_generic),
                                    span: crate::lexer::Span::test(),
                                }),
                                body: HirBlock {
                                    stmts: vec![HirStmt::Expr(HirExpr {
                                        kind: HirExprKind::Var("value".to_string()),
                                        ty: Type::Generic(trait_generic),
                                        span: crate::lexer::Span::test(),
                                    })],
                                    ty: Type::Generic(trait_generic),
                                },
                            }],
                        },
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
            receiver_pattern: vec![].into(),
            trait_name: Some("Unwrap".to_string()),
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

    let HirStmt::Expr(expr) = &lowerer.items.impl_def(impl_id).unwrap().methods["unwrap"]
        .body
        .stmts[0]
    else {
        panic!("default body should contain match expression");
    };
    let HirExprKind::Match { arms, .. } = &expr.kind else {
        panic!("default body should contain match expression");
    };
    let HirPattern::Struct(_, _, type_args, _) = &arms[0].pattern else {
        panic!("match arm should contain struct pattern");
    };
    assert_eq!(type_args, &vec![Type::I64]);
    assert_eq!(arms[0].guard.as_ref().unwrap().ty, Type::I64);
}

#[test]
fn conformance_substitutes_default_method_generic_bound_type_args() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(89);
    let impl_id = def_id(90);
    let method_id = def_id(91);
    let bound_trait_id = def_id(92);
    let method_generic = GenericParamId {
        owner: method_id,
        index: 0,
    };
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
        name: "Factory".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "make".to_string(),
            HirFunction {
                id: method_id,
                name: "make".to_string(),
                generic_params: vec![GenericParamDecl::type_param(method_generic, "U")],
                generic_bounds: HashMap::from([(
                    method_generic,
                    vec![TraitBound {
                        trait_id: bound_trait_id,
                        type_args: vec![Type::Generic(trait_generic)],
                    }],
                )])
                .into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(trait_self),
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
            receiver_pattern: vec![].into(),
            trait_name: Some("Factory".to_string()),
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

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["make"];
    let remapped_method_generic = method.generic_params[0].id;
    assert_eq!(remapped_method_generic.owner, method.id);
    assert_eq!(
        method.generic_bounds[&remapped_method_generic][0].type_args,
        vec![Type::I64]
    );
}

#[test]
fn conformance_substitutes_default_method_lambda_capture_types() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(60);
    let impl_id = def_id(61);
    let default_method_id = def_id(62);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let self_ty = Type::Generic(trait_self);
    let capture_ty = Type::Generic(trait_generic);
    let lambda_ty = Type::function(Vec::new(), capture_ty.clone());

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Factory".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![],
        methods: HashMap::from([(
            "make".to_string(),
            HirFunction {
                id: default_method_id,
                name: "make".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: self_ty.clone(),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: lambda_ty.clone(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Lambda {
                            params: Vec::new(),
                            body: HirBlock {
                                stmts: Vec::new(),
                                ty: capture_ty.clone(),
                            },
                            captures: vec![HirClosureCapture {
                                name: "captured".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                kind: HirClosureCaptureKind::Move,
                                mutable: false,
                                ty: capture_ty.clone(),
                            }],
                        },
                        ty: lambda_ty.clone(),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: lambda_ty.clone(),
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

    let HirStmt::Expr(expr) = &lowerer.items.impl_def(impl_id).unwrap().methods["make"]
        .body
        .stmts[0]
    else {
        panic!("default method body should contain lambda expression");
    };
    let HirExprKind::Lambda { captures, .. } = &expr.kind else {
        panic!("default method body should contain lambda expression");
    };
    assert_eq!(captures[0].ty, Type::I64);
}

#[test]
fn conformance_substitutes_default_method_lambda_projection_carriers() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = def_id(100);
    let impl_id = def_id(101);
    let default_method_id = def_id(102);
    let box_id = def_id(103);
    let trait_generic = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let trait_self = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let self_ty = Type::Generic(trait_self);
    let projection_ty = Type::Projection {
        ty: Box::new(self_ty.clone()),
        trait_id,
        assoc_type: AssociatedTypeKey {
            assoc_type_id: AssocTypeId(0),
            owner: trait_id,
        },
        trait_args: vec![Type::Generic(trait_generic)],
    };
    let impl_output_ty = Type::Struct {
        id: box_id,
        args: vec![Type::I64],
    };
    let lambda_ty = Type::function(vec![projection_ty.clone()], projection_ty.clone());

    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Factory".to_string(),
        generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
        associated_types: vec![HirAssociatedTypeDecl {
            id: AssocTypeId(0),
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::from([(
            "make".to_string(),
            HirFunction {
                id: default_method_id,
                name: "make".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: self_ty.clone(),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: lambda_ty.clone(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Lambda {
                            params: vec![HirParam {
                                name: "value".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                ty: projection_ty.clone(),
                                mutable: false,
                                is_ref: false,
                            }],
                            body: HirBlock {
                                stmts: Vec::new(),
                                ty: projection_ty.clone(),
                            },
                            captures: vec![HirClosureCapture {
                                name: "captured".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                kind: HirClosureCaptureKind::Move,
                                mutable: false,
                                ty: projection_ty.clone(),
                            }],
                        },
                        ty: lambda_ty.clone(),
                        span: crate::lexer::Span::test(),
                    })],
                    ty: lambda_ty.clone(),
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
            receiver_pattern: vec![].into(),
            trait_name: Some("Factory".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "I64",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: vec![HirAssociatedTypeDef {
                id: AssocTypeId(0),
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: impl_output_ty.clone(),
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    lowerer.check_trait_conformance();

    let method = &lowerer.items.impl_def(impl_id).unwrap().methods["make"];
    let expected_lambda_ty = Type::function(vec![impl_output_ty.clone()], impl_output_ty.clone());
    assert_eq!(method.ret_type, expected_lambda_ty);

    let HirStmt::Expr(expr) = &method.body.stmts[0] else {
        panic!("default method body should contain lambda expression");
    };
    assert_eq!(expr.ty, expected_lambda_ty);

    let HirExprKind::Lambda {
        params,
        body,
        captures,
    } = &expr.kind
    else {
        panic!("default method body should contain lambda expression");
    };
    assert_eq!(params[0].ty, impl_output_ty);
    assert_eq!(body.ty, impl_output_ty);
    assert_eq!(captures[0].ty, impl_output_ty);
}
