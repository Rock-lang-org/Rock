use super::*;
use crate::collect::resolver::ResolverTables;
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirFunction, HirImpl, HirImplOwner, HirParam};
use crate::ids::{CrateId, DefId, LocalDefId, TypeVarId};
use crate::lexer::Span;
use crate::types::ReceiverMode;
use crate::types::{GenericParamDecl, GenericParamId, Type};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn receiver_candidate(expr: HirExpr) -> ReceiverCandidate {
    ReceiverCandidate {
        expr,
        adjustment: ReceiverAdjustment::None,
        can_autoref_mut: false,
    }
}

fn method(id: DefId, owner: DefId) -> HirFunction {
    HirFunction {
        id,
        name: "value".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Struct {
                id: owner,
                args: Vec::new(),
            },
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Shared),
        is_unsafe: false,
    }
}

fn operator_method(id: DefId, owner: DefId, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: "+".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![
            HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Struct {
                    id: owner,
                    args: Vec::new(),
                },
                mutable: false,
                is_ref: false,
            },
            HirParam {
                name: "rhs".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Struct {
                    id: owner,
                    args: Vec::new(),
                },
                mutable: false,
                is_ref: false,
            },
        ],
        ret_type: ret_type.clone(),
        body: HirBlock {
            stmts: Vec::new(),
            ty: ret_type,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Shared),
        is_unsafe: false,
    }
}

fn register_trait_member(
    traits: &mut HashMap<DefId, crate::hir::HirTrait>,
    imp: &HirImpl,
    method_name: &str,
) -> DefId {
    let trait_id = imp.trait_id.expect("test trait impl");
    let trait_name = imp.trait_name.clone().expect("test trait name");
    let method = imp.methods.get(method_name).expect("test impl method");
    let member_id = DefId::new(
        method.id.crate_id,
        LocalDefId(
            method
                .id
                .local
                .0
                .checked_add(10_000)
                .expect("test member ID overflow"),
        ),
    );
    let trait_def = traits
        .entry(trait_id)
        .or_insert_with(|| crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: trait_name,
            generic_params: imp.trait_generics.clone(),
            associated_types: imp
                .associated_types
                .iter()
                .map(|assoc| crate::hir::HirAssociatedTypeDecl {
                    id: assoc.id,
                    name: assoc.name.clone(),
                    kind: assoc.kind.clone(),
                })
                .collect(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        });
    assert_eq!(trait_def.id, trait_id);
    trait_def.signatures.insert(
        method_name.to_string(),
        crate::hir::HirFunctionSig {
            id: member_id,
            name: method_name.to_string(),
            generic_params: method.generic_params.clone(),
            params: method.params.iter().map(|param| param.ty.clone()).collect(),
            ret: method.ret_type.clone(),
            generic_bounds: method.generic_bounds.clone(),
            self_receiver: method.self_receiver,
            is_unsafe: method.is_unsafe,
        },
    );
    member_id
}

fn renamed_trait_member_fixture(
    trait_id: DefId,
    member_id: DefId,
    source_name: &str,
    impl_id: DefId,
    impl_method_id: DefId,
) -> (HashMap<DefId, HirTrait>, HashMap<DefId, HirImpl>, HirExpr) {
    let owner_id = def_id(9);
    let receiver_ty = Type::Struct {
        id: owner_id,
        args: Vec::new(),
    };
    let mut trait_method = method(member_id, owner_id);
    trait_method.name = source_name.to_string();
    let trait_def = HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "RenamedProtocol".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([(source_name.to_string(), trait_method)]),
        signatures: HashMap::new(),
    };

    let mut impl_method = method(impl_method_id, owner_id);
    impl_method.name = source_name.to_string();
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Owner".to_string()),
        type_name: "Owner".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(receiver_ty.clone()),
        trait_name: Some("RenamedProtocol".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([(source_name.to_string(), impl_method)]),
    };
    let receiver = HirExpr {
        kind: HirExprKind::Var("owner".to_string()),
        ty: receiver_ty,
        span: Span::test(),
    };

    (
        HashMap::from([(trait_id, trait_def)]),
        HashMap::from([(impl_id, imp)]),
        receiver,
    )
}

fn effective_trait_methods_for(
    traits: &HashMap<DefId, HirTrait>,
    impls: &HashMap<DefId, HirImpl>,
) -> HashMap<(DefId, DefId), DefId> {
    impls
        .values()
        .filter_map(|imp| {
            let trait_id = imp.trait_id?;
            let trait_def = traits.get(&trait_id)?;
            Some(
                imp.methods
                    .iter()
                    .filter_map(|(name, method)| {
                        let member_id = trait_def
                            .methods
                            .get(name)
                            .map(|member| member.id)
                            .or_else(|| {
                                trait_def.signatures.get(name).map(|signature| signature.id)
                            })?;
                        Some(((imp.id, member_id), method.id))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
}

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
    let resolver = ResolverTables::default();
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
    let resolver = ResolverTables::default();
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
fn exact_index_selection_reports_missing_implementation() {
    let traits: HashMap<DefId, HirTrait> = HashMap::new();
    let impls: HashMap<DefId, HirImpl> = HashMap::new();
    let bounds = HirGenericBounds::new();
    let service = SelectionService::new(&traits, &impls, None, None, &bounds);
    let receiver = HirExpr {
        kind: HirExprKind::Var("items".to_string()),
        ty: Type::Array(Box::new(Type::I64), 4),
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
    let resolver = ResolverTables::default();
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
    let resolver = ResolverTables::default();
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
    let resolver = ResolverTables::default();
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
    let resolver = ResolverTables::default();
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
    let resolver = ResolverTables::default();
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
    let resolver = ResolverTables::default();
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
    let resolver = ResolverTables::default();
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
    let resolver = ResolverTables::default();
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
    let resolver = ResolverTables::default();
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
    let resolver = ResolverTables::default();
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

#[test]
fn supertrait_bound_implies_parent_bound_by_trait_id() {
    let functor_id = def_id(200);
    let applicative_id = def_id(201);
    let caller_id = def_id(202);
    let unary = crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    );
    let functor_target = GenericParamDecl::new(
        GenericParamId {
            owner: functor_id,
            index: 0,
        },
        "F",
        unary.clone(),
    );
    let applicative_target = GenericParamDecl::new(
        GenericParamId {
            owner: applicative_id,
            index: 0,
        },
        "F",
        unary,
    );
    let traits = HashMap::from([
        (
            functor_id,
            HirTrait {
                id: functor_id,
                name: "pkg::Functor".to_string(),
                generic_params: Vec::new(),
                target: Some(functor_target),
                predicates: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        ),
        (
            applicative_id,
            HirTrait {
                id: applicative_id,
                name: "pkg::Applicative".to_string(),
                generic_params: Vec::new(),
                target: Some(applicative_target.clone()),
                predicates: vec![crate::types::Predicate::Trait {
                    subject: Type::Generic(applicative_target.id),
                    trait_id: functor_id,
                    args: Vec::new(),
                }],
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        ),
    ]);
    let caller_generic = GenericParamId {
        owner: caller_id,
        index: 0,
    };
    let current_bounds = HashMap::from([(
        caller_generic,
        vec![TraitBound {
            trait_id: applicative_id,
            type_args: Vec::new(),
        }],
    )])
    .into();
    let impls = HashMap::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_bounds);

    assert!(service.trait_bound_satisfied(
        &Type::Generic(caller_generic),
        &TraitBound {
            trait_id: functor_id,
            type_args: Vec::new(),
        },
    ));
}

#[test]
fn constructor_trait_selection_preserves_result_section_substitution() {
    let trait_id = def_id(700);
    let member_id = def_id(701);
    let impl_id = def_id(702);
    let method_id = def_id(703);
    let result_id = def_id(704);
    let io_error_id = def_id(705);
    let impl_error = GenericParamDecl::type_param(
        GenericParamId {
            owner: impl_id,
            index: 0,
        },
        "E",
    );
    let unary = crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    );
    let static_method = |id, name: &str| HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HirGenericBounds::new(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let trait_def = HirTrait {
        id: trait_id,
        name: "Applicative".to_string(),
        generic_params: Vec::new(),
        target: Some(GenericParamDecl::new(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "F",
            unary.clone(),
        )),
        predicates: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("pure".to_string(), static_method(member_id, "pure"))]),
        signatures: HashMap::new(),
    };
    let bound = || Type::BoundVar {
        depth: 0,
        index: 0,
        kind: crate::type_services::kind::Kind::Type,
    };
    let section = |error| Type::Lambda {
        params: vec![crate::type_services::kind::Kind::Type],
        body: Box::new(Type::Enum {
            id: result_id,
            args: vec![bound(), error],
        }),
    };
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("ResultSection".to_string()),
        type_name: "ResultSection".to_string(),
        type_generics: vec![impl_error.clone()],
        receiver_pattern: HirImplReceiverPattern::Constructor(section(Type::Generic(
            impl_error.id,
        ))),
        trait_name: Some("Applicative".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HirGenericBounds::new(),
        methods: HashMap::from([("pure".to_string(), static_method(method_id, "pure"))]),
    };
    let traits = HashMap::from([(trait_id, trait_def)]);
    let impls = HashMap::from([(impl_id, imp)]);
    let bounds = HirGenericBounds::new();
    let effective = effective_trait_methods_for(&traits, &impls);
    let service = SelectionService::new(&traits, &impls, None, None, &bounds)
        .with_effective_trait_methods(&effective);
    let io_error = Type::Struct {
        id: io_error_id,
        args: Vec::new(),
    };

    let selected = service
        .select_constructor_trait_member(&section(io_error.clone()), trait_id, &[], member_id)
        .expect("Result section should select its Applicative impl");

    assert_eq!(selected.target.impl_id(), Some(impl_id));
    assert_eq!(selected.target.trait_id(), Some(trait_id));
    assert_eq!(
        selected
            .target
            .owner_substitution
            .iter()
            .find(|binding| binding.param == impl_error.id)
            .map(|binding| &binding.ty),
        Some(&io_error),
    );

    let inferred = service
        .infer_constructor_target_for_applied_type(
            &Type::Enum {
                id: result_id,
                args: vec![Type::I64, io_error.clone()],
            },
            trait_id,
            &[],
        )
        .expect("constructor inference should not be ambiguous")
        .expect("Result application should infer its constructor section");
    assert_eq!(inferred, section(io_error));
}

#[test]
fn constructor_trait_selection_reports_upstream_ambiguity() {
    let trait_id = def_id(710);
    let member_id = def_id(711);
    let option_id = def_id(712);
    let unary = crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    );
    let function = |id| HirFunction {
        id,
        name: "pure".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HirGenericBounds::new(),
        params: Vec::new(),
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let trait_def = HirTrait {
        id: trait_id,
        name: "Applicative".to_string(),
        generic_params: Vec::new(),
        target: Some(GenericParamDecl::new(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "F",
            unary,
        )),
        predicates: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("pure".to_string(), function(member_id))]),
        signatures: HashMap::new(),
    };
    let target = Type::Constructor {
        id: option_id,
        flavor: crate::types::NominalTypeKind::Enum,
    };
    let impl_for = |impl_id: DefId, method_id: DefId| HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Option".to_string()),
        type_name: "Option".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Constructor(target.clone()),
        trait_name: Some("Applicative".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HirGenericBounds::new(),
        methods: HashMap::from([("pure".to_string(), function(method_id))]),
    };
    let traits = HashMap::from([(trait_id, trait_def)]);
    let impls = HashMap::from([
        (def_id(713), impl_for(def_id(713), def_id(714))),
        (def_id(715), impl_for(def_id(715), def_id(716))),
    ]);
    let bounds = HirGenericBounds::new();
    let effective = effective_trait_methods_for(&traits, &impls);
    let service = SelectionService::new(&traits, &impls, None, None, &bounds)
        .with_effective_trait_methods(&effective);

    let error = service
        .select_constructor_trait_member(&target, trait_id, &[], member_id)
        .expect_err("overlapping constructor impls must remain defensively ambiguous");

    assert!(error
        .message()
        .contains("upstream coherence invariant violation"));
}
