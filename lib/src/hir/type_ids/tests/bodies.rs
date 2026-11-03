use super::*;

fn single_type_arg_bound(trait_id: DefId, ty: Type) -> TraitBound {
    TraitBound {
        trait_id,
        type_args: vec![ty],
    }
}

#[test]
fn hir_type_ids_collect_body_type_locations() {
    let function_id = def_id(20);
    let capture_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };
    let lambda_ty = Type::function(vec![Type::I64], Type::I64);
    let function = HirFunction {
        id: function_id,
        name: "main".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: vec![
                crate::hir::HirStmt::Let {
                    name: "x".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::I64,
                    value: crate::hir::HirExpr {
                        kind: crate::hir::HirExprKind::IntLiteral(1),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    },
                    mutable: false,
                },
                crate::hir::HirStmt::Expr(crate::hir::HirExpr {
                    kind: crate::hir::HirExprKind::Lambda {
                        params: vec![HirParam {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I64,
                            mutable: false,
                            is_ref: false,
                        }],
                        body: HirBlock {
                            stmts: vec![crate::hir::HirStmt::Expr(crate::hir::HirExpr {
                                kind: crate::hir::HirExprKind::Var("value".to_string()),
                                ty: Type::I64,
                                span: crate::lexer::Span::test(),
                            })],
                            ty: Type::I64,
                        },
                        captures: vec![crate::hir::HirClosureCapture {
                            name: "x".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            kind: crate::hir::HirClosureCaptureKind::SharedBorrow,
                            mutable: false,
                            ty: capture_ty.clone(),
                        }],
                    },
                    ty: lambda_ty.clone(),
                    span: crate::lexer::Span::test(),
                }),
            ],
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = program(
        HashMap::from([(function_id, function)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("main".to_string(), function_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let mut context = TypeContext::new();
    let ids = collect_hir_type_ids(&program, &mut context);

    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::Block {
                owner: function_id,
                path: vec![],
            })
            .unwrap()
        ),
        Type::Unit
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::LetStmt {
                owner: function_id,
                path: vec![0],
                name: "x".to_string(),
            })
            .unwrap()
        ),
        Type::I64
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::Expr {
                owner: function_id,
                path: vec![0, 0],
            })
            .unwrap()
        ),
        Type::I64
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::Expr {
                owner: function_id,
                path: vec![1, 0],
            })
            .unwrap()
        ),
        lambda_ty
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ClosureCapture {
                owner: function_id,
                path: vec![1, 0],
                name: "x".to_string(),
            })
            .unwrap()
        ),
        capture_ty
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ClosureParam {
                owner: function_id,
                path: vec![1, 0],
                index: 0,
            })
            .unwrap()
        ),
        Type::I64
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::Block {
                owner: function_id,
                path: vec![1, 0, 1],
            })
            .unwrap()
        ),
        Type::I64
    );
}

#[test]
fn hir_type_ids_collect_in_stable_semantic_order() {
    let mut functions = HashMap::new();
    let mut function_names = HashMap::new();
    for index in (1..=16).rev() {
        let id = def_id(index);
        let ty = Type::Struct {
            id,
            args: Vec::new(),
        };
        functions.insert(id, function(id, &format!("f{index}"), ty.clone(), ty));
        function_names.insert(format!("f{index}"), id);
    }

    let program = program(
        functions,
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HirNameTables {
            functions_by_name: function_names,
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let mut context = TypeContext::new();
    let ids = collect_hir_type_ids(&program, &mut context);

    for index in 1..=16 {
        let function = def_id(index);
        let id = ids
            .get(&HirTypeLocation::FunctionReturn { function })
            .unwrap();
        assert_eq!(id, crate::ids::TypeId(index - 1));
        assert_eq!(
            context.type_for(id),
            Type::Struct {
                id: function,
                args: Vec::new(),
            }
        );
    }
}

#[test]
fn hir_type_ids_collect_bound_type_args() {
    let function_id = def_id(1);
    let trait_id = def_id(2);
    let signature_id = def_id(3);
    let impl_id = def_id(4);
    let function_param = generic(function_id, 0);
    let signature_param = generic(signature_id, 0);
    let impl_param = generic(impl_id, 0);

    let mut bounded_function = function(function_id, "bounded", Type::I64, Type::I64);
    *bounded_function.generic_bounds = HashMap::from([(
        function_param,
        vec![single_type_arg_bound(
            trait_id,
            Type::Pointer(Box::new(Type::U8)),
        )],
    )])
    .into();

    let program = program(
        HashMap::from([(function_id, bounded_function)]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Iterable".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "next".to_string(),
                    HirFunctionSig {
                        id: signature_id,
                        name: "next".to_string(),
                        generic_params: vec![GenericParamDecl::type_param(signature_param, "T")],
                        params: vec![Type::I64],
                        ret: Type::Bool,
                        generic_bounds: HashMap::from([(
                            signature_param,
                            vec![single_type_arg_bound(trait_id, Type::Bool)],
                        )])
                        .into(),
                        self_receiver: None,
                        is_unsafe: false,
                    },
                )]),
            },
        )]),
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: GenericParamDecl::type_params(impl_id, ["T"]),
                receiver_pattern: Vec::new().into(),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::from([(
                    impl_param,
                    vec![single_type_arg_bound(trait_id, Type::Generic(impl_param))],
                )])
                .into(),
                methods: HashMap::new(),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("bounded".to_string(), function_id)]),
            traits_by_name: HashMap::from([("Iterable".to_string(), trait_id)]),
            ..HirNameTables::default()
        },
        HashMap::new(),
    );

    let mut context = TypeContext::new();
    let ids = collect_hir_type_ids(&program, &mut context);

    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::FunctionBoundArg {
                function: function_id,
                param: function_param,
                bound_index: 0,
                arg_index: 0,
            })
            .unwrap()
        ),
        Type::Pointer(Box::new(Type::U8))
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::TraitSignatureBoundArg {
                trait_id,
                signature: signature_id,
                param: signature_param,
                bound_index: 0,
                arg_index: 0,
            })
            .unwrap()
        ),
        Type::Bool
    );
    assert_eq!(
        context.type_for(
            ids.get(&HirTypeLocation::ImplBoundArg {
                impl_id,
                param: impl_param,
                bound_index: 0,
                arg_index: 0,
            })
            .unwrap()
        ),
        Type::Generic(impl_param)
    );
}
