use super::*;

#[test]
fn inferred_candidates_only_normalize_eligible_impls_in_identity_order() {
    let owner = def_id(10);
    let mut impls = HashMap::new();
    for index in (20..52).rev() {
        let mut function = method(def_id(index + 100), owner);
        if index == 22 {
            function.self_receiver = None;
            function.is_method = false;
            function.params.clear();
        }
        let methods = if index <= 22 {
            HashMap::from([("value".to_string(), function)])
        } else {
            HashMap::new()
        };
        let imp = HirImpl {
            id: def_id(index),
            owner: HirImplOwner::Named("Owner".to_string()),
            type_name: "Owner".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HirGenericBounds::new(),
            methods,
        };
        impls.insert(imp.id, imp);
    }
    let traits = HashMap::new();
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("unresolved".to_string()),
        ty: Type::TypeVar(TypeVarId(0)),
        span: Span::test(),
    };
    let mut normalizations = 0;
    let inferred = service.select_inferred_method_candidates(&receiver, "value", |ty| {
        normalizations += 1;
        ty.clone()
    });
    assert_eq!(
        normalizations, 2,
        "irrelevant impls must not start candidate searches"
    );
    assert_eq!(
        inferred
            .iter()
            .map(|selected| selected.target.impl_id().unwrap())
            .collect::<Vec<_>>(),
        vec![def_id(20), def_id(21)],
    );
    let mut concrete_receiver = receiver.clone();
    concrete_receiver.ty = Type::Struct {
        id: owner,
        args: Vec::new(),
    };
    let candidates = [receiver_candidate(concrete_receiver)];
    let concrete = service.select_concrete_method_candidates(&candidates, "value", |ty| ty.clone());
    assert_eq!(
        inferred
            .iter()
            .map(SelectedMethod::authority)
            .collect::<Vec<_>>(),
        concrete
            .iter()
            .map(SelectedMethod::authority)
            .collect::<Vec<_>>(),
    );
    assert!(matches!(
        service.select_concrete_method(&candidates, "value", |ty| ty.clone()),
        Err(SelectionDiagnostic::AmbiguousCandidates { candidates, .. })
            if candidates == vec![def_id(20), def_id(21)]
    ));
}

#[test]
fn selection_service_selects_current_trait_self_method_by_trait_identity() {
    let trait_id = def_id(10);
    let method_id = def_id(11);
    let self_param = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let mut method = method(method_id, trait_id);
    method.name = "value".to_string();
    method.params[0].ty = Type::Generic(self_param);
    method.ret_type = Type::Bool;

    let traits = HashMap::from([(
        trait_id,
        crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Reader".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("value".to_string(), method.clone())]),
            signatures: HashMap::new(),
        },
    )]);
    let impls: HashMap<DefId, HirImpl> = HashMap::new();
    let current_impl_bounds = HirGenericBounds::new();
    let service =
        SelectionService::new(&traits, &impls, None, Some(trait_id), &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("self".to_string()),
        ty: Type::Generic(self_param),
        span: Span::test(),
    };

    let selected = service
        .select_current_trait_method(receiver, "value", true)
        .expect("expected current-trait selection");

    assert_eq!(selected.origin, SelectedOrigin::CurrentTrait { trait_id });
    assert_eq!(selected.return_type, Type::Bool);
    assert_eq!(selected.target.impl_id(), None);
    assert_eq!(selected.target.trait_id(), Some(trait_id));
    assert_eq!(selected.target.method_id(), Some(method_id));
}

#[test]
fn selection_service_current_trait_signature_uses_canonical_params_without_duplicate_self() {
    let trait_id = def_id(12);
    let method_id = def_id(13);
    let self_param = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let self_ty = Type::Reference {
        mutable: true,
        inner: Box::new(Type::Generic(self_param)),
    };
    let traits = HashMap::from([(
        trait_id,
        crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Reader".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "set".to_string(),
                crate::hir::HirFunctionSig {
                    id: method_id,
                    name: "set".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(self_param, "Self")],
                    params: vec![self_ty.clone(), Type::I64],
                    ret: Type::Bool,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Mut),
                    is_unsafe: false,
                },
            )]),
        },
    )]);
    let impls: HashMap<DefId, HirImpl> = HashMap::new();
    let current_impl_bounds = HirGenericBounds::new();
    let service =
        SelectionService::new(&traits, &impls, None, Some(trait_id), &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("self".to_string()),
        ty: Type::Generic(self_param),
        span: Span::test(),
    };

    let selected = service
        .select_current_trait_method(receiver, "set", true)
        .expect("expected current-trait signature selection");
    let function = selected
        .function
        .expect("selection should fabricate function");

    assert_eq!(function.params.len(), 2);
    assert_eq!(function.params[0].name, "self");
    assert!(function.params[0].mutable);
    assert_eq!(function.params[0].ty, self_ty);
    assert_eq!(function.params[1].name, "arg1");
    assert_eq!(function.params[1].ty, Type::I64);
    assert_eq!(selected.substituted_params.len(), 1);
    assert_eq!(selected.substituted_params[0].ty, Type::I64);
}

#[test]
fn selection_service_selects_trait_impl_with_trait_args() {
    let owner = def_id(60);
    let trait_id = def_id(61);
    let impl_id = def_id(62);
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Bag".to_string()),
        type_name: "Bag".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner,
            args: Vec::new(),
        }),
        trait_name: Some("Index".to_string()),
        trait_id: Some(trait_id),
        trait_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "Idx",
        )],
        trait_arg_types: vec![Type::I64],
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::new(),
    };
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let impls = HashMap::from([(imp.id, imp)]);
    let mut resolver = ResolverTables::default();
    resolver.item_names_by_id.insert(owner, "Bag".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver_ty = Type::Struct {
        id: owner,
        args: Vec::new(),
    };

    let selected = service
        .select_trait_impl(&receiver_ty, trait_id, &[Type::I64])
        .expect("expected trait impl selection");

    assert_eq!(selected.id, impl_id);
    assert!(service
        .select_trait_impl(&receiver_ty, trait_id, &[Type::Bool])
        .is_err());
}

#[test]
fn selection_service_rejects_equally_specific_trait_impls() {
    let owner = def_id(600);
    let trait_id = def_id(601);
    let impl_template = HirImpl {
        id: def_id(602),
        owner: HirImplOwner::Named("Bag".to_string()),
        type_name: "Bag".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner,
            args: Vec::new(),
        }),
        trait_name: Some("Marker".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::new(),
    };
    let mut second = impl_template.clone();
    second.id = def_id(603);
    let impls = HashMap::from([(impl_template.id, impl_template), (second.id, second)]);
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let mut resolver = ResolverTables::default();
    resolver.item_names_by_id.insert(owner, "Bag".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);

    assert!(service
        .select_trait_impl(
            &Type::Struct {
                id: owner,
                args: Vec::new(),
            },
            trait_id,
            &[],
        )
        .is_err());
}

#[test]
fn selection_service_rejects_differently_specific_trait_impls() {
    let owner = def_id(610);
    let trait_id = def_id(611);
    let generic_impl_id = def_id(612);
    let generic = GenericParamId {
        owner: generic_impl_id,
        index: 0,
    };
    let generic_impl = HirImpl {
        id: generic_impl_id,
        owner: HirImplOwner::Named("T".to_string()),
        type_name: "T".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: generic_impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: vec![Type::Generic(generic)].into(),
        trait_name: Some("Marker".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::new(),
    };
    let concrete_impl = HirImpl {
        id: def_id(613),
        owner: HirImplOwner::Named("Bag".to_string()),
        type_name: "Bag".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner,
            args: Vec::new(),
        }),
        trait_name: Some("Marker".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::new(),
    };
    let impls = HashMap::from([
        (generic_impl.id, generic_impl),
        (concrete_impl.id, concrete_impl),
    ]);
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let mut resolver = ResolverTables::default();
    resolver.item_names_by_id.insert(owner, "Bag".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);

    let error = service
        .select_trait_impl(
            &Type::Struct {
                id: owner,
                args: Vec::new(),
            },
            trait_id,
            &[],
        )
        .unwrap_err();
    assert_eq!(
            error,
            crate::selection::SelectionDiagnostic::AmbiguousCandidates {
                operation: "the requested trait (upstream coherence invariant violation: overlapping impls reached selection)".to_string(),
                receiver: Type::Struct {
                    id: owner,
                    args: Vec::new(),
                },
                candidates: vec![generic_impl_id, def_id(613)],
            }
        );
}

#[test]
fn selection_service_selects_generic_fixed_array_builtin_slice_impl() {
    let impl_id = def_id(20);
    let trait_id = def_id(30);
    let method_id = def_id(21);
    let generic_t = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let mut method = method(method_id, impl_id);
    method.name = "echo".to_string();
    method.params[0].ty = Type::Array(Box::new(Type::Generic(generic_t)), 3);
    method.params.push(HirParam {
        name: "value".to_string(),
        local_id: crate::ids::HirLocalId(1),
        ty: Type::I64,
        mutable: false,
        is_ref: false,
    });

    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::BuiltinSlice,
        type_name: "[T; 3]".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: HirImplReceiverPattern::SliceFamily {
            element: Type::Generic(generic_t),
        },
        trait_name: Some("Echo".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("echo".to_string(), method)]),
    };
    let mut traits = HashMap::new();
    register_trait_member(&mut traits, &imp, "echo");
    let impls = HashMap::from([(imp.id, imp)]);
    let current_impl_bounds = HirGenericBounds::new();
    let effective = effective_trait_methods_for(&traits, &impls);
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
        .with_effective_trait_methods(&effective);
    let receiver = HirExpr {
        kind: HirExprKind::Var("arr".to_string()),
        ty: Type::Array(Box::new(Type::I64), 3),
        span: Span::test(),
    };

    let target = service
        .select_concrete_method(&[receiver_candidate(receiver)], "echo", |ty| ty.clone())
        .expect("expected fixed-array impl selection")
        .target;

    assert_eq!(target.impl_id(), Some(impl_id));
    assert_eq!(target.trait_id(), Some(trait_id));
    assert_eq!(target.method_id(), Some(method_id));
}
