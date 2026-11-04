use super::*;

#[test]
fn bound_method_selection_preserves_distinct_trait_arguments() {
    let trait_id = def_id(10);
    let method_id = def_id(11);
    let rhs_param = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let self_param = GenericParamId {
        owner: trait_id,
        index: 1,
    };
    let mut compare = operator_method(method_id, trait_id, Type::Bool);
    compare.name = "compare".to_string();
    compare.params[0].ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Generic(self_param)),
    };
    compare.params[1].ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Generic(rhs_param)),
    };
    let traits = HashMap::from([(
        trait_id,
        HirTrait {
            id: trait_id,
            name: "Compare".to_string(),
            generic_params: vec![GenericParamDecl::type_param(rhs_param, "Rhs")],
            target: None,
            predicates: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("compare".to_string(), compare)]),
            signatures: HashMap::new(),
        },
    )]);
    let impls = HashMap::new();
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver_ty = Type::Generic(GenericParamId {
        owner: def_id(20),
        index: 0,
    });
    let receivers = [receiver_candidate(HirExpr {
        kind: HirExprKind::Var("left".to_string()),
        ty: receiver_ty.clone(),
        span: Span::test(),
    })];
    let bounds = [Type::I64, Type::Bool].map(|rhs| TraitBound {
        trait_id,
        type_args: vec![rhs],
    });
    assert!(matches!(
        service.select_bound_method(&receivers, &bounds, "compare", receiver_ty.clone()),
        Err(SelectionDiagnostic::AmbiguousCandidates { .. }),
    ));
    for rhs in [Type::I64, Type::Bool] {
        let expected = Type::Reference {
            mutable: false,
            inner: Box::new(rhs.clone()),
        };
        let selected = service
            .select_bound_method_matching(
                &receivers,
                &bounds,
                "compare",
                receiver_ty.clone(),
                false,
                |candidate| candidate.substituted_params[0].ty == expected,
            )
            .expect("right-hand argument should select its exact trait instantiation");
        assert_eq!(selected.target.method_id(), Some(method_id));
        assert_eq!(selected.target.trait_args(), &[rhs]);
    }
}

#[test]
fn selection_service_bound_signature_uses_canonical_params_without_duplicate_self() {
    let trait_id = def_id(14);
    let method_id = def_id(15);
    let self_param = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let self_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Generic(self_param)),
    };
    let traits = HashMap::from([(
        trait_id,
        crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "show".to_string(),
                crate::hir::HirFunctionSig {
                    id: method_id,
                    name: "show".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(self_param, "Self")],
                    params: vec![self_ty.clone()],
                    ret: Type::I64,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        },
    )]);
    let impls: HashMap<DefId, HirImpl> = HashMap::new();
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("value".to_string()),
        ty: Type::Generic(self_param),
        span: Span::test(),
    };

    let selected = service
        .select_bound_method(
            &[receiver_candidate(receiver)],
            &[crate::types::TraitBound {
                trait_id,
                type_args: Vec::new(),
            }],
            "show",
            Type::Generic(self_param),
        )
        .expect("expected bound signature selection");
    let function = selected
        .function
        .expect("selection should fabricate function");

    assert_eq!(function.params.len(), 1);
    assert_eq!(function.params[0].name, "self");
    assert_eq!(function.params[0].ty, self_ty);
    assert!(selected.substituted_params.is_empty());
}

#[test]
fn selection_service_bound_mut_receiver_uses_autoref_mut_candidate() {
    let trait_id = def_id(18);
    let method_id = def_id(19);
    let self_param = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let traits = HashMap::from([(
        trait_id,
        crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Touch".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "touch".to_string(),
                crate::hir::HirFunctionSig {
                    id: method_id,
                    name: "touch".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(self_param, "Self")],
                    params: vec![Type::Reference {
                        mutable: true,
                        inner: Box::new(Type::Generic(self_param)),
                    }],
                    ret: Type::Unit,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Mut),
                    is_unsafe: false,
                },
            )]),
        },
    )]);
    let impls: HashMap<DefId, HirImpl> = HashMap::new();
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("value".to_string()),
        ty: Type::Generic(self_param),
        span: Span::test(),
    };

    let selected = service
        .select_bound_method(
            &[ReceiverCandidate {
                expr: receiver,
                adjustment: ReceiverAdjustment::None,
                can_autoref_mut: true,
            }],
            &[crate::types::TraitBound {
                trait_id,
                type_args: Vec::new(),
            }],
            "touch",
            Type::Generic(self_param),
        )
        .expect("mutable trait-bound receiver should autoref mut");

    assert_eq!(selected.receiver_adjustment, ReceiverAdjustment::AutorefMut);
}

#[test]
fn selection_service_bound_shared_receiver_on_ref_uses_deref_candidate() {
    let trait_id = def_id(20);
    let method_id = def_id(21);
    let self_param = GenericParamId {
        owner: trait_id,
        index: 0,
    };
    let traits = HashMap::from([(
        trait_id,
        crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Readable".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "read".to_string(),
                crate::hir::HirFunctionSig {
                    id: method_id,
                    name: "read".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(self_param, "Self")],
                    params: vec![Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::Generic(self_param)),
                    }],
                    ret: Type::I64,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        },
    )]);
    let impls: HashMap<DefId, HirImpl> = HashMap::new();
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("value".to_string()),
        ty: Type::Reference {
            mutable: false,
            inner: Box::new(Type::Generic(self_param)),
        },
        span: Span::test(),
    };
    let deref_receiver = HirExpr {
        kind: HirExprKind::Deref(Box::new(receiver.clone())),
        ty: Type::Generic(self_param),
        span: Span::test(),
    };

    let selected = service
        .select_bound_method(
            &[
                ReceiverCandidate {
                    expr: receiver,
                    adjustment: ReceiverAdjustment::None,
                    can_autoref_mut: false,
                },
                ReceiverCandidate {
                    expr: deref_receiver,
                    adjustment: ReceiverAdjustment::BuiltinDeref,
                    can_autoref_mut: false,
                },
            ],
            &[crate::types::TraitBound {
                trait_id,
                type_args: Vec::new(),
            }],
            "read",
            Type::Generic(self_param),
        )
        .expect("shared trait-bound receiver on &T should dereference before borrow");

    assert!(matches!(selected.receiver.kind, HirExprKind::Deref(_)));
    assert_eq!(
        selected.receiver_adjustment,
        ReceiverAdjustment::AutorefShared
    );
}

#[test]
fn selection_service_rejects_bound_signature_receiver_missing_from_params() {
    let trait_id = def_id(16);
    let method_id = def_id(17);
    let traits = HashMap::from([(
        trait_id,
        crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "show".to_string(),
                crate::hir::HirFunctionSig {
                    id: method_id,
                    name: "show".to_string(),
                    generic_params: Vec::new(),
                    params: Vec::new(),
                    ret: Type::I64,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        },
    )]);
    let impls: HashMap<DefId, HirImpl> = HashMap::new();
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("value".to_string()),
        ty: Type::I64,
        span: Span::test(),
    };

    assert!(service
        .select_bound_method(
            &[receiver_candidate(receiver)],
            &[crate::types::TraitBound {
                trait_id,
                type_args: Vec::new(),
            }],
            "show",
            Type::I64,
        )
        .is_err());
}

#[test]
fn selection_service_current_generic_bound_satisfies_impl_bound() {
    let owner = def_id(63);
    let trait_id = def_id(64);
    let bound_trait_id = def_id(65);
    let impl_id = def_id(66);
    let impl_generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let caller_generic = GenericParamId {
        owner: def_id(67),
        index: 0,
    };
    let marker_bound = TraitBound {
        trait_id: bound_trait_id,
        type_args: Vec::new(),
    };
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner,
            args: vec![Type::Generic(impl_generic)],
        }),
        trait_name: Some("Provider".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::from([(
            impl_generic,
            vec![TraitBound {
                trait_id: bound_trait_id,
                type_args: Vec::new(),
            }],
        )])
        .into(),
        methods: HashMap::new(),
    };
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let impls = HashMap::from([(imp.id, imp)]);
    let mut resolver = ResolverTables::default();
    resolver.item_names_by_id.insert(owner, "Box".to_string());
    let current_impl_bounds = HashMap::from([(caller_generic, vec![marker_bound])]).into();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver_ty = Type::Struct {
        id: owner,
        args: vec![Type::Generic(caller_generic)],
    };

    let selected = service
        .select_trait_impl(&receiver_ty, trait_id, &[])
        .expect("matching current generic bound should satisfy impl bound");

    assert_eq!(selected.id, impl_id);
}

#[test]
fn selection_service_strict_trait_impl_rejects_pending_impl_bound() {
    let owner = def_id(68);
    let trait_id = def_id(69);
    let bound_trait_id = def_id(70);
    let impl_id = def_id(71);
    let impl_generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner,
            args: vec![Type::Generic(impl_generic)],
        }),
        trait_name: Some("Provider".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::from([(
            impl_generic,
            vec![TraitBound {
                trait_id: bound_trait_id,
                type_args: Vec::new(),
            }],
        )])
        .into(),
        methods: HashMap::new(),
    };
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let impls = HashMap::from([(imp.id, imp)]);
    let mut resolver = ResolverTables::default();
    resolver.item_names_by_id.insert(owner, "Box".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver_ty = Type::Struct {
        id: owner,
        args: vec![Type::TypeVar(TypeVarId(0))],
    };

    assert!(service
        .select_trait_impl(&receiver_ty, trait_id, &[])
        .is_ok());
    assert!(service
        .select_trait_impl_strict(&receiver_ty, trait_id, &[])
        .is_err());
}

#[test]
fn selection_service_selects_type_var_bound_signature_by_trait_identity() {
    let trait_id = def_id(30);
    let signature_id = def_id(31);
    let trait_def = crate::hir::HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Readable".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "read".to_string(),
            crate::hir::HirFunctionSig {
                id: signature_id,
                name: "read".to_string(),
                generic_params: Vec::new(),
                params: vec![Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::I64),
                }],
                ret: Type::I64,
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(ReceiverMode::Shared),
                is_unsafe: false,
            },
        )]),
    };
    let traits = HashMap::from([(trait_def.id, trait_def)]);
    let impls: HashMap<DefId, HirImpl> = HashMap::new();
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("receiver".to_string()),
        ty: Type::I64,
        span: Span::test(),
    };
    let bounds = vec![crate::types::TraitBound {
        trait_id,
        type_args: Vec::new(),
    }];

    let selected = service
        .select_bound_method(&[receiver_candidate(receiver)], &bounds, "read", Type::I64)
        .expect("expected trait-bound signature selection");

    assert_eq!(selected.target.trait_id(), Some(trait_id));
    assert_eq!(selected.target.method_id(), Some(signature_id));
    assert_eq!(selected.return_type, Type::I64);

    let authority = selected.authority();
    assert_eq!(authority.impl_id(), None);
    assert_eq!(authority.trait_id(), Some(trait_id));
    assert_eq!(authority.method_id(), Some(signature_id));
    assert!(authority.trait_args().is_empty());
    assert_eq!(
        authority.receiver_adjustment,
        ReceiverAdjustment::AutorefShared
    );
}

#[test]
fn selection_records_unresolved_impl_bound_obligation() {
    let owner = def_id(80);
    let trait_id = def_id(81);
    let bound_trait_id = def_id(82);
    let impl_id = def_id(83);
    let method_id = def_id(84);
    let generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let unresolved = TypeVarId(0);
    let mut index_method = method(method_id, owner);
    index_method.name = "index".to_string();
    index_method.params[0].ty = Type::Struct {
        id: owner,
        args: vec![Type::Generic(generic)],
    };
    index_method.params.push(HirParam {
        name: "index".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::I64,
        mutable: false,
        is_ref: false,
    });
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![GenericParamDecl::type_param(generic, "T")],
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner,
            args: vec![Type::Generic(generic)],
        }),
        trait_name: Some("Index".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: vec![Type::I64],
        associated_types: Vec::new(),
        bounds: HashMap::from([(
            generic,
            vec![TraitBound {
                trait_id: bound_trait_id,
                type_args: Vec::new(),
            }],
        )])
        .into(),
        methods: HashMap::from([("index".to_string(), index_method)]),
    };
    let mut traits = HashMap::new();
    let index_member_id = register_trait_member(&mut traits, &imp, "index");
    let impls = HashMap::from([(imp.id, imp)]);
    let mut resolver = ResolverTables::default();
    resolver.item_names_by_id.insert(owner, "Box".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let effective = HashMap::from([((impl_id, index_member_id), method_id)]);
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
        .with_effective_trait_methods(&effective);
    let receiver = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: Type::Struct {
            id: owner,
            args: vec![Type::TypeVar(unresolved)],
        },
        span: Span::test(),
    };

    let selected = service
        .select_required_index_method(
            &[receiver_candidate(receiver)],
            trait_id,
            index_member_id,
            AssocTypeId(0),
            &Type::I64,
            "[]",
            |ty| ty.clone(),
        )
        .expect("unresolved bounded impl should remain selectable");

    assert_eq!(
        selected.pending_impl_bounds,
        vec![(
            Type::TypeVar(unresolved),
            TraitBound {
                trait_id: bound_trait_id,
                type_args: Vec::new(),
            },
        )]
    );
}
