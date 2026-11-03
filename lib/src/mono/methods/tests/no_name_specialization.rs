use super::*;

#[test]
fn monomorphize_standalone_method_call_without_selected_target_does_not_specialize_by_name() {
    let mut mono = Monomorphizer::new();
    let impl_id = def_id(90);
    let method_id = def_id(91);
    mono.generic_impls
        .insert(impl_id, generic_map_impl("Option", impl_id, method_id));
    let (recv, func_arg, mut expr) = map_expr_for_type("Option");

    let error = mono
        .monomorphize_standalone_method_call("map", &[recv, func_arg], &mut expr)
        .expect_err("a targetless standalone method must be rejected");
    assert!(matches!(
        error.kind,
        crate::mono::MonoErrorKind::InvalidBindings { .. }
    ));

    assert!(
        matches!(expr.kind, HirExprKind::MethodCall(..)),
        "targetless standalone methods must not specialize by type and method name"
    );
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn monomorphize_trait_method_call_without_selected_target_does_not_specialize_by_name() {
    let mut mono = Monomorphizer::new();
    let impl_id = def_id(92);
    let trait_id = def_id(93);
    let method_id = def_id(94);
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
    let recv = HirExpr {
        kind: HirExprKind::Var("some".to_string()),
        ty: enum_ty("Option", vec![Type::I64]),
        span: Span::test(),
    };
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "println".to_string(),
            vec![],
            Some(ReceiverMode::Move),
            HirMethodCallTarget::trait_method(
                def_id(999),
                def_id(998),
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::I32,
        span: Span::test(),
    };

    let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

    assert!(
        matches!(expr.kind, HirExprKind::MethodCall(..)),
        "targetless trait methods must not specialize by receiver and method name"
    );
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn monomorphize_trait_method_call_rejects_target_without_trait_identity() {
    let mut mono = Monomorphizer::new();
    let impl_id = def_id(96);
    let method_id = def_id(97);
    let method = option_println_method(impl_id, method_id);
    mono.trait_impls.insert(
        def_id(98),
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
            trait_id: Some(def_id(98)),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("println".to_string(), method)]),
        }],
    );
    let recv = qualified_option_receiver();
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "println".to_string(),
            vec![],
            Some(ReceiverMode::Move),
            HirMethodCallTarget::impl_method(impl_id, method_id, None),
        ),
        ty: Type::I32,
        span: Span::test(),
    };

    let error = mono
        .monomorphize_trait_method_call("println", &[recv], &mut expr)
        .expect_err("an impl target without trait identity must be rejected");

    assert!(matches!(
        error.kind,
        crate::mono::MonoErrorKind::InvalidBindings { .. }
    ));

    assert!(matches!(expr.kind, HirExprKind::MethodCall(..)));
    assert!(mono.instances.records().next().is_none());
}
