use super::*;

#[test]
fn selection_service_selects_required_trait_member_by_exact_id() {
    let trait_id = def_id(10);
    let member_id = def_id(11);
    let impl_id = def_id(12);
    let impl_method_id = def_id(13);
    let (traits, impls, receiver) =
        renamed_trait_member_fixture(trait_id, member_id, "split", impl_id, impl_method_id);
    let effective = HashMap::from([((impl_id, member_id), impl_method_id)]);
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds)
        .with_effective_trait_methods(&effective);

    let selected = service
        .select_required_trait_member(&receiver, &receiver.ty, trait_id, member_id)
        .expect("expected exact trait-member selection");

    match &selected.target.target {
        crate::hir::HirSelectedMethodTarget::ImplMethod {
            impl_id: selected_impl_id,
            method_id,
            selected_trait: Some(selected_trait),
        } => {
            assert_eq!(*selected_impl_id, impl_id);
            assert_eq!(*method_id, impl_method_id);
            assert_eq!(selected_trait.trait_id, trait_id);
            assert_eq!(selected_trait.member_id, member_id);
        }
        target => panic!("expected trait impl target, found {target:?}"),
    }
    assert_eq!(
        selected.function.as_ref().map(|function| function.id),
        Some(impl_method_id)
    );
}

#[test]
fn selection_service_rejects_trait_method_without_effective_mapping() {
    let trait_id = def_id(14);
    let member_id = def_id(15);
    let impl_id = def_id(16);
    let impl_method_id = def_id(17);
    let (traits, impls, receiver) =
        renamed_trait_member_fixture(trait_id, member_id, "split", impl_id, impl_method_id);
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds);

    let result = service.select_required_trait_method(&receiver, &receiver.ty, trait_id, "split");

    assert!(matches!(
        result,
        Err(SelectionDiagnostic::NoImplementation { .. })
    ));
}

#[test]
fn selection_service_selects_static_trait_member_by_exact_id_and_trait_args() {
    let trait_id = def_id(20);
    let member_id = def_id(21);
    let impl_id = def_id(22);
    let impl_method_id = def_id(23);
    let (mut traits, mut impls, mut owner) =
        renamed_trait_member_fixture(trait_id, member_id, "recover", impl_id, impl_method_id);
    traits
        .get_mut(&trait_id)
        .expect("fixture trait")
        .methods
        .get_mut("recover")
        .expect("fixture trait member")
        .is_method = false;
    traits
        .get_mut(&trait_id)
        .expect("fixture trait")
        .methods
        .get_mut("recover")
        .expect("fixture trait member")
        .self_receiver = None;
    let imp = impls.get_mut(&impl_id).expect("fixture impl");
    imp.trait_arg_types = vec![Type::I64];
    let impl_method = imp.methods.get_mut("recover").expect("fixture impl method");
    impl_method.is_method = false;
    impl_method.self_receiver = None;
    owner.kind = HirExprKind::Unit;
    let effective = HashMap::from([((impl_id, member_id), impl_method_id)]);
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds)
        .with_effective_trait_methods(&effective);

    let selected = service
        .select_static_trait_member(&owner, &owner.ty, trait_id, &[Type::I64], member_id)
        .expect("expected exact static trait-member selection");

    match &selected.target.target {
        crate::hir::HirSelectedMethodTarget::ImplMethod {
            impl_id: selected_impl_id,
            method_id,
            selected_trait: Some(selected_trait),
        } => {
            assert_eq!(*selected_impl_id, impl_id);
            assert_eq!(*method_id, impl_method_id);
            assert_eq!(selected_trait.member_id, member_id);
            assert_eq!(selected_trait.trait_args, vec![Type::I64]);
        }
        target => panic!("expected trait impl target, found {target:?}"),
    }
    assert_eq!(
        selected.function.as_ref().map(|function| function.id),
        Some(impl_method_id)
    );
}

#[test]
fn selection_service_does_not_fallback_to_same_named_impl_method_without_exact_mapping() {
    let trait_id = def_id(30);
    let member_id = def_id(31);
    let impl_id = def_id(32);
    let impl_method_id = def_id(33);
    let (traits, impls, receiver) =
        renamed_trait_member_fixture(trait_id, member_id, "split", impl_id, impl_method_id);
    let effective = HashMap::new();
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds)
        .with_effective_trait_methods(&effective);

    assert!(matches!(
        service.select_required_trait_member(&receiver, &receiver.ty, trait_id, member_id),
        Err(SelectionDiagnostic::NoImplementation { .. })
    ));
}

#[test]
fn selection_service_allows_exact_trait_default_through_effective_mapping() {
    let trait_id = def_id(40);
    let member_id = def_id(41);
    let impl_id = def_id(42);
    let impl_method_id = def_id(43);
    let (traits, mut impls, receiver) =
        renamed_trait_member_fixture(trait_id, member_id, "split", impl_id, impl_method_id);
    impls
        .get_mut(&impl_id)
        .expect("fixture impl")
        .methods
        .clear();
    let effective = HashMap::from([((impl_id, member_id), member_id)]);
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds)
        .with_effective_trait_methods(&effective);

    let selected = service
        .select_required_trait_member(&receiver, &receiver.ty, trait_id, member_id)
        .expect("expected exact default selection");

    assert_eq!(selected.target.method_id(), Some(member_id));
    assert_eq!(
        selected.function.as_ref().map(|function| function.id),
        Some(member_id)
    );
}

#[test]
fn selection_service_rejects_same_spelled_member_from_another_trait() {
    let trait_id = def_id(50);
    let member_id = def_id(51);
    let impl_id = def_id(52);
    let impl_method_id = def_id(53);
    let (mut traits, impls, receiver) =
        renamed_trait_member_fixture(trait_id, member_id, "split", impl_id, impl_method_id);
    let other_trait_id = def_id(54);
    let other_member_id = def_id(55);
    let mut other_member = method(other_member_id, def_id(9));
    other_member.name = "split".to_string();
    traits.insert(
        other_trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: other_trait_id,
            name: "OtherProtocol".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("split".to_string(), other_member)]),
            signatures: HashMap::new(),
        },
    );
    let effective = HashMap::from([((impl_id, other_member_id), impl_method_id)]);
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds)
        .with_effective_trait_methods(&effective);

    assert!(matches!(
        service.select_required_trait_member(&receiver, &receiver.ty, trait_id, member_id),
        Err(SelectionDiagnostic::NoImplementation { .. })
    ));
}

#[test]
fn selection_service_reports_invalid_or_foreign_trait_member_ids_without_panicking() {
    let trait_id = def_id(60);
    let member_id = def_id(61);
    let impl_id = def_id(62);
    let impl_method_id = def_id(63);
    let (mut traits, impls, receiver) =
        renamed_trait_member_fixture(trait_id, member_id, "split", impl_id, impl_method_id);
    let foreign_trait_id = def_id(64);
    let foreign_member_id = def_id(65);
    let mut foreign_member = method(foreign_member_id, def_id(9));
    foreign_member.name = "split".to_string();
    traits.insert(
        foreign_trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: foreign_trait_id,
            name: "ForeignProtocol".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("split".to_string(), foreign_member)]),
            signatures: HashMap::new(),
        },
    );
    let effective = HashMap::from([((impl_id, member_id), impl_method_id)]);
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds)
        .with_effective_trait_methods(&effective);

    for invalid_member_id in [def_id(66), foreign_member_id] {
        let error = match service.select_required_trait_member(
            &receiver,
            &receiver.ty,
            trait_id,
            invalid_member_id,
        ) {
            Ok(_) => panic!("invalid trait member ID selected a method"),
            Err(error) => error,
        };
        let message = error.message();
        assert_eq!(
            message,
            "Trait '<unknown trait>' does not declare the selected member"
        );
        assert!(!message.contains("DefId"));
    }
}

#[test]
fn trait_lookup_does_not_select_payload_under_a_different_map_key() {
    let trait_id = def_id(1);
    let wrong_key = def_id(2);
    let traits = HashMap::from([(
        wrong_key,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Reader".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    )]);
    let impls: HashMap<DefId, HirImpl> = HashMap::new();
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds);

    assert!(service.trait_by_id(trait_id).is_none());
}

#[test]
fn builtin_sized_identity_uses_marked_trait_id() {
    let marked_sized_id = def_id(26);
    let unmarked_id = def_id(27);
    let traits = HashMap::new();
    let impls = HashMap::new();
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, Some(marked_sized_id), None, &bounds);

    assert!(service.is_builtin_sized_trait(marked_sized_id));
    assert!(!service.is_builtin_sized_trait(unmarked_id));
}

#[test]
fn impl_method_does_not_select_trait_default_under_a_different_map_key() {
    let owner = def_id(3);
    let trait_id = def_id(4);
    let wrong_key = def_id(5);
    let default_method = method(def_id(6), owner);
    let traits = HashMap::from([(
        wrong_key,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Reader".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("value".to_string(), default_method)]),
            signatures: HashMap::new(),
        },
    )]);
    let imp = HirImpl {
        id: def_id(7),
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner,
            args: Vec::new(),
        }),
        trait_name: Some("Reader".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::new(),
    };
    let impls = HashMap::from([(imp.id, imp.clone())]);
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds);

    assert!(service.impl_method(&imp, "value").is_none());
}

#[test]
fn trait_impl_method_absent_from_trait_is_not_selected() {
    let owner = def_id(5);
    let trait_id = def_id(6);
    let impl_id = def_id(7);
    let method_id = def_id(8);
    let traits = HashMap::from([(
        trait_id,
        crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Marker".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    )]);
    let impls: HashMap<DefId, HirImpl> = vec![HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
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
        methods: HashMap::from([("value".to_string(), method(method_id, owner))]),
    }]
    .into_iter()
    .map(|imp| (imp.id, imp))
    .collect();
    let mut resolver = ResolverTables::default();
    resolver.item_names_by_id.insert(owner, "Box".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver = receiver_candidate(HirExpr {
        ty: Type::Struct {
            id: owner,
            args: Vec::new(),
        },
        kind: HirExprKind::Var("box".to_string()),
        span: Span::test(),
    });

    assert!(matches!(
        service.select_concrete_method(&[receiver], "value", Type::clone),
        Err(SelectionDiagnostic::NoImplementation { .. })
    ));
    let receiver = HirExpr {
        ty: Type::Struct {
            id: owner,
            args: Vec::new(),
        },
        kind: HirExprKind::Var("box".to_string()),
        span: Span::test(),
    };
    assert!(matches!(
        service.select_required_trait_method(&receiver, &receiver.ty, trait_id, "value"),
        Err(SelectionDiagnostic::TraitMemberMissing {
            trait_id: missing_trait_id,
            method_name,
        }) if missing_trait_id == trait_id && method_name == "value"
    ));
}

#[test]
fn selection_service_selects_concrete_impl_method_by_identity() {
    let owner = def_id(10);
    let impl_id = def_id(20);
    let method_id = def_id(21);
    let method = method(method_id, owner);
    let imp = HirImpl {
        id: impl_id,
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
        methods: HashMap::from([("value".to_string(), method)]),
    };
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let impls = HashMap::from([(imp.id, imp)]);
    let mut resolver = ResolverTables::default();
    resolver.item_names_by_id.insert(owner, "Box".to_string());
    let current_impl_bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: Type::Struct {
            id: owner,
            args: Vec::new(),
        },
        span: Span::test(),
    };

    let selected = service
        .select_concrete_method(&[receiver_candidate(receiver)], "value", |ty| ty.clone())
        .expect("expected selected method");

    assert_eq!(selected.target.impl_id(), Some(impl_id));
    assert_eq!(selected.target.method_id(), Some(method_id));
    assert_eq!(selected.return_type, Type::I64);
}
