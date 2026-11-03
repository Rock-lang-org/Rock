use super::*;

#[test]
fn selected_method_type_args_use_authority_sidecar_order() {
    let mut mono = Monomorphizer::new();
    let method_id = def_id(30);
    let t_id = GenericParamId {
        owner: method_id,
        index: 0,
    };
    let u_id = GenericParamId {
        owner: method_id,
        index: 1,
    };
    let method = HirFunction {
        id: method_id,
        name: "pair".to_string(),
        generic_params: vec![
            GenericParamDecl::type_param(u_id, "U"),
            GenericParamDecl::type_param(t_id, "T"),
        ],
        generic_bounds: HashMap::new().into(),
        params: vec![
            HirParam {
                name: "left".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(t_id),
                mutable: false,
                is_ref: false,
            },
            HirParam {
                name: "right".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(u_id),
                mutable: false,
                is_ref: false,
            },
        ],
        ret_type: Type::Unit,
        body: empty_body(Type::Unit),
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let impl_id = def_id(31);
    let mut target = HirMethodCallTarget::impl_method(impl_id, method_id, None);
    target.method_substitution = vec![
        crate::hir::HirTypeBinding {
            param: t_id,
            ty: Type::I32,
        },
        crate::hir::HirTypeBinding {
            param: u_id,
            ty: Type::Bool,
        },
    ];
    let type_args = mono
        .combine_impl_and_method_type_args(&method, &[], &[], &[], impl_id, &target)
        .expect("complete selected authority");

    assert_eq!(
        type_args
            .iter()
            .map(|id| mono.type_for(*id))
            .collect::<Vec<_>>(),
        vec![Type::Bool, Type::I32]
    );

    target.method_substitution.pop();
    assert!(mono
        .combine_impl_and_method_type_args(&method, &[], &[], &[], impl_id, &target)
        .is_none());
}

#[test]
fn trait_method_bindings_map_member_owned_generics_to_impl_method_generics() {
    let mut mono = Monomorphizer::new();
    let trait_id = def_id(32);
    let trait_member_id = def_id(33);
    let impl_id = def_id(34);
    let impl_method_id = def_id(35);
    let first_impl_param = GenericParamId {
        owner: impl_method_id,
        index: 0,
    };
    let second_impl_param = GenericParamId {
        owner: impl_method_id,
        index: 1,
    };
    let method = HirFunction {
        id: impl_method_id,
        name: "convert".to_string(),
        generic_params: vec![
            GenericParamDecl::type_param(first_impl_param, "T"),
            GenericParamDecl::type_param(second_impl_param, "U"),
        ],
        generic_bounds: HashMap::new().into(),
        params: vec![],
        ret_type: Type::Unit,
        body: empty_body(Type::Unit),
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let mut target = HirMethodCallTarget::trait_method(
        trait_id,
        trait_member_id,
        vec![],
        crate::hir::HirTraitDispatchKind::TraitBound,
    );
    target.method_substitution = vec![
        crate::hir::HirTypeBinding {
            param: GenericParamId {
                owner: trait_member_id,
                index: 0,
            },
            ty: Type::I32,
        },
        crate::hir::HirTypeBinding {
            param: GenericParamId {
                owner: trait_member_id,
                index: 1,
            },
            ty: Type::Bool,
        },
    ];

    let type_args = mono
        .combine_impl_and_method_type_args(&method, &[], &[], &[], impl_id, &target)
        .expect("member-owned trait bindings");

    assert_eq!(
        type_args
            .iter()
            .map(|id| mono.type_for(*id))
            .collect::<Vec<_>>(),
        vec![Type::I32, Type::Bool]
    );
}

#[test]
fn monomorphize_selected_index_and_index_mut_methods() {
    let mut mono = Monomorphizer::new();
    let cell_id = def_id(110);
    let index_trait_id = def_id(111);
    let index_mut_trait_id = def_id(112);
    let index_impl_id = def_id(113);
    let index_mut_impl_id = def_id(114);
    let index_member_id = def_id(115);
    let index_mut_member_id = def_id(116);
    let index_method_id = def_id(117);
    let index_mut_method_id = def_id(118);
    let cell_ty = Type::Struct {
        id: cell_id,
        args: Vec::new(),
    };
    let method = |id: DefId, name: &str, mutable: bool| HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![
            HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: cell_ty.clone(),
                mutable: false,
                is_ref: false,
            },
            HirParam {
                name: "key".to_string(),
                local_id: crate::ids::HirLocalId(1),
                ty: Type::I64,
                mutable: false,
                is_ref: false,
            },
        ],
        ret_type: Type::Reference {
            mutable,
            inner: Box::new(Type::I64),
        },
        body: empty_body(Type::Reference {
            mutable,
            inner: Box::new(Type::I64),
        }),
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };
    let indexed_impl =
        |impl_id: DefId, trait_id: DefId, type_name: &str, method: HirFunction| HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named(type_name.to_string()),
            type_name: type_name.to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(cell_ty.clone()),
            trait_name: Some(type_name.to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::I64],
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([(method.name.clone(), method)]),
        };

    mono.effective_trait_methods
        .insert((index_impl_id, index_member_id), index_method_id);
    mono.effective_trait_methods.insert(
        (index_mut_impl_id, index_mut_member_id),
        index_mut_method_id,
    );
    mono.trait_impls.insert(
        index_trait_id,
        vec![indexed_impl(
            index_impl_id,
            index_trait_id,
            "Index",
            method(index_method_id, "read_at", false),
        )],
    );
    mono.trait_impls.insert(
        index_mut_trait_id,
        vec![indexed_impl(
            index_mut_impl_id,
            index_mut_trait_id,
            "IndexMut",
            method(index_mut_method_id, "write_at", true),
        )],
    );

    let receiver = HirExpr {
        kind: HirExprKind::Var("cell".to_string()),
        ty: cell_ty,
        span: Span::test(),
    };
    let key = HirExpr {
        kind: HirExprKind::IntLiteral(0),
        ty: Type::I64,
        span: Span::test(),
    };
    let mut index_expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(receiver.clone()),
            "read_at".to_string(),
            vec![key.clone()],
            Some(ReceiverMode::Move),
            HirMethodCallTarget::trait_method(
                index_trait_id,
                index_member_id,
                vec![Type::I64],
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
        span: Span::test(),
    };
    let mut index_mut_expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(receiver.clone()),
            "write_at".to_string(),
            vec![key.clone()],
            Some(ReceiverMode::Move),
            HirMethodCallTarget::trait_method(
                index_mut_trait_id,
                index_mut_member_id,
                vec![Type::I64],
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::Reference {
            mutable: true,
            inner: Box::new(Type::I64),
        },
        span: Span::test(),
    };

    mono.monomorphize_trait_method_call(
        "read_at",
        &[receiver.clone(), key.clone()],
        &mut index_expr,
    )
    .expect("selected Index method should materialize");
    mono.monomorphize_trait_method_call("write_at", &[receiver, key], &mut index_mut_expr)
        .expect("selected IndexMut method should materialize");

    assert!(matches!(index_expr.kind, HirExprKind::Call(..)));
    assert!(matches!(index_mut_expr.kind, HirExprKind::Call(..)));
    let origins = mono
        .instances
        .records()
        .map(|record| record.origin.clone())
        .collect::<Vec<_>>();
    assert!(origins.contains(&InstanceOrigin::ImplMethod {
        owner: InstanceImplOwner::Named(index_impl_id),
        method: index_method_id,
    }));
    assert!(origins.contains(&InstanceOrigin::ImplMethod {
        owner: InstanceImplOwner::Named(index_mut_impl_id),
        method: index_mut_method_id,
    }));
}

#[test]
fn selected_trait_impl_method_call_rejects_wrong_method_id() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(710));
    let trait_id = DefId::new(CrateId(0), LocalDefId(711));
    let actual_method_id = DefId::new(CrateId(0), LocalDefId(712));
    let selected_method_id = DefId::new(CrateId(0), LocalDefId(713));
    let method = option_println_method(impl_id, actual_method_id);
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
    let recv = qualified_option_receiver();
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "println".to_string(),
            vec![],
            Some(ReceiverMode::Move),
            HirMethodCallTarget::impl_method(
                impl_id,
                selected_method_id,
                Some(crate::hir::HirSelectedTraitMember {
                    trait_id,
                    member_id: selected_method_id,
                    trait_args: Vec::new(),
                }),
            ),
        ),
        ty: Type::I32,
        span: Span::test(),
    };

    let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

    assert!(matches!(expr.kind, HirExprKind::MethodCall(..)));
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn selected_trait_method_call_without_impl_id_specializes_matching_trait_impl_method() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(720));
    let trait_id = DefId::new(CrateId(0), LocalDefId(721));
    let actual_method_id = DefId::new(CrateId(0), LocalDefId(722));
    let trait_method_id = DefId::new(CrateId(0), LocalDefId(723));
    let method = option_println_method(impl_id, actual_method_id);
    mono.effective_trait_methods
        .insert((impl_id, trait_method_id), actual_method_id);
    mono.set_impl_receiver_pattern_for_test(
        impl_id,
        HirImplReceiverPattern::Exact(enum_ty(
            "Option",
            vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })],
        )),
    );
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

    let recv = qualified_option_receiver();
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "println".to_string(),
            vec![],
            Some(ReceiverMode::Move),
            HirMethodCallTarget::trait_method(
                trait_id,
                trait_method_id,
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::I32,
        span: Span::test(),
    };

    let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

    let HirExprKind::Call(callee, _, _) = &expr.kind else {
        panic!("trait-bound method call should lower to a direct call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("trait-bound method callee should be resolved");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("trait-bound method callee should target an instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(
        record.origin,
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: actual_method_id,
        }
    );
}
