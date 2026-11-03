use std::collections::HashMap;

use crate::ast::{
    Block, Expression, Ident, IdentOrType, IdentifierPath, Literal, LiteralKind, Operand, Operator,
    PrimaryExpr, Statement, UnaryExpr,
};
use crate::hir::{
    HirBlock, HirCallTarget, HirEnum, HirExpr, HirExprKind, HirFunction, HirImpl, HirImplOwner,
    HirParam, HirStruct, HirVarTarget,
};
use crate::ids::{CrateId, DefId, HirLocalId, LocalDefId};
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::selection::SelectionDiagnostic;
use crate::types::{GenericParamDecl, GenericParamId, Type};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

#[test]
fn unsafe_control_statements_do_not_require_a_value_span() {
    let mut lowerer = Lowerer::new_for_test();
    for statement in [
        Statement::Continue(None),
        Statement::Return(None),
        Statement::Break(None),
    ] {
        let operand = Operand::Unsafe(
            Block {
                statements: vec![statement],
            },
            Span::test(),
        );
        let hir = lowerer.with_test_body_context(|lowerer| lowerer.lower_operand(&operand));
        assert!(matches!(hir.kind, HirExprKind::UnsafeBlock(_)));
        assert_eq!(hir.span, Span::test());
    }
}

#[test]
fn raw_pointer_dereference_diagnostic_is_selection() {
    let mut lowerer = Lowerer::new_for_test();
    let operator = Operator {
        value: "*".to_string(),
        span: Span::test(),
    };
    let expression = UnaryExpr::UnaryExpr(
        operator,
        Box::new(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(Ident {
                    name: "ptr".to_string(),
                    span: Span::test(),
                })],
            }),
            secondaries: None,
            type_annotation: None,
        })),
    );

    lowerer.with_test_body_context(|lowerer| {
        lowerer.scope.define_local(
            "ptr".to_string(),
            Type::Pointer(Box::new(Type::I64)),
            false,
            HirLocalId(0),
        );
        lowerer.lower_unary_expr(&expression);
    });

    assert_eq!(
        lowerer.diagnostics.errors()[0].code,
        crate::diagnostic::DiagnosticCode::Selection
    );
}

fn test_function(id: DefId, name: &str, params: Vec<Type>, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: Default::default(),
        params: params
            .into_iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("p{index}"),
                local_id: crate::ids::HirLocalId(0),
                ty,
                mutable: false,
                is_ref: false,
            })
            .collect(),
        ret_type,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

#[test]
fn instantiate_function_type_freshens_bounds_only_parameters() {
    let mut lowerer = Lowerer::new_for_test();
    let owner = def_id(800);
    let source = GenericParamId { owner, index: 0 };
    let item = GenericParamId { owner, index: 1 };
    let mut function = test_function(owner, "visit", vec![Type::Generic(source)], Type::Unit);
    function.generic_params = vec![
        GenericParamDecl::new(source, "C", crate::type_services::kind::Kind::Type),
        GenericParamDecl::new(item, "T", crate::type_services::kind::Kind::Type),
    ];
    function.generic_bounds.insert(
        source,
        vec![crate::types::TraitBound {
            trait_id: def_id(801),
            type_args: vec![Type::Generic(item)],
        }],
    );
    let mut item_variables = Vec::new();
    for _ in 0..2 {
        let Type::Function { params, .. } =
            lowerer.instantiate_function_type(&function, Span::test())
        else {
            panic!("expected instantiated function");
        };
        let Type::TypeVar(source_variable) = params[0] else {
            panic!("expected fresh source variable");
        };
        let bounds = lowerer.engine.get_bounds(source_variable);
        let Type::TypeVar(item_variable) = bounds[0].type_args[0] else {
            panic!("bounds-only parameter was not instantiated");
        };
        item_variables.push(item_variable);
    }
    assert_ne!(item_variables[0], item_variables[1]);
}

fn operator_impl(
    impl_id: DefId,
    method_id: DefId,
    receiver_ty: Type,
    operator: &str,
    param_types: Vec<Type>,
) -> HirImpl {
    let mut method = test_function(method_id, operator, param_types, Type::I64);
    method.is_method = true;
    HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("OperatorOwner".to_string()),
        type_name: "OperatorOwner".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(receiver_ty),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([(operator.to_string(), method)]),
    }
}

fn int_expr(value: u64) -> UnaryExpr {
    UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Literal(Literal {
            kind: LiteralKind::Number(value),
            span: Span::test(),
        }),
        secondaries: None,
        type_annotation: None,
    })
}

fn string_expr(value: &str) -> UnaryExpr {
    UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Literal(Literal {
            kind: LiteralKind::String(value.to_string()),
            span: Span::test(),
        }),
        secondaries: None,
        type_annotation: None,
    })
}

fn var_expr(name: &str) -> UnaryExpr {
    UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Ident(IdentifierPath {
            path: vec![IdentOrType::Ident(Ident {
                name: name.to_string(),
                span: Span::test(),
            })],
        }),
        secondaries: None,
        type_annotation: None,
    })
}

#[test]
fn test_string_literal_lowers_to_borrowed_str() {
    let mut lowerer = Lowerer::new_for_test();
    let hir = lowerer.lower_literal(&Literal {
        kind: LiteralKind::String("hello".to_string()),
        span: Span::test(),
    });

    assert_eq!(
        hir.ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        }
    );
}

#[test]
fn function_valued_ampersand_records_call_target() {
    let mut lowerer = Lowerer::new_for_test();
    let function_id = def_id(3);
    let param_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };
    let function_ty = Type::function(vec![param_ty.clone()], Type::I64);
    lowerer.items.insert_function(test_function(
        function_id,
        "borrowed",
        vec![param_ty],
        Type::I64,
    ));
    lowerer
        .resolver
        .item_paths
        .insert("borrowed".to_string(), function_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(function_id, "borrowed".to_string());
    lowerer
        .scope
        .define_alias("borrowed".to_string(), function_ty, false);
    lowerer.infix_precedence.insert("&".to_string(), 5);

    let expr = Expression::BinopExpr(
        var_expr("borrowed"),
        Operator {
            value: "&".to_string(),
            span: Span::test(),
        },
        Box::new(Expression::UnaryExpr(int_expr(1))),
    );
    let hir = lowerer.lower_expression(&expr);

    match hir.kind {
        HirExprKind::Call(_, _, Some(HirCallTarget::Function(id))) => {
            assert_eq!(id, function_id);
        }
        other => panic!("expected function-valued ampersand call target, got {other:?}"),
    }
}

#[test]
fn direct_function_target_does_not_require_method_payload() {
    let mut lowerer = Lowerer::new_for_test();
    let function_id = def_id(33);
    let resolved = crate::lower::resolution::LowerResolvedValue {
        name: "function".to_string(),
        ty: Type::I64,
        target: Some(HirVarTarget::Function(function_id)),
        is_alias: false,
        scope_index: None,
        should_instantiate: false,
    };

    assert_eq!(
        lowerer.instantiate_resolved_value_type(&resolved, Span::test()),
        Type::I64
    );
}

#[test]
fn custom_operator_import_alias_lowers_callee_to_function_id() {
    let mut lowerer = Lowerer::new_for_test();
    let operator_id = def_id(1);
    let inc_id = def_id(2);
    let inc_ty = Type::function(vec![Type::I64], Type::I64);
    let operator_ty = Type::function(vec![Type::I64, inc_ty.clone()], Type::I64);

    lowerer.items.insert_function(test_function(
        operator_id,
        "|>",
        vec![Type::I64, inc_ty.clone()],
        Type::I64,
    ));
    lowerer
        .items
        .insert_function(test_function(inc_id, "inc", vec![Type::I64], Type::I64));
    lowerer
        .scope
        .define_alias("|>".to_string(), operator_ty, false);
    lowerer.resolver.insert_import_alias_with_name(
        "|>".to_string(),
        "ops::|>".to_string(),
        operator_id,
    );
    lowerer.infix_precedence.insert("|>".to_string(), 5);

    let expr = Expression::BinopExpr(
        int_expr(41),
        Operator {
            value: "|>".to_string(),
            span: Span::test(),
        },
        Box::new(Expression::UnaryExpr(var_expr("inc"))),
    );
    let hir = lowerer.lower_expression(&expr);

    let HirExprKind::Call(callee, _, _) = hir.kind else {
        panic!("expected custom operator to lower to a call");
    };
    match callee.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "ops::|>");
            assert_eq!(reference.target, HirVarTarget::Function(operator_id));
        }
        other => panic!("expected resolved operator function target, got {other:?}"),
    }
}

#[test]
fn custom_operator_module_alias_lowers_through_resolver_id() {
    let mut lowerer = Lowerer::new_for_test();
    let function_id = def_id(30);
    lowerer.items.insert_function(test_function(
        function_id,
        "demo::ops::%%",
        vec![Type::I64, Type::I64],
        Type::I64,
    ));
    lowerer.resolver.insert_module_alias_with_name(
        "%%".to_string(),
        "demo::ops::%%".to_string(),
        function_id,
    );
    lowerer.infix_precedence.insert("%%".to_string(), 5);

    let expr = Expression::BinopExpr(
        int_expr(1),
        Operator {
            value: "%%".to_string(),
            span: Span::test(),
        },
        Box::new(Expression::UnaryExpr(int_expr(2))),
    );
    let hir = lowerer.lower_expression(&expr);

    let HirExprKind::Call(callee, _, Some(HirCallTarget::Function(id))) = hir.kind else {
        panic!(
            "expected resolver-backed custom operator call, got {:?}",
            hir.kind
        );
    };
    assert_eq!(id, function_id);
    match callee.kind {
        HirExprKind::ResolvedVar(reference) => assert_eq!(reference.name, "demo::ops::%%"),
        other => panic!("expected resolved operator callee, got {other:?}"),
    }
}

#[test]
fn custom_operator_module_alias_shadows_root_operator_with_same_symbol() {
    let mut lowerer = Lowerer::new_for_test();
    let root_id = def_id(31);
    let module_id = def_id(32);
    lowerer.items.insert_function(test_function(
        root_id,
        "%%",
        vec![Type::I64, Type::I64],
        Type::I64,
    ));
    lowerer.items.insert_function(test_function(
        module_id,
        "demo::ops::%%",
        vec![Type::I64, Type::I64],
        Type::I64,
    ));
    lowerer
        .resolver
        .item_paths
        .insert("%%".to_string(), root_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(root_id, "%%".to_string());
    lowerer.resolver.insert_module_alias_with_name(
        "%%".to_string(),
        "demo::ops::%%".to_string(),
        module_id,
    );
    lowerer.infix_precedence.insert("%%".to_string(), 5);

    let expr = Expression::BinopExpr(
        int_expr(1),
        Operator {
            value: "%%".to_string(),
            span: Span::test(),
        },
        Box::new(Expression::UnaryExpr(int_expr(2))),
    );
    let hir = lowerer.lower_expression(&expr);

    let HirExprKind::Call(callee, _, Some(HirCallTarget::Function(id))) = hir.kind else {
        panic!("expected module alias operator call, got {:?}", hir.kind);
    };
    assert_eq!(id, module_id);
    match callee.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "demo::ops::%%");
            assert_eq!(reference.target, HirVarTarget::Function(module_id));
        }
        other => panic!("expected module alias operator callee, got {other:?}"),
    }
}

#[test]
fn custom_operator_scope_local_lowers_callee_to_local_call_target() {
    let mut lowerer = Lowerer::new_for_test();
    let operator_local = crate::ids::HirLocalId(7);
    let operator_ty = Type::function(vec![Type::I64, Type::I64], Type::I64);

    lowerer
        .scope
        .define_local("%%".to_string(), operator_ty, false, operator_local);
    lowerer.infix_precedence.insert("%%".to_string(), 5);

    let expr = Expression::BinopExpr(
        int_expr(40),
        Operator {
            value: "%%".to_string(),
            span: Span::test(),
        },
        Box::new(Expression::UnaryExpr(int_expr(2))),
    );
    let hir = lowerer.lower_expression(&expr);

    let HirExprKind::Call(callee, _, Some(HirCallTarget::Local(target))) = hir.kind else {
        panic!(
            "expected local custom operator call target, got {:?}",
            hir.kind
        );
    };
    assert_eq!(target, operator_local);
    match callee.kind {
        HirExprKind::ResolvedVar(reference) => {
            assert_eq!(reference.name, "%%");
            assert_eq!(reference.target, HirVarTarget::Local(operator_local));
        }
        other => panic!("expected resolved local operator callee, got {other:?}"),
    }
}

#[test]
fn inherent_impl_custom_operator_call_carries_method_target() {
    let mut lowerer = Lowerer::new_for_test();
    let struct_id = def_id(10);
    let impl_id = def_id(11);
    let method_id = def_id(12);
    let receiver_ty = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let mut method = test_function(
        method_id,
        "%%",
        vec![receiver_ty.clone(), Type::I64],
        Type::I64,
    );
    method.is_method = true;

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
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(receiver_ty.clone()),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("%%".to_string(), method)]),
        })
        .unwrap();
    lowerer.infix_precedence.insert("%%".to_string(), 5);

    let left = HirExpr {
        ty: receiver_ty,
        kind: HirExprKind::Var("box".to_string()),
        span: Span::test(),
    };
    let right = HirExpr {
        ty: Type::I64,
        kind: HirExprKind::IntLiteral(2),
        span: Span::test(),
    };
    let hir = lowerer.build_precedence_tree(&[left, right], &["%%".to_string()], 0, 1);

    match hir.kind {
        HirExprKind::MethodCall(_, _, _, _, Some(target)) => {
            assert_eq!(target.impl_id(), Some(impl_id));
            assert_eq!(target.trait_id(), None);
            assert!(target.trait_args().is_empty());
            assert_eq!(target.method_id(), Some(method_id));
        }
        other => panic!("expected inherent custom operator target, got {other:?}"),
    }
}

#[test]
fn custom_operator_impl_selection_uses_canonical_impl_method() {
    let mut lowerer = Lowerer::new_for_test();
    let option_id = def_id(20);
    let impl_id = def_id(21);
    let method_id = def_id(22);
    let receiver_ty = Type::Enum {
        id: option_id,
        args: vec![Type::I64],
    };
    let mut method = test_function(
        method_id,
        "<&>",
        vec![receiver_ty.clone(), Type::I64],
        Type::I64,
    );
    method.is_method = true;
    lowerer.items.insert_enumeration(HirEnum {
        id: option_id,
        name: "Option".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: option_id,
                index: 0,
            },
            "T",
        )],
        variants: Vec::new(),
    });
    lowerer
        .resolver
        .item_names_by_id
        .insert(option_id, "Option".to_string());
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Option".to_string()),
            type_name: "Option".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(receiver_ty.clone()),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("<&>".to_string(), method)]),
        })
        .unwrap();
    lowerer.infix_precedence.insert("<&>".to_string(), 5);
    let left_ty = lowerer.engine.fresh_type_var();
    lowerer
        .engine
        .unify(&left_ty, &receiver_ty)
        .expect("test receiver type should resolve to Option");

    let left = HirExpr {
        ty: left_ty,
        kind: HirExprKind::Var("option".to_string()),
        span: Span::test(),
    };
    let right = HirExpr {
        ty: Type::I64,
        kind: HirExprKind::IntLiteral(2),
        span: Span::test(),
    };
    let hir = lowerer.build_precedence_tree(&[left, right], &["<&>".to_string()], 0, 1);

    match hir.kind {
        HirExprKind::MethodCall(_, _, _, _, Some(target)) => {
            assert_eq!(target.impl_id(), Some(impl_id));
            assert_eq!(target.method_id(), Some(method_id));
        }
        other => panic!("expected impl-backed custom operator target, got {other:?}"),
    }
}

#[test]
fn missing_concrete_operator_impl_lowers_to_error_typed_expression() {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.infix_precedence.insert("+".to_string(), 6);
    let expr = Expression::BinopExpr(
        string_expr("left"),
        Operator {
            value: "+".to_string(),
            span: Span::test(),
        },
        Box::new(Expression::UnaryExpr(string_expr("right"))),
    );

    let hir = lowerer.lower_expression(&expr);

    assert_eq!(hir.ty, Type::Error);
    assert!(
        !matches!(hir.kind, HirExprKind::Unit),
        "missing operator impl must not recover as executable unit"
    );
    assert!(lowerer.diagnostics.errors().iter().any(|error| error
        .message
        .contains("No implementation found for operator '+'")));
}

#[test]
fn missing_concrete_unary_operator_impl_lowers_to_error_typed_expression() {
    let mut lowerer = Lowerer::new_for_test();
    let expr = UnaryExpr::UnaryExpr(
        Operator {
            value: "-".to_string(),
            span: Span::test(),
        },
        Box::new(string_expr("not numeric")),
    );

    let hir = lowerer.lower_unary_expr(&expr);

    assert_eq!(hir.ty, Type::Error);
    assert!(
        !matches!(hir.kind, HirExprKind::Unit),
        "missing unary operator impl must not recover as executable unit"
    );
    assert!(lowerer.diagnostics.errors().iter().any(|error| error
        .message
        .contains("No implementation found for operator '-'")));
}

#[test]
fn unresolved_binary_operator_reports_all_ambiguous_impl_ids() {
    let mut lowerer = Lowerer::new_for_test();
    let first_impl = def_id(30);
    let second_impl = def_id(31);
    lowerer
        .items
        .insert_impl(operator_impl(
            first_impl,
            def_id(32),
            Type::Bool,
            "%%",
            vec![Type::Bool, Type::I64],
        ))
        .unwrap();
    lowerer
        .items
        .insert_impl(operator_impl(
            second_impl,
            def_id(33),
            Type::Str,
            "%%",
            vec![Type::Str, Type::I64],
        ))
        .unwrap();
    let left_ty = lowerer.engine.fresh_type_var();
    let left = HirExpr {
        ty: left_ty.clone(),
        kind: HirExprKind::Var("left".to_string()),
        span: Span::test(),
    };
    let right = HirExpr {
        ty: Type::I64,
        kind: HirExprKind::IntLiteral(1),
        span: Span::test(),
    };

    assert_eq!(
        lowerer.infer_unresolved_binary_operator_receiver(&left, &right, "%%"),
        Err(SelectionDiagnostic::AmbiguousCandidates {
            operation: "%%".to_string(),
            receiver: left_ty,
            candidates: vec![first_impl, second_impl],
        })
    );
}

#[test]
fn unresolved_unary_operator_reports_all_ambiguous_impl_ids() {
    let mut lowerer = Lowerer::new_for_test();
    let first_impl = def_id(40);
    let second_impl = def_id(41);
    lowerer
        .items
        .insert_impl(operator_impl(
            first_impl,
            def_id(42),
            Type::Bool,
            "!",
            vec![Type::Bool],
        ))
        .unwrap();
    lowerer
        .items
        .insert_impl(operator_impl(
            second_impl,
            def_id(43),
            Type::Str,
            "!",
            vec![Type::Str],
        ))
        .unwrap();
    let operand_ty = lowerer.engine.fresh_type_var();
    let operand = HirExpr {
        ty: operand_ty.clone(),
        kind: HirExprKind::Var("operand".to_string()),
        span: Span::test(),
    };

    assert_eq!(
        lowerer.infer_unresolved_unary_operator_receiver(&operand, "!"),
        Err(SelectionDiagnostic::AmbiguousCandidates {
            operation: "!".to_string(),
            receiver: operand_ty,
            candidates: vec![first_impl, second_impl],
        })
    );
}

#[test]
fn concrete_operator_does_not_select_first_compatible_impl() {
    let mut lowerer = Lowerer::new_for_test();
    let first_impl = def_id(50);
    let second_impl = def_id(51);
    lowerer
        .items
        .insert_impl(operator_impl(
            first_impl,
            def_id(52),
            Type::Bool,
            "%%",
            vec![Type::Bool, Type::I64],
        ))
        .unwrap();
    lowerer
        .items
        .insert_impl(operator_impl(
            second_impl,
            def_id(53),
            Type::Bool,
            "%%",
            vec![Type::Bool, Type::I64],
        ))
        .unwrap();
    lowerer.infix_precedence.insert("%%".to_string(), 5);
    let left = HirExpr {
        ty: Type::Bool,
        kind: HirExprKind::BoolLiteral(true),
        span: Span::test(),
    };
    let right = HirExpr {
        ty: Type::I64,
        kind: HirExprKind::IntLiteral(1),
        span: Span::test(),
    };

    let hir = lowerer.build_precedence_tree(&[left, right], &["%%".to_string()], 0, 1);

    assert_eq!(hir.ty, Type::Error);
    assert!(lowerer.diagnostics.errors().iter().any(|error| {
        error.message.contains("Ambiguous selection for '%%'")
            && error.message.contains("multiple matching implementations")
            && !error.message.contains("DefId")
    }));
}
