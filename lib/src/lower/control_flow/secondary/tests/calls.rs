use super::*;

#[test]
fn try_with_missing_marked_trait_id_reports_protocol_unavailable() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.language_items.try_protocol = Some(TryLanguageItems {
        try_trait_id: DefId::new(CrateId(0), LocalDefId(100)),
        output_id: AssocTypeId(0),
        residual_id: AssocTypeId(1),
        branch_method_id: DefId::new(CrateId(0), LocalDefId(101)),
        from_residual_trait_id: DefId::new(CrateId(0), LocalDefId(102)),
        from_residual_method_id: DefId::new(CrateId(0), LocalDefId(103)),
        control_flow_enum_id: DefId::new(CrateId(0), LocalDefId(104)),
        break_variant_id: VariantId(0),
        continue_variant_id: VariantId(1),
    });

    let lowered = lowerer.apply_secondary(
        receiver_expr(Type::I64),
        &SecondaryExpr::Interogation,
        ExprUse::Value,
        false,
    );

    assert_eq!(lowered.ty, Type::Error);
    assert_eq!(
        lowerer.errors()[0].message,
        "Cannot use '?' because the Try language-item protocol is unavailable"
    );
}

fn lowerer_with_inherent_method() -> (Lowerer, DefId, DefId) {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = DefId::new(CrateId(0), LocalDefId(10));
    let impl_id = DefId::new(CrateId(0), LocalDefId(20));
    let method_id = DefId::new(CrateId(0), LocalDefId(21));
    lowerer.items.insert_structure(HirStruct {
        id: struct_id,
        name: "Box".to_string(),
        generic_params: Vec::new(),
        fields: Vec::new(),
    });
    lowerer
        .resolver
        .item_names_by_id
        .insert(struct_id, "Box".to_string());
    let mut method = test_function(method_id, "value", Type::I64);
    method.is_method = true;
    method.self_receiver = Some(crate::types::ReceiverMode::Shared);
    method.params.push(HirParam {
        name: "self".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
        mutable: false,
        is_ref: false,
    });

    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        })
        .unwrap();
    (lowerer, impl_id, method_id)
}

fn lowerer_with_curried_inherent_method() -> (Lowerer, DefId, DefId) {
    let (mut lowerer, impl_id, method_id) = lowerer_with_inherent_method();
    let method = lowerer
        .items
        .impl_def_mut(impl_id)
        .and_then(|imp| imp.methods.get_mut("value"))
        .expect("test helper should create value method");
    method.is_curried = true;
    method.params.push(HirParam {
        name: "amount".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::I64,
        mutable: false,
        is_ref: false,
    });
    method.ret_type = Type::function(vec![Type::I64], Type::I64);

    (lowerer, impl_id, method_id)
}

fn number_expression(value: u64) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Literal(Literal {
            kind: LiteralKind::Number(value),
            span: Span::test(),
        }),
        secondaries: None,
        type_annotation: None,
    }))
}

fn ident_expression(name: &str) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Ident(IdentifierPath {
            path: vec![IdentOrType::Ident(Ident {
                name: name.to_string(),
                span: Span::test(),
            })],
        }),
        secondaries: None,
        type_annotation: None,
    }))
}

fn call_expr(name: &str, args: Vec<Expression>) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Ident(IdentifierPath {
            path: vec![IdentOrType::Ident(Ident {
                name: name.to_string(),
                span: Span::test(),
            })],
        }),
        secondaries: Some(vec![SecondaryExpr::Arguments(
            args.into_iter()
                .map(|arg| crate::ast::Argument { arg })
                .collect(),
        )]),
        type_annotation: None,
    }))
}

#[test]
fn direct_function_call_records_call_target() {
    let mut lowerer = Lowerer::new_for_test();
    let function_id = DefId::new(CrateId(0), LocalDefId(88));
    lowerer
        .items
        .insert_function(test_function(function_id, "answer", Type::I64));
    lowerer
        .resolver
        .item_paths
        .insert("answer".to_string(), function_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(function_id, "answer".to_string());
    lowerer.scope.define_alias(
        "answer".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );

    let expr = call_expr("answer", Vec::new());
    let hir = lowerer.lower_expression(&expr);

    match hir.kind {
        HirExprKind::Call(_, _, Some(HirCallTarget::Function(id))) => {
            assert_eq!(id, function_id);
        }
        other => panic!("expected function call target, got {other:?}"),
    }
}

#[test]
fn direct_function_lookup_does_not_resolve_method_id() {
    let (lowerer, _impl_id, method_id) = lowerer_with_inherent_method();

    assert!(lowerer.current_function(method_id).is_none());
}

#[test]
fn direct_extern_call_records_call_target() {
    let mut lowerer = Lowerer::new_for_test();
    let extern_id = DefId::new(CrateId(0), LocalDefId(89));
    lowerer.items.insert_extern(HirExtern {
        id: extern_id,
        name: "puts".to_string(),
        params: vec![Type::I32],
        ret: Type::I32,
        variadic: false,
        is_unsafe: false,
    });
    lowerer
        .resolver
        .item_paths
        .insert("puts".to_string(), extern_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(extern_id, "puts".to_string());
    lowerer.scope.define_alias(
        "puts".to_string(),
        Type::function(vec![Type::I32], Type::I32),
        false,
    );

    let expr = call_expr("puts", vec![number_expression(0)]);
    let hir = lowerer.lower_expression(&expr);

    match hir.kind {
        HirExprKind::Call(_, _, Some(HirCallTarget::Extern(id))) => assert_eq!(id, extern_id),
        other => panic!("expected extern call target, got {other:?}"),
    }
}

#[test]
fn local_function_value_call_records_call_target() {
    let mut lowerer = Lowerer::new_for_test();
    let expr = call_expr("callback", Vec::new());
    let (local_id, hir) = lowerer.with_test_body_context(|lowerer| {
        let local_id = lowerer.fresh_local_id();
        lowerer.scope.define_local(
            "callback".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
            local_id,
        );
        (local_id, lowerer.lower_expression(&expr))
    });

    match hir.kind {
        HirExprKind::Call(_, _, Some(HirCallTarget::Local(id))) => assert_eq!(id, local_id),
        other => panic!("expected local function-value call target, got {other:?}"),
    }
}

#[test]
fn arity_error_direct_call_records_call_target() {
    let mut lowerer = Lowerer::new_for_test();
    let function_id = DefId::new(CrateId(0), LocalDefId(91));
    lowerer
        .items
        .insert_function(test_function(function_id, "pair", Type::I64));
    lowerer
        .resolver
        .item_paths
        .insert("pair".to_string(), function_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(function_id, "pair".to_string());
    lowerer.scope.define_alias(
        "pair".to_string(),
        Type::function(vec![Type::I64, Type::I64], Type::I64),
        false,
    );

    let expr = call_expr("pair", vec![number_expression(1)]);
    let hir = lowerer.lower_expression(&expr);

    assert_eq!(hir.ty, Type::Error);
    match hir.kind {
        HirExprKind::Call(_, _, Some(HirCallTarget::Function(id))) => {
            assert_eq!(id, function_id);
        }
        other => panic!("expected arity-error call target, got {other:?}"),
    }
}

#[test]
fn unsupported_index_produces_error_expression() {
    let mut lowerer = Lowerer::new_for_test();
    let source_span = Span {
        file_path: "main.rk".into(),
        start: 10,
        end: 15,
    };
    let mut receiver = receiver_expr(Type::Str);
    receiver.span = source_span.clone();
    let lowered = lowerer.apply_secondary(
        receiver,
        &SecondaryExpr::Indice(Box::new(number_expression(0))),
        ExprUse::Value,
        false,
    );

    assert_eq!(lowered.ty, Type::Error);
    assert_eq!(lowered.span.file_path, source_span.file_path);
    assert_eq!(lowered.span.start, source_span.start);
    assert_eq!(lowered.span.end, source_span.end);
    assert!(matches!(
        lowered.kind,
        HirExprKind::Var(ref name) if name == "<error>"
    ));
    assert!(!lowerer.errors().is_empty());
    let diagnostic_origin = lowerer.errors()[0].span();
    let diagnostic_span = diagnostic_origin
        .as_ref()
        .expect("unsupported index diagnostic should have a span");
    assert_eq!(diagnostic_span.file_path, source_span.file_path);
    assert_eq!(diagnostic_span.start, source_span.start);
    assert_eq!(diagnostic_span.end, source_span.end);
}

#[test]
fn accepted_index_is_selected_method_call() {
    let mut lowerer = Lowerer::new_for_test();
    let bag_id = DefId::new(CrateId(0), LocalDefId(50));
    let trait_id = DefId::new(CrateId(1), LocalDefId(150));
    let impl_id = DefId::new(CrateId(0), LocalDefId(52));
    let method_id = DefId::new(CrateId(0), LocalDefId(53));
    let generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Index".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "Idx",
        )],
        associated_types: vec![crate::hir::HirAssociatedTypeDecl {
            id: AssocTypeId(0),
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "index".to_string(),
            HirFunctionSig {
                id: method_id,
                name: "index".to_string(),
                generic_params: Vec::new(),
                params: vec![
                    Type::TypeVar(crate::ids::TypeVarId(0)),
                    Type::Generic(generic),
                ],
                ret: Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Generic(generic)),
                },
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(crate::types::ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    });
    lowerer.language_items.index = Some(IndexLanguageItems {
        trait_id,
        output_id: AssocTypeId(0),
        method_id,
    });
    lowerer
        .resolver
        .item_names_by_id
        .insert(bag_id, "Bag".to_string());
    let mut method = test_function(
        method_id,
        "index",
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Generic(generic)),
        },
    );
    method.is_method = true;
    method.self_receiver = Some(crate::types::ReceiverMode::Shared);
    method.params.push(HirParam {
        name: "self".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::TypeVar(crate::ids::TypeVarId(0)),
        mutable: false,
        is_ref: false,
    });
    method.params.push(HirParam {
        name: "index".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Generic(generic),
        mutable: false,
        is_ref: false,
    });
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Bag".to_string()),
            type_name: "Bag".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                id: bag_id,
                args: Vec::new(),
            }),
            trait_name: Some("Index".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(generic, "T")],
            trait_arg_types: vec![Type::Generic(generic)],
            associated_types: vec![crate::hir::HirAssociatedTypeDef {
                id: AssocTypeId(0),
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("index".to_string(), method)]),
        })
        .unwrap();
    lowerer
        .imported_effective_trait_methods
        .insert((impl_id, method_id), method_id);
    let receiver = receiver_expr(Type::Struct {
        id: bag_id,
        args: Vec::new(),
    });
    lowerer.scope.define("index".to_string(), Type::I64, false);

    let lowered = lowerer.apply_secondary(
        receiver,
        &SecondaryExpr::Indice(Box::new(ident_expression("index"))),
        ExprUse::Value,
        false,
    );

    assert_eq!(lowered.ty, Type::I64, "errors: {:?}", lowerer.errors());
    assert!(
        lowerer.errors().is_empty(),
        "errors: {:?}",
        lowerer.errors()
    );
    match lowered.kind {
        HirExprKind::Deref(method_call) => match method_call.kind {
            HirExprKind::MethodCall(_, name, args, _, Some(target)) => {
                assert_eq!(name, "index");
                assert_eq!(args.len(), 1);
                assert_eq!(target.impl_id(), Some(impl_id));
                assert_eq!(target.trait_id(), Some(trait_id));
                assert_eq!(target.method_id(), Some(method_id));
            }
            other => panic!("expected selected index method call, got {other:?}"),
        },
        other => panic!("expected dereference of selected index method call, got {other:?}"),
    }
}

#[test]
fn inherent_impl_method_call_carries_exact_target() {
    let (mut lowerer, impl_id, method_id) = lowerer_with_inherent_method();
    let receiver = receiver_expr(Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(10)),
        args: Vec::new(),
    });
    let field = HirExpr {
        kind: HirExprKind::FieldAccess(Box::new(receiver), "value".to_string(), None),
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
            assert_eq!(target.impl_id(), Some(impl_id));
            assert_eq!(target.trait_id(), None);
            assert!(target.trait_args().is_empty());
            assert_eq!(target.method_id(), Some(method_id));
        }
        other => panic!("expected exact inherent method target, got {other:?}"),
    }
}

#[test]
fn empty_bang_rejects_method_with_explicit_params() {
    let (mut lowerer, impl_id, _) = lowerer_with_inherent_method();
    {
        let method = lowerer
            .items
            .impl_def_mut(impl_id)
            .and_then(|imp| imp.methods.get_mut("value"))
            .expect("test helper should create value method");
        method.params.push(HirParam {
            name: "amount".to_string(),
            local_id: crate::ids::HirLocalId(1),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        });
        method.ret_type = Type::Unit;
        method.body.ty = Type::Unit;
    }
    let receiver = receiver_expr(Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(10)),
        args: Vec::new(),
    });
    assert!(
        lowerer
            .concrete_method_candidate(receiver.clone(), "value")
            .is_some(),
        "fixture should select the canonical inherent method"
    );
    let method_field = lowerer.apply_secondary(
        receiver,
        &SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
            name: "value".to_string(),
            span: Span::test(),
        })),
        ExprUse::Value,
        false,
    );
    let lowered = lowerer.apply_secondary(
        method_field,
        &SecondaryExpr::Arguments(Vec::new()),
        ExprUse::Value,
        false,
    );

    assert_eq!(lowered.ty, Type::Error);
    assert!(lowerer
        .diagnostics
        .errors()
        .iter()
        .any(|error| { error.message == "Type mismatch: method 'value' expected 1 args, got 0" }));
}

#[test]
fn inherent_impl_method_value_carries_exact_target() {
    let (mut lowerer, impl_id, method_id) = lowerer_with_inherent_method();

    let lowered = lowerer.apply_secondary(
        receiver_expr(Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(10)),
            args: Vec::new(),
        }),
        &SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
            name: "value".to_string(),
            span: Span::test(),
        })),
        ExprUse::Value,
        true,
    );

    match lowered.kind {
        HirExprKind::Lambda { body, .. } => match body.stmts.as_slice() {
            [crate::hir::HirStmt::Expr(HirExpr {
                kind: HirExprKind::MethodCall(_, _, _, _, Some(target)),
                ..
            })] => {
                assert_eq!(target.impl_id(), Some(impl_id));
                assert_eq!(target.trait_id(), None);
                assert_eq!(target.method_id(), Some(method_id));
            }
            other => panic!("expected targeted method-value body, got {other:?}"),
        },
        other => panic!("expected method-value lambda, got {other:?}"),
    }
}

#[test]
fn curried_inherent_method_partial_application_carries_exact_target() {
    let (mut lowerer, impl_id, method_id) = lowerer_with_curried_inherent_method();
    let receiver = receiver_expr(Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(10)),
        args: Vec::new(),
    });
    let field = HirExpr {
        kind: HirExprKind::FieldAccess(Box::new(receiver), "value".to_string(), None),
        ty: lowerer.engine.fresh_type_var(),
        span: Span::test(),
    };
    let lowered = lowerer.apply_secondary(
        field,
        &SecondaryExpr::Arguments(vec![crate::ast::Argument {
            arg: crate::ast::Expression::UnaryExpr(crate::ast::UnaryExpr::PrimaryExpr(
                crate::ast::PrimaryExpr {
                    operand: crate::ast::Operand::Literal(crate::ast::Literal {
                        kind: crate::ast::LiteralKind::Number(1),
                        span: Span::test(),
                    }),
                    secondaries: None,
                    type_annotation: None,
                },
            )),
        }]),
        ExprUse::Value,
        false,
    );

    match lowered.kind {
        HirExprKind::MethodCall(_, _, args, _, Some(target)) => {
            assert_eq!(args.len(), 1);
            assert_eq!(target.impl_id(), Some(impl_id));
            assert_eq!(target.trait_id(), None);
            assert_eq!(target.method_id(), Some(method_id));
        }
        other => panic!("expected curried targeted method call, got {other:?}"),
    }
}
