use super::*;

#[test]
fn drop_materialization_reports_incomplete_receiver_substitution() {
    let mut mono = Monomorphizer::new();
    let drop_trait_id = def_id(40);
    let drop_member_id = def_id(41);
    let impl_id = def_id(42);
    let method_id = def_id(43);
    let owner_id = def_id(44);
    let method = HirFunction {
        id: method_id,
        name: "drop".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::Unit,
        body: empty_body(Type::Unit),
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };
    let imp = HirImpl {
        id: impl_id,
        owner: crate::hir::HirImplOwner::Named("Wrapper".to_string()),
        type_name: "Wrapper".to_string(),
        type_generics: type_generic(impl_id, "T"),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner_id,
            args: Vec::new(),
        }),
        trait_name: Some("Drop".to_string()),
        trait_id: Some(drop_trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("drop".to_string(), method)]),
    };
    mono.drop_trait_id = Some(drop_trait_id);
    mono.drop_method_id = Some(drop_member_id);
    mono.effective_trait_methods
        .insert((impl_id, drop_member_id), method_id);
    mono.trait_impls.insert(drop_trait_id, vec![imp]);

    mono.monomorphize_drop_for_type(
        &Type::Struct {
            id: owner_id,
            args: Vec::new(),
        },
        None,
    );

    assert!(mono.diagnostics.0.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("could not resolve complete receiver substitution")
    }));
}

#[test]
fn drop_materialization_rejects_differently_specific_matching_impls() {
    let mut mono = Monomorphizer::new();
    let drop_trait_id = def_id(45);
    let drop_member_id = def_id(46);
    let generic_impl_id = def_id(47);
    let generic_method_id = def_id(48);
    let concrete_impl_id = def_id(49);
    let concrete_method_id = def_id(50);
    let owner_id = def_id(51);
    let drop_method = |id| HirFunction {
        id,
        name: "drop".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::Unit,
        body: empty_body(Type::Unit),
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };
    let generic_impl = HirImpl {
        id: generic_impl_id,
        owner: crate::hir::HirImplOwner::Named("T".to_string()),
        type_name: "T".to_string(),
        type_generics: type_generic(generic_impl_id, "T"),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Generic(GenericParamId {
            owner: generic_impl_id,
            index: 0,
        })),
        trait_name: Some("Drop".to_string()),
        trait_id: Some(drop_trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("drop".to_string(), drop_method(generic_method_id))]),
    };
    let concrete_impl = HirImpl {
        id: concrete_impl_id,
        owner: crate::hir::HirImplOwner::Named("Wrapper".to_string()),
        type_name: "Wrapper".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
            id: owner_id,
            args: Vec::new(),
        }),
        trait_name: Some("Drop".to_string()),
        trait_id: Some(drop_trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("drop".to_string(), drop_method(concrete_method_id))]),
    };
    mono.drop_trait_id = Some(drop_trait_id);
    mono.drop_method_id = Some(drop_member_id);
    mono.effective_trait_methods
        .insert((generic_impl_id, drop_member_id), generic_method_id);
    mono.effective_trait_methods
        .insert((concrete_impl_id, drop_member_id), concrete_method_id);
    mono.trait_impls
        .insert(drop_trait_id, vec![generic_impl, concrete_impl]);

    mono.monomorphize_drop_for_type(
        &Type::Struct {
            id: owner_id,
            args: Vec::new(),
        },
        None,
    );

    assert!(mono.diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message.contains("ambiguous Drop implementation")
            && !diagnostic.message.contains("DefId")
            && diagnostic.code == Some(crate::diagnostic::DiagnosticCode::Mono)
    }));
    assert_eq!(mono.instances.len(), 0);
}

fn overlapping_trait_impl(
    impl_id: DefId,
    method_id: DefId,
    trait_id: DefId,
    receiver_pattern: HirImplReceiverPattern,
    method_name: &str,
    self_receiver: Option<ReceiverMode>,
) -> HirImpl {
    let receiver_ty = match &receiver_pattern {
        HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => ty.clone(),
        HirImplReceiverPattern::SliceFamily { element } => Type::Slice(Box::new(element.clone())),
    };
    let params = self_receiver
        .map(|_| {
            vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: receiver_ty,
                mutable: false,
                is_ref: false,
            }]
        })
        .unwrap_or_default();
    let method = HirFunction {
        id: method_id,
        name: method_name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params,
        ret_type: Type::Unit,
        body: empty_body(Type::Unit),
        is_curried: false,
        is_method: self_receiver.is_some(),
        self_receiver,
        is_unsafe: false,
    };
    HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Owner".to_string()),
        type_name: "Owner".to_string(),
        type_generics: if let HirImplReceiverPattern::Exact(Type::Generic(id)) = &receiver_pattern {
            vec![GenericParamDecl::type_param(*id, "T")]
        } else {
            Vec::new()
        },
        receiver_pattern,
        trait_name: Some("Trait".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([(method_name.to_string(), method)]),
    }
}

fn install_overlapping_trait_impls(
    mono: &mut Monomorphizer,
    trait_id: DefId,
    member_id: DefId,
    generic_impl_id: DefId,
    generic_method_id: DefId,
    concrete_impl_id: DefId,
    concrete_method_id: DefId,
    owner_ty: &Type,
    method_name: &str,
    self_receiver: Option<ReceiverMode>,
) {
    let generic_impl = overlapping_trait_impl(
        generic_impl_id,
        generic_method_id,
        trait_id,
        HirImplReceiverPattern::Exact(Type::Generic(GenericParamId {
            owner: generic_impl_id,
            index: 0,
        })),
        method_name,
        self_receiver,
    );
    let concrete_impl = overlapping_trait_impl(
        concrete_impl_id,
        concrete_method_id,
        trait_id,
        HirImplReceiverPattern::Exact(owner_ty.clone()),
        method_name,
        self_receiver,
    );
    mono.effective_trait_methods
        .insert((generic_impl_id, member_id), generic_method_id);
    mono.effective_trait_methods
        .insert((concrete_impl_id, member_id), concrete_method_id);
    mono.trait_impls
        .insert(trait_id, vec![generic_impl, concrete_impl]);
}

fn assert_ambiguous_impl_error(
    error: crate::mono::MonoError,
    trait_id: DefId,
    member_id: DefId,
    expected_impls: &[DefId],
) {
    let crate::mono::MonoErrorKind::AmbiguousImpls {
        trait_id: actual_trait,
        member_id: actual_member,
        impls,
    } = error.kind
    else {
        panic!("expected ambiguous impl error, got {:?}", error.kind);
    };
    assert_eq!(actual_trait, trait_id);
    assert_eq!(actual_member, member_id);
    assert_eq!(impls, expected_impls);
}

#[test]
fn static_method_materialization_rejects_differently_specific_matching_impls() {
    let mut mono = Monomorphizer::new();
    let trait_id = def_id(52);
    let member_id = def_id(53);
    let generic_impl_id = def_id(54);
    let generic_method_id = def_id(55);
    let concrete_impl_id = def_id(56);
    let concrete_method_id = def_id(57);
    let owner_ty = Type::Struct {
        id: def_id(58),
        args: Vec::new(),
    };
    install_overlapping_trait_impls(
        &mut mono,
        trait_id,
        member_id,
        generic_impl_id,
        generic_method_id,
        concrete_impl_id,
        concrete_method_id,
        &owner_ty,
        "make",
        None,
    );
    let target = HirMethodCallTarget::trait_method(
        trait_id,
        member_id,
        Vec::new(),
        crate::hir::HirTraitDispatchKind::TraitBound,
    );

    let error = mono
        .monomorphize_static_method_call(&owner_ty, &target, &[], Span::test())
        .expect_err("overlapping static impls must be ambiguous");

    assert_ambiguous_impl_error(
        error,
        trait_id,
        member_id,
        &[generic_impl_id, concrete_impl_id],
    );
    assert_eq!(mono.instances.len(), 0);
}

#[test]
fn constructor_trait_selection_mono_resolves_deferred_authority_to_instance() {
    let mut mono = Monomorphizer::new();
    let trait_id = def_id(580);
    let member_id = def_id(581);
    let impl_id = def_id(582);
    let method_id = def_id(583);
    let owner_ty = Type::Constructor {
        id: def_id(584),
        flavor: crate::types::NominalTypeKind::Enum,
    };
    let imp = overlapping_trait_impl(
        impl_id,
        method_id,
        trait_id,
        HirImplReceiverPattern::Constructor(owner_ty.clone()),
        "pure",
        None,
    );
    mono.effective_trait_methods
        .insert((impl_id, member_id), method_id);
    mono.trait_impls.insert(trait_id, vec![imp]);
    let target = HirMethodCallTarget::trait_method(
        trait_id,
        member_id,
        Vec::new(),
        crate::hir::HirTraitDispatchKind::TraitBound,
    );

    let (_, call_target, _) = mono
        .monomorphize_static_method_call(&owner_ty, &target, &[], Span::test())
        .expect("deferred constructor authority should resolve");

    assert!(matches!(
        call_target,
        crate::hir::HirCallTarget::Instance(_)
    ));
    assert_eq!(mono.instances.len(), 1);
}

#[test]
fn trait_method_materialization_rejects_differently_specific_matching_impls() {
    let mut mono = Monomorphizer::new();
    let trait_id = def_id(59);
    let member_id = def_id(60);
    let generic_impl_id = def_id(61);
    let generic_method_id = def_id(62);
    let concrete_impl_id = def_id(63);
    let concrete_method_id = def_id(64);
    let owner_ty = Type::Struct {
        id: def_id(65),
        args: Vec::new(),
    };
    install_overlapping_trait_impls(
        &mut mono,
        trait_id,
        member_id,
        generic_impl_id,
        generic_method_id,
        concrete_impl_id,
        concrete_method_id,
        &owner_ty,
        "value",
        Some(ReceiverMode::Move),
    );
    let target = HirMethodCallTarget::trait_method(
        trait_id,
        member_id,
        Vec::new(),
        crate::hir::HirTraitDispatchKind::TraitBound,
    );
    let receiver = HirExpr {
        kind: HirExprKind::Var("owner".to_string()),
        ty: owner_ty.clone(),
        span: Span::test(),
    };
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(receiver.clone()),
            "value".to_string(),
            Vec::new(),
            Some(ReceiverMode::Move),
            target,
        ),
        ty: Type::Unit,
        span: Span::test(),
    };

    let error = mono
        .monomorphize_trait_method_call("value", &[receiver], &mut expr)
        .expect_err("overlapping trait impls must be ambiguous");

    assert_ambiguous_impl_error(
        error,
        trait_id,
        member_id,
        &[generic_impl_id, concrete_impl_id],
    );
    assert_eq!(mono.instances.len(), 0);
    assert!(matches!(expr.kind, HirExprKind::MethodCall(..)));
}
