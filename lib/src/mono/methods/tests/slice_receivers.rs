use super::*;

fn borrowed_slice_println_method(mutable: bool) -> HirFunction {
    HirFunction {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        name: "println".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(0),
                index: 0,
            },
            "T",
        )],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Reference {
                mutable,
                inner: Box::new(Type::Slice(Box::new(Type::Generic(GenericParamId {
                    owner: def_id(0),
                    index: 0,
                })))),
            },
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I32,
        body: empty_body(Type::Unit),
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    }
}

#[test]
fn test_monomorphize_trait_method_call_does_not_treat_str_as_u8_slice() {
    let mut mono = Monomorphizer::new();
    seed_owner(&mut mono, "&[U8]");
    mono.trait_impls.insert(
        def_id(400),
        vec![HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            owner: HirImplOwner::BuiltinSlice,
            type_name: "&[U8]".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![Type::U8].into(),
            trait_name: Some("Bytes".to_string()),
            trait_id: Some(def_id(400)),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("println".to_string(), byte_slice_println_method())]),
        }],
    );

    let recv = HirExpr {
        kind: HirExprKind::Var("text".to_string()),
        ty: Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        },
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

    assert!(matches!(expr.kind, HirExprKind::MethodCall(..)));
}

#[test]
fn test_monomorphize_trait_method_call_preserves_borrowed_slice_self_type() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(0));
    let trait_id = def_id(600);
    let method_id = def_id(601);
    let mut method = borrowed_slice_println_method(false);
    method.id = method_id;
    mono.trait_impls.insert(
        trait_id,
        vec![HirImpl {
            id: impl_id,
            owner: HirImplOwner::BuiltinSlice,
            type_name: "&[T]".to_string(),
            type_generics: type_generic(impl_id, "T"),
            receiver_pattern: vec![Type::Generic(GenericParamId {
                owner: def_id(0),
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
    mono.set_impl_receiver_pattern_for_test(
        impl_id,
        crate::hir::HirImplReceiverPattern::SliceFamily {
            element: Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            }),
        },
    );

    let recv = HirExpr {
        kind: HirExprKind::Var("bytes".to_string()),
        ty: Type::Reference {
            mutable: false,
            inner: Box::new(Type::Slice(Box::new(Type::U8))),
        },
        span: Span::test(),
    };
    let mut selected_target = selected_impl_target(impl_id, method_id, Type::U8);
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

    let body = mono
        .instances
        .records()
        .find_map(|record| mono.instances.pre_mir_body(record.id))
        .expect("expected monomorphized borrowed slice method");

    assert_eq!(
        body.params[0].ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Slice(Box::new(Type::U8))),
        }
    );
}
