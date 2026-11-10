use super::*;

#[test]
fn exact_index_selection_reports_missing_implementation() {
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let impls: HashMap<DefId, HirImpl> = HashMap::new();
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("items".to_string()),
        ty: Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Value(4)),
        span: Span::test(),
    };

    assert!(matches!(
        service.select_required_index_method(
            &[receiver_candidate(receiver)],
            def_id(10),
            def_id(11),
            AssocTypeId(0),
            &Type::I64,
            "[]",
            |ty| ty.clone(),
        ),
        Err(SelectionDiagnostic::NoImplementation { .. })
    ));
}

#[test]
fn selection_service_selects_unary_operator_method_by_required_trait() {
    let owner = def_id(20);
    let trait_id = def_id(30);
    let impl_id = def_id(40);
    let method_id = def_id(41);
    let mut method = operator_method(method_id, owner, Type::I64);
    method.name = "-".to_string();
    method.params.pop();
    let mut traits = HashMap::from([(
        trait_id,
        crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Neg".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    )]);
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Number".to_string()),
        type_name: "Number".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner,
            args: Vec::new(),
        }),
        trait_name: Some("Neg".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("-".to_string(), method)]),
    };
    register_trait_member(&mut traits, &imp, "-");
    let impls = HashMap::from([(imp.id, imp)]);
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(owner, "Number".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let effective = effective_trait_methods_for(&traits, &impls);
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
        .with_effective_trait_methods(&effective);
    let receiver = HirExpr {
        kind: HirExprKind::Var("n".to_string()),
        ty: Type::Struct {
            id: owner,
            args: Vec::new(),
        },
        span: Span::test(),
    };

    let selected = service
        .select_unary_operator_method(&receiver, &receiver.ty, trait_id, "-")
        .expect("expected unary operator selection");

    assert_eq!(
        selected.origin,
        SelectedOrigin::TraitImpl { impl_id, trait_id }
    );
    assert_eq!(selected.target.method_id(), Some(method_id));
    assert_eq!(selected.return_type, Type::I64);
}

#[test]
fn selection_service_records_unresolved_generic_trait_operator_target() {
    let trait_id = def_id(30);
    let method_id = def_id(31);
    let traits = HashMap::from([(
        trait_id,
        crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Num".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "+".to_string(),
                crate::hir::HirFunctionSig {
                    id: method_id,
                    name: "+".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::I64],
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
        ty: Type::TypeVar(TypeVarId(7)),
        span: Span::test(),
    };

    let selected = service
        .select_unresolved_generic_method(receiver, trait_id, "+", Vec::new(), Type::I64)
        .expect("expected unresolved generic operator target");

    assert_eq!(
        selected.origin,
        SelectedOrigin::UnresolvedGeneric { trait_id }
    );
    assert_eq!(selected.target.method_id(), Some(method_id));
    assert_eq!(selected.return_type, Type::I64);
}

#[test]
fn selection_service_records_unresolved_generic_index_target() {
    let trait_id = def_id(30);
    let method_id = def_id(31);
    let traits = HashMap::from([(
        trait_id,
        crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Index".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "index".to_string(),
                crate::hir::HirFunctionSig {
                    id: method_id,
                    name: "index".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::I64],
                    ret: Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::I64),
                    },
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
        kind: HirExprKind::Var("items".to_string()),
        ty: Type::TypeVar(TypeVarId(7)),
        span: Span::test(),
    };
    let return_type = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };

    let selected = service
        .select_unresolved_generic_required_index_method(
            receiver,
            trait_id,
            method_id,
            AssocTypeId(0),
            Type::I64,
            return_type.clone(),
            "[]",
        )
        .expect("expected unresolved generic index target");

    let target = selected.target;
    assert_eq!(target.impl_id(), None);
    assert_eq!(target.trait_id(), Some(trait_id));
    assert_eq!(target.trait_args(), &[Type::I64]);
    assert_eq!(target.method_id(), Some(method_id));
    assert_eq!(selected.return_type, return_type);
}

#[test]
fn index_selection_reports_missing_marked_trait_without_panicking() {
    let traits = HashMap::new();
    let impls = HashMap::new();
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds);
    let trait_id = def_id(40);
    let member_id = def_id(41);
    let receiver = HirExpr {
        kind: HirExprKind::Var("value".to_string()),
        ty: Type::TypeVar(TypeVarId(0)),
        span: Span::test(),
    };

    assert!(matches!(
        service.select_unresolved_generic_required_index_method(
            receiver,
            trait_id,
            member_id,
            AssocTypeId(0),
            Type::I64,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
            "[]",
        ),
        Err(SelectionDiagnostic::TraitMemberIdMissing {
            trait_id: error_trait_id,
            member_id: error_member_id,
        }) if error_trait_id == trait_id && error_member_id == member_id
    ));
}

#[test]
fn selection_service_rejects_index_receiver_display_name_collision() {
    let left_owner = def_id(143);
    let right_owner = def_id(144);
    let trait_id = def_id(145);
    let left_impl_id = def_id(146);
    let right_impl_id = def_id(147);
    let left_method_id = def_id(148);
    let right_method_id = def_id(149);
    let mut left_method = method(left_method_id, left_owner);
    left_method.name = "index".to_string();
    left_method.params.push(HirParam {
        name: "index".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::I64,
        mutable: false,
        is_ref: false,
    });
    let mut right_method = method(right_method_id, right_owner);
    right_method.name = "index".to_string();
    right_method.params.push(HirParam {
        name: "index".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::I64,
        mutable: false,
        is_ref: false,
    });
    let impls: HashMap<DefId, HirImpl> = vec![
        HirImpl {
            id: left_impl_id,
            owner: HirImplOwner::Named("Bag".to_string()),
            type_name: "Bag".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: left_owner,
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
            methods: HashMap::from([("index".to_string(), left_method)]),
        },
        HirImpl {
            id: right_impl_id,
            owner: HirImplOwner::Named("Bag".to_string()),
            type_name: "Bag".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: right_owner,
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
            methods: HashMap::from([("index".to_string(), right_method)]),
        },
    ]
    .into_iter()
    .map(|imp| (imp.id, imp))
    .collect();
    let mut traits = HashMap::new();
    let index_member_id =
        register_trait_member(&mut traits, impls.get(&right_impl_id).unwrap(), "index");
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(right_owner, "Bag".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let effective = HashMap::from([((right_impl_id, index_member_id), right_method_id)]);
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
        .with_effective_trait_methods(&effective);
    let receiver = HirExpr {
        kind: HirExprKind::Var("bag".to_string()),
        ty: Type::Struct {
            id: right_owner,
            args: Vec::new(),
        },
        span: Span::test(),
    };

    let authority = service
        .select_required_index_method(
            &[receiver_candidate(receiver)],
            trait_id,
            index_member_id,
            AssocTypeId(0),
            &Type::I64,
            "[]",
            |ty| ty.clone(),
        )
        .expect("expected selected index method by receiver identity")
        .authority();

    assert_eq!(authority.impl_id(), Some(right_impl_id));
    assert_eq!(authority.method_id(), Some(right_method_id));
}

#[test]
fn selection_service_selects_required_trait_operator_impl() {
    let owner = def_id(10);
    let inherent_impl_id = def_id(20);
    let inherent_method_id = def_id(21);
    let trait_id = def_id(30);
    let trait_impl_id = def_id(40);
    let trait_method_id = def_id(41);
    let inherent_impl = HirImpl {
        id: inherent_impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
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
        methods: HashMap::from([(
            "+".to_string(),
            operator_method(inherent_method_id, owner, Type::Bool),
        )]),
    };
    let trait_impl = HirImpl {
        id: trait_impl_id,
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
        trait_arg_types: vec![Type::Struct {
            id: owner,
            args: Vec::new(),
        }],
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "+".to_string(),
            operator_method(
                trait_method_id,
                owner,
                Type::Struct {
                    id: owner,
                    args: Vec::new(),
                },
            ),
        )]),
    };
    let mut traits = HashMap::new();
    register_trait_member(&mut traits, &trait_impl, "+");
    let impls = HashMap::from([
        (inherent_impl.id, inherent_impl),
        (trait_impl.id, trait_impl),
    ]);
    let mut resolver = ResolverTables::default();
    resolver.item_names_by_id.insert(owner, "Box".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let effective = effective_trait_methods_for(&traits, &impls);
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
        .with_effective_trait_methods(&effective);
    let receiver_ty = Type::Struct {
        id: owner,
        args: Vec::new(),
    };
    let receiver = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: receiver_ty.clone(),
        span: Span::test(),
    };

    let selected = service
        .select_required_trait_method(&receiver, &receiver_ty, trait_id, "+")
        .expect("expected required trait operator selection");

    let target = &selected.target;
    assert_eq!(target.impl_id(), Some(trait_impl_id));
    assert_eq!(target.trait_id(), Some(trait_id));
    assert_eq!(target.trait_args(), &[receiver_ty.clone()]);
    assert_eq!(target.method_id(), Some(trait_method_id));
    assert_eq!(selected.return_type, receiver_ty);

    let authority = selected.authority();
    assert_eq!(authority.impl_id(), Some(trait_impl_id));
    assert_eq!(authority.trait_id(), Some(trait_id));
    assert_eq!(authority.method_id(), Some(trait_method_id));
    assert_eq!(authority.trait_args(), &[receiver_ty.clone()]);
    assert_eq!(authority.return_type, receiver_ty);
}

#[test]
fn selection_service_prefers_user_index_impl_over_builtin_output() {
    let owner = def_id(50);
    let trait_id = def_id(51);
    let assoc_id = crate::ids::AssocTypeId(0);
    let impl_id = def_id(52);
    let method_id = def_id(53);
    let mut index_method = method(method_id, owner);
    index_method.name = "index".to_string();
    index_method.ret_type = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };
    index_method.params.push(HirParam {
        name: "index".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::I64,
        mutable: false,
        is_ref: false,
    });
    let mut traits = HashMap::from([(
        trait_id,
        crate::hir::HirTrait {
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
                id: assoc_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    )]);
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
        methods: HashMap::from([("index".to_string(), index_method)]),
    };
    let index_member_id = register_trait_member(&mut traits, &imp, "index");
    let impls = HashMap::from([(imp.id, imp)]);
    let mut resolver = ResolverTables::default();
    resolver.item_names_by_id.insert(owner, "Bag".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let effective = HashMap::from([((impl_id, index_member_id), method_id)]);
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
        .with_effective_trait_methods(&effective);
    let receiver = HirExpr {
        kind: HirExprKind::Var("bag".to_string()),
        ty: Type::Struct {
            id: owner,
            args: Vec::new(),
        },
        span: Span::test(),
    };

    let selected = service
        .select_required_index_method(
            &[receiver_candidate(receiver)],
            trait_id,
            index_member_id,
            assoc_id,
            &Type::I64,
            "[]",
            |ty| ty.clone(),
        )
        .expect("expected user index impl");

    assert_eq!(selected.target.impl_id(), Some(impl_id));

    let authority = selected.authority();
    assert_eq!(authority.target, selected.target.target);
}

#[test]
fn selection_authority_records_associated_output_return_type() {
    let owner = def_id(70);
    let trait_id = def_id(71);
    let impl_id = def_id(72);
    let method_id = def_id(73);
    let mut method = method(method_id, owner);
    method.name = "next".to_string();
    method.ret_type = Type::Projection {
        ty: Box::new(Type::Struct {
            id: owner,
            args: Vec::new(),
        }),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: crate::ids::AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("IteratorBox".to_string()),
        type_name: "IteratorBox".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner,
            args: Vec::new(),
        }),
        trait_name: Some("Iterator".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: vec![crate::hir::HirAssociatedTypeDef {
            id: crate::ids::AssocTypeId(0),
            name: "Item".to_string(),
            kind: crate::type_services::kind::Kind::Type,
            ty: Type::I64,
        }],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("next".to_string(), method.clone())]),
    };
    let mut traits = HashMap::new();
    register_trait_member(&mut traits, &imp, "next");
    let impls = HashMap::from([(imp.id, imp)]);
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(owner, "IteratorBox".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let effective = effective_trait_methods_for(&traits, &impls);
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
        .with_effective_trait_methods(&effective);
    let receiver = HirExpr {
        kind: HirExprKind::Var("iter".to_string()),
        ty: Type::Struct {
            id: owner,
            args: Vec::new(),
        },
        span: Span::test(),
    };

    let authority = service
        .select_required_trait_method(&receiver, &receiver.ty, trait_id, "next")
        .expect("expected iterator selection")
        .authority();

    assert_eq!(authority.impl_id(), Some(impl_id));
    assert_eq!(authority.trait_id(), Some(trait_id));
    assert_eq!(authority.method_id(), Some(method_id));
    assert_eq!(authority.return_type, method.ret_type);
}

#[test]
fn selection_service_rejects_index_impl_with_conflicting_generic_bindings() {
    let owner = def_id(60);
    let trait_id = def_id(61);
    let impl_id = def_id(62);
    let method_id = def_id(63);
    let generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let mut index_method = method(method_id, owner);
    index_method.name = "index".to_string();
    index_method.params[0].ty = Type::TypeVar(TypeVarId(0));
    index_method.params.push(HirParam {
        name: "index".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Generic(generic),
        mutable: false,
        is_ref: false,
    });
    let traits = HashMap::from([(
        trait_id,
        crate::hir::HirTrait {
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
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    )]);
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Bag".to_string()),
        type_name: "Bag".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner,
            args: vec![Type::Generic(generic)],
        }),
        trait_name: Some("Index".to_string()),
        trait_id: Some(trait_id),
        trait_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "T",
        )],
        trait_arg_types: vec![Type::Generic(generic)],
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("index".to_string(), index_method)]),
    };
    let impls = HashMap::from([(imp.id, imp)]);
    let mut resolver = ResolverTables::default();
    resolver.item_names_by_id.insert(owner, "Bag".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("bag".to_string()),
        ty: Type::Struct {
            id: owner,
            args: vec![Type::I64],
        },
        span: Span::test(),
    };

    assert!(service
        .select_required_index_method(
            &[receiver_candidate(receiver)],
            trait_id,
            method_id,
            AssocTypeId(0),
            &Type::Bool,
            "[]",
            |ty| { ty.clone() }
        )
        .is_err());
}
