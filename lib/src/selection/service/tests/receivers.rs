use super::*;

#[test]
fn selection_service_rejects_static_member_mapped_to_implicit_receiver_method() {
    let trait_id = def_id(24);
    let member_id = def_id(25);
    let impl_id = def_id(26);
    let impl_method_id = def_id(27);
    let (mut traits, mut impls, mut owner) =
        renamed_trait_member_fixture(trait_id, member_id, "recover", impl_id, impl_method_id);
    let trait_member = traits
        .get_mut(&trait_id)
        .expect("fixture trait")
        .methods
        .get_mut("recover")
        .expect("fixture trait member");
    trait_member.is_method = false;
    trait_member.self_receiver = None;
    let impl_method = impls
        .get_mut(&impl_id)
        .expect("fixture impl")
        .methods
        .get_mut("recover")
        .expect("fixture impl method");
    impl_method.is_method = true;
    impl_method.self_receiver = None;
    owner.kind = HirExprKind::Unit;
    let effective = HashMap::from([((impl_id, member_id), impl_method_id)]);
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds)
        .with_effective_trait_methods(&effective);

    assert!(matches!(
        service.select_static_trait_member(&owner, &owner.ty, trait_id, &[], member_id),
        Err(SelectionDiagnostic::NoImplementation { operation, .. }) if operation == "recover"
    ));
}

fn mut_method(id: DefId, owner: DefId) -> HirFunction {
    let mut method = method(id, owner);
    method.name = "inc".to_string();
    method.self_receiver = Some(ReceiverMode::Mut);
    method.params[0].ty = Type::Reference {
        mutable: true,
        inner: Box::new(Type::Struct {
            id: owner,
            args: Vec::new(),
        }),
    };
    method.params[0].mutable = true;
    method.ret_type = Type::Unit;
    method.body.ty = Type::Unit;
    method
}

#[test]
fn selection_rejects_mut_receiver_without_mutable_lvalue() {
    let owner = def_id(10);
    let impl_id = def_id(20);
    let method_id = def_id(21);
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Counter".to_string()),
        type_name: "Counter".to_string(),
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
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("inc".to_string(), mut_method(method_id, owner))]),
    };
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let impls = HashMap::from([(imp.id, imp)]);
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(owner, "Counter".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver_ty = Type::Struct {
        id: owner,
        args: Vec::new(),
    };
    let receiver = HirExpr {
        kind: HirExprKind::Var("counter".to_string()),
        ty: receiver_ty.clone(),
        span: Span::test(),
    };
    let shared_receiver = HirExpr {
        kind: HirExprKind::Var("counter_ref".to_string()),
        ty: Type::Reference {
            mutable: false,
            inner: Box::new(receiver_ty),
        },
        span: Span::test(),
    };

    assert!(service
        .select_concrete_method(&[receiver_candidate(receiver.clone())], "inc", |ty| ty
            .clone())
        .is_err());
    assert!(service
        .select_concrete_method(
            &[ReceiverCandidate {
                expr: shared_receiver,
                adjustment: ReceiverAdjustment::None,
                can_autoref_mut: false,
            }],
            "inc",
            |ty| ty.clone(),
        )
        .is_err());
    let selected = service
        .select_concrete_method(
            &[ReceiverCandidate {
                expr: receiver,
                adjustment: ReceiverAdjustment::None,
                can_autoref_mut: true,
            }],
            "inc",
            |ty| ty.clone(),
        )
        .expect("mutable lvalue should match mut receiver");

    assert_eq!(selected.receiver_adjustment, ReceiverAdjustment::AutorefMut);
}

#[test]
fn selection_uses_mut_to_shared_ref_for_reference_owned_impl() {
    let trait_id = def_id(30);
    let impl_id = def_id(40);
    let method_id = def_id(41);
    let method = HirFunction {
        id: method_id,
        name: "show".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(Type::Str),
            },
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Str,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Str,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Shared),
        is_unsafe: false,
    };
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("&Str".to_string()),
        type_name: "&Str".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        }),
        trait_name: Some("Show".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("show".to_string(), method)]),
    };
    let mut traits = HashMap::new();
    register_trait_member(&mut traits, &imp, "show");
    let impls = HashMap::from([(imp.id, imp)]);
    let resolver = ResolverTables::default();
    let current_impl_bounds = HirGenericBounds::new();
    let effective = effective_trait_methods_for(&traits, &impls);
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
        .with_effective_trait_methods(&effective);
    let receiver = HirExpr {
        kind: HirExprKind::Var("s".to_string()),
        ty: Type::Reference {
            mutable: true,
            inner: Box::new(Type::Str),
        },
        span: Span::test(),
    };

    let selected = service
        .select_concrete_method(
            &[ReceiverCandidate {
                expr: receiver,
                adjustment: ReceiverAdjustment::None,
                can_autoref_mut: false,
            }],
            "show",
            |ty| ty.clone(),
        )
        .expect("&mut Str should match &Str reference-owned impl");

    assert_eq!(
        selected.receiver_adjustment,
        ReceiverAdjustment::MutToSharedRef
    );
}

#[test]
fn selection_autorefs_value_for_reference_owned_shared_method() {
    let trait_id = def_id(30);
    let impl_id = def_id(40);
    let method_id = def_id(41);
    let method = HirFunction {
        id: method_id,
        name: "show".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Str),
                }),
            },
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Str,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Str,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Shared),
        is_unsafe: false,
    };
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("&Str".to_string()),
        type_name: "&Str".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        }),
        trait_name: Some("Show".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("show".to_string(), method)]),
    };
    let mut traits = HashMap::new();
    register_trait_member(&mut traits, &imp, "show");
    let impls = HashMap::from([(imp.id, imp)]);
    let resolver = ResolverTables::default();
    let current_impl_bounds = HirGenericBounds::new();
    let effective = effective_trait_methods_for(&traits, &impls);
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
        .with_effective_trait_methods(&effective);
    let receiver = HirExpr {
        kind: HirExprKind::Var("s".to_string()),
        ty: Type::Str,
        span: Span::test(),
    };

    let selected = service
        .select_concrete_method(&[receiver_candidate(receiver)], "show", |ty| ty.clone())
        .expect("Str should autoref to the &Str impl owner before shared self borrow");

    assert_eq!(
        selected.receiver_adjustment,
        ReceiverAdjustment::AutorefShared
    );
}

#[test]
fn selection_authority_selects_same_named_method_by_receiver_identity() {
    let left_owner = def_id(100);
    let right_owner = def_id(101);
    let left_impl_id = def_id(110);
    let right_impl_id = def_id(111);
    let left_method_id = def_id(120);
    let right_method_id = def_id(121);
    let impls: HashMap<DefId, HirImpl> = vec![
        HirImpl {
            id: left_impl_id,
            owner: HirImplOwner::Named("left::Box".to_string()),
            type_name: "left::Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: left_owner,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method(left_method_id, left_owner))]),
        },
        HirImpl {
            id: right_impl_id,
            owner: HirImplOwner::Named("right::Box".to_string()),
            type_name: "right::Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: right_owner,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method(right_method_id, right_owner))]),
        },
    ]
    .into_iter()
    .map(|imp| (imp.id, imp))
    .collect();
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(left_owner, "left::Box".to_string());
    resolver
        .item_names_by_id
        .insert(right_owner, "right::Box".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: Type::Struct {
            id: right_owner,
            args: Vec::new(),
        },
        span: Span::test(),
    };

    let authority = service
        .select_concrete_method(&[receiver_candidate(receiver)], "value", |ty| ty.clone())
        .expect("expected right-side selected method")
        .authority();

    assert_eq!(authority.impl_id(), Some(right_impl_id));
    assert_eq!(authority.method_id(), Some(right_method_id));
}

#[test]
fn selection_service_rejects_shared_receiver_display_name_collision() {
    let left_owner = def_id(130);
    let right_owner = def_id(131);
    let left_impl_id = def_id(132);
    let right_impl_id = def_id(133);
    let left_method_id = def_id(134);
    let right_method_id = def_id(135);
    let impls: HashMap<DefId, HirImpl> = vec![
        HirImpl {
            id: left_impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: left_owner,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method(left_method_id, left_owner))]),
        },
        HirImpl {
            id: right_impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: right_owner,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method(right_method_id, right_owner))]),
        },
    ]
    .into_iter()
    .map(|imp| (imp.id, imp))
    .collect();
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(right_owner, "Box".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: Type::Struct {
            id: right_owner,
            args: Vec::new(),
        },
        span: Span::test(),
    };

    let authority = service
        .select_concrete_method(&[receiver_candidate(receiver)], "value", |ty| ty.clone())
        .expect("expected selected method by receiver identity")
        .authority();

    assert_eq!(authority.impl_id(), Some(right_impl_id));
    assert_eq!(authority.method_id(), Some(right_method_id));
}

#[test]
fn selection_service_rejects_trait_receiver_display_name_collision() {
    let left_owner = def_id(136);
    let right_owner = def_id(137);
    let trait_id = def_id(138);
    let left_impl_id = def_id(139);
    let right_impl_id = def_id(140);
    let left_method_id = def_id(141);
    let right_method_id = def_id(142);
    let receiver_ty = Type::Struct {
        id: right_owner,
        args: Vec::new(),
    };
    let impls: HashMap<DefId, HirImpl> = vec![
        HirImpl {
            id: left_impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: left_owner,
                args: Vec::new(),
            }),
            trait_name: Some("Add".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "+".to_string(),
                operator_method(left_method_id, left_owner, Type::I64),
            )]),
        },
        HirImpl {
            id: right_impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: right_owner,
                args: Vec::new(),
            }),
            trait_name: Some("Add".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "+".to_string(),
                operator_method(right_method_id, right_owner, Type::I64),
            )]),
        },
    ]
    .into_iter()
    .map(|imp| (imp.id, imp))
    .collect();
    let mut traits = HashMap::new();
    register_trait_member(&mut traits, impls.get(&right_impl_id).unwrap(), "+");
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(right_owner, "Box".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let effective = effective_trait_methods_for(&traits, &impls);
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
        .with_effective_trait_methods(&effective);
    let receiver = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: receiver_ty.clone(),
        span: Span::test(),
    };

    let authority = service
        .select_required_trait_method(&receiver, &receiver_ty, trait_id, "+")
        .expect("expected selected trait method by receiver identity")
        .authority();

    assert_eq!(authority.impl_id(), Some(right_impl_id));
    assert_eq!(authority.method_id(), Some(right_method_id));
}

#[test]
fn selection_authority_uses_dependency_resolver_for_artifact_backed_receiver() {
    let dependency_owner = DefId::new(CrateId(7), LocalDefId(3));
    let impl_id = DefId::new(CrateId(7), LocalDefId(4));
    let method_id = DefId::new(CrateId(7), LocalDefId(5));
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("dep::Box".to_string()),
        type_name: "dep::Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: dependency_owner,
            args: Vec::new(),
        }),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("value".to_string(), method(method_id, dependency_owner))]),
    };
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let impls = HashMap::from([(imp.id, imp)]);
    let resolver = ResolverTables::default();
    let mut dependency_resolver = ResolverTables::default();
    dependency_resolver
        .item_names_by_id
        .insert(dependency_owner, "dep::Box".to_string());
    let dependency_resolvers = HashMap::from([("dep".to_string(), dependency_resolver)]);
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: Type::Struct {
            id: dependency_owner,
            args: Vec::new(),
        },
        span: Span::test(),
    };

    let authority = service
        .select_concrete_method(&[receiver_candidate(receiver)], "value", |ty| ty.clone())
        .expect("expected dependency-backed selected method")
        .authority();

    assert_eq!(authority.impl_id(), Some(impl_id));
    assert_eq!(authority.method_id(), Some(method_id));
}

#[test]
fn selection_service_rejects_unrelated_empty_receiver_arg_impl_without_identity() {
    let receiver_owner = def_id(10);
    let unrelated_impl_id = def_id(20);
    let unrelated_method_id = def_id(21);
    let unrelated_method = method(unrelated_method_id, receiver_owner);
    let unrelated_impl = HirImpl {
        id: unrelated_impl_id,
        owner: HirImplOwner::Named("Other".to_string()),
        type_name: "Other".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: def_id(22),
            args: Vec::new(),
        }),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("value".to_string(), unrelated_method)]),
    };
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let impls = HashMap::from([(unrelated_impl.id, unrelated_impl)]);
    let resolver = ResolverTables::default();
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: Type::Struct {
            id: receiver_owner,
            args: Vec::new(),
        },
        span: Span::test(),
    };

    assert!(service
        .select_concrete_method(&[receiver_candidate(receiver)], "value", |ty| ty.clone())
        .is_err());
}

#[test]
fn selection_service_selects_borrowed_receiver_trait_method_for_value_receiver() {
    let owner = def_id(60);
    let trait_id = def_id(61);
    let impl_id = def_id(62);
    let method_id = def_id(63);
    let receiver_ty = Type::Struct {
        id: owner,
        args: Vec::new(),
    };
    let mut method = operator_method(method_id, owner, receiver_ty.clone());
    method.params[0].ty = Type::Reference {
        mutable: false,
        inner: Box::new(receiver_ty.clone()),
    };
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner,
            args: Vec::new(),
        }),
        trait_name: Some("Num".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: vec![receiver_ty.clone()],
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("+".to_string(), method)]),
    };
    let mut traits = HashMap::new();
    register_trait_member(&mut traits, &imp, "+");
    let impls = HashMap::from([(imp.id, imp)]);
    let mut resolver = ResolverTables::default();
    resolver.item_names_by_id.insert(owner, "Box".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let effective = effective_trait_methods_for(&traits, &impls);
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
        .with_effective_trait_methods(&effective);
    let receiver = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: receiver_ty.clone(),
        span: Span::test(),
    };

    let selected = service
        .select_required_trait_method(&receiver, &receiver_ty, trait_id, "+")
        .expect("expected borrowed receiver method to match value receiver");

    let target = &selected.target;
    assert_eq!(target.impl_id(), Some(impl_id));
    assert_eq!(target.method_id(), Some(method_id));
    assert_eq!(selected.return_type, receiver_ty);
}
