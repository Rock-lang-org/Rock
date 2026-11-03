use std::collections::HashMap;

use crate::hir::{
    HirAssociatedTypeDecl, HirAssociatedTypeDef, HirBlock, HirExpr, HirExprKind, HirFunction,
    HirFunctionSig, HirImpl, HirImplOwner, HirParam, HirTrait, HirVarRef, HirVarTarget,
};
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::lower::Lowerer;
use crate::selection::ReceiverAdjustment;
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, Type};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

#[test]
fn lowerer_matching_wrappers_match_selection_helpers() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(80));
    let generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Vec".to_string()),
        type_name: "Vec".to_string(),
        type_generics: vec![GenericParamDecl::type_param(generic, "T")],
        receiver_pattern: vec![Type::Generic(generic)].into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::new(),
    };
    let recv_ty = Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(81)),
        args: vec![Type::I64],
    };
    let mut lowerer_subst = HashMap::new();
    let mut selection_subst = HashMap::new();

    Lowerer::seed_receiver_substitution_from_impl(&imp, &recv_ty, &mut lowerer_subst);
    crate::selection::seed_receiver_substitution_from_impl(&imp, &recv_ty, &mut selection_subst);

    assert_eq!(lowerer_subst, selection_subst);
    assert!(crate::selection::type_pattern_matches(
        &Type::Generic(generic),
        &Type::I64,
        &mut HashMap::new()
    ));
}

#[test]
fn generated_operator_deref_persists_exact_selected_authority() {
    let owner_id = def_id(90);
    let trait_id = def_id(91);
    let member_id = def_id(92);
    let impl_id = def_id(93);
    let method_id = def_id(94);
    let receiver_ty = Type::Struct {
        id: owner_id,
        args: Vec::new(),
    };
    let return_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .resolver
        .item_names_by_id
        .insert(owner_id, "Box".to_string());
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "PointerLike".to_string());
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "PointerLike".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "*".to_string(),
            HirFunctionSig {
                id: member_id,
                name: "*".to_string(),
                generic_params: Vec::new(),
                params: vec![receiver_ty.clone()],
                ret: return_ty.clone(),
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(crate::types::ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(receiver_ty.clone()),
            trait_name: Some("PointerLike".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([(
                "*".to_string(),
                HirFunction {
                    id: method_id,
                    name: "*".to_string(),
                    generic_params: Vec::new(),
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: receiver_ty.clone(),
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: return_ty.clone(),
                    body: HirBlock {
                        stmts: Vec::new(),
                        ty: return_ty,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(crate::types::ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();
    lowerer
        .imported_effective_trait_methods
        .insert((impl_id, member_id), method_id);
    let expr = HirExpr {
        ty: receiver_ty,
        kind: HirExprKind::Var("box".to_string()),
        span: crate::lexer::Span::test(),
    };
    let candidates = lowerer
        .selection_service()
        .select_concrete_method_candidates(
            &[crate::selection::ReceiverCandidate {
                expr: expr.clone(),
                adjustment: crate::selection::ReceiverAdjustment::None,
                can_autoref_mut: false,
            }],
            "*",
            Type::clone,
        );
    assert_eq!(candidates.len(), 1, "expected one operator candidate");

    let deref = lowerer
        .apply_trait_deref(expr)
        .expect("operator protocol should select the exact impl");
    let HirExprKind::Deref(method_call) = deref.kind else {
        panic!("expected generated dereference");
    };
    let HirExprKind::MethodCall(_, method_name, _, _, Some(target)) = method_call.kind else {
        panic!("expected authority-bearing protocol method call");
    };
    assert_eq!(method_name, "*");
    assert_eq!(target.impl_id(), Some(impl_id));
    assert_eq!(target.trait_id(), Some(trait_id));
    assert_eq!(target.method_id(), Some(method_id));
}

#[test]
fn generic_nominal_pattern_matching_requires_same_def_id() {
    let pattern = Type::Struct {
        id: def_id(1),
        args: vec![Type::I64],
    };
    let actual = Type::Struct {
        id: def_id(2),
        args: vec![Type::I64],
    };

    assert!(!Lowerer::type_pattern_matches_after_subst(
        &pattern, &actual
    ));
}

#[test]
fn generic_substitution_does_not_cross_nominal_def_ids() {
    let expected = Type::Struct {
        id: def_id(1),
        args: vec![Type::Generic(GenericParamId {
            owner: def_id(0),
            index: 0,
        })],
    };
    let actual = Type::Struct {
        id: def_id(2),
        args: vec![Type::I64],
    };
    let mut subst = HashMap::new();

    Lowerer::infer_generic_subst_from_types(&expected, &actual, &mut subst);

    assert!(subst.is_empty());
}

#[test]
fn receiver_adjustment_coerces_resolved_local_array_to_slice_ref() {
    let mut lowerer = Lowerer::new_for_test();
    let array_ty = Type::Array(Box::new(Type::I64), 3);
    let expr = HirExpr {
        ty: array_ty,
        kind: HirExprKind::ResolvedVar(HirVarRef {
            name: "arr".to_string(),
            target: HirVarTarget::Local(crate::ids::HirLocalId(0)),
        }),
        span: crate::lexer::Span::test(),
    };

    let candidates = lowerer.receiver_adjustment_candidates(expr);

    assert!(candidates.iter().any(|candidate| matches!(
        &candidate.expr.ty,
        Type::Reference { mutable: false, inner }
            if matches!(inner.as_ref(), Type::Slice(elem) if elem.as_ref() == &Type::I64)
    )));
}

#[test]
fn receiver_adjustment_candidates_include_mutable_array_slice() {
    let owner_id = def_id(90);
    let trait_id = def_id(91);
    let member_id = def_id(92);
    let impl_id = def_id(93);
    let method_id = def_id(94);
    let mut lowerer = Lowerer::new_for_test();
    let array_ty = Type::Array(Box::new(Type::I64), 3);
    let return_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };
    lowerer
        .resolver
        .item_names_by_id
        .insert(owner_id, "[I64; 3]".to_string());
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "PointerLike".to_string());
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "PointerLike".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::from([(
            "*".to_string(),
            HirFunctionSig {
                id: member_id,
                name: "*".to_string(),
                generic_params: Vec::new(),
                params: vec![array_ty.clone()],
                ret: return_ty.clone(),
                generic_bounds: HashMap::new().into(),
                self_receiver: Some(crate::types::ReceiverMode::Move),
                is_unsafe: false,
            },
        )]),
    });
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("[I64; 3]".to_string()),
            type_name: "[I64; 3]".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(array_ty.clone()),
            trait_name: Some("PointerLike".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([(
                "*".to_string(),
                HirFunction {
                    id: method_id,
                    name: "*".to_string(),
                    generic_params: Vec::new(),
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: array_ty.clone(),
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: return_ty.clone(),
                    body: HirBlock {
                        stmts: Vec::new(),
                        ty: return_ty,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(crate::types::ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        })
        .unwrap();
    lowerer
        .imported_effective_trait_methods
        .insert((impl_id, member_id), method_id);
    lowerer.scope.define_local(
        "arr".to_string(),
        array_ty.clone(),
        true,
        crate::ids::HirLocalId(0),
    );
    let expr = HirExpr {
        ty: array_ty,
        kind: HirExprKind::ResolvedVar(HirVarRef {
            name: "arr".to_string(),
            target: HirVarTarget::Local(crate::ids::HirLocalId(0)),
        }),
        span: crate::lexer::Span::test(),
    };

    let candidates = lowerer.receiver_adjustment_candidates(expr);

    assert_eq!(candidates[0].adjustment, ReceiverAdjustment::None);
    assert_eq!(
        candidates[1].adjustment,
        ReceiverAdjustment::ArrayValueToSliceRef
    );
    assert!(matches!(
        (&candidates[0].expr.ty, &candidates[2].expr.ty, candidates[2].adjustment),
        (
            Type::Array(elem, 3),
            Type::Reference { mutable: true, inner },
            ReceiverAdjustment::ArrayValueToMutSliceRef
        ) if elem.as_ref() == &Type::I64
            && matches!(inner.as_ref(), Type::Slice(slice_elem) if slice_elem.as_ref() == &Type::I64)
    ));
}

#[test]
fn projection_resolution_rejects_assoc_type_owned_by_different_trait() {
    let show_id = def_id(30);
    let other_id = def_id(31);
    let impl_id = def_id(32);
    let box_id = def_id(33);
    let assoc_type_id = AssocTypeId(0);
    let base_ty = Type::Struct {
        id: box_id,
        args: Vec::new(),
    };

    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .resolver
        .item_names_by_id
        .insert(box_id, "Box".to_string());
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: show_id,
        name: "Show".to_string(),
        generic_params: Vec::new(),
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_type_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
    });
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: other_id,
        name: "Other".to_string(),
        generic_params: Vec::new(),
        associated_types: vec![HirAssociatedTypeDecl {
            id: assoc_type_id,
            name: "Output".to_string(),
            kind: crate::type_services::kind::Kind::Type,
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
    });
    lowerer
        .resolver
        .item_paths
        .insert("Show".to_string(), show_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(show_id, "Show".to_string());
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(base_ty.clone()),
            trait_name: Some("Show".to_string()),
            trait_id: Some(show_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();

    let projection = Type::Projection {
        ty: Box::new(base_ty),
        trait_id: show_id,
        assoc_type: AssociatedTypeKey {
            owner: other_id,
            assoc_type_id,
        },
        trait_args: Vec::new(),
    };

    assert!(matches!(
        lowerer.resolve_projection_type(&projection),
        Type::Projection { .. }
    ));
}

#[test]
fn nominal_method_lookup_in_context_preserves_canonical_owner_name() {
    let id = def_id(42);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .resolver
        .item_names_by_id
        .insert(id, "dep::module::Foo".to_string());

    assert_eq!(
        lowerer.get_type_name_for_method_lookup_in_context(&Type::Struct {
            id,
            args: Vec::new(),
        }),
        Some("dep::module::Foo".to_string())
    );
}

#[test]
fn trait_impl_selection_uses_typed_receiver_pattern() {
    let id = def_id(42);
    let trait_id = def_id(44);
    let mut lowerer = Lowerer::new_for_test();
    lowerer
        .resolver
        .item_names_by_id
        .insert(id, "dep::module::Foo".to_string());
    lowerer.items.insert_trait_def(HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Show".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    });
    lowerer
        .resolver
        .item_paths
        .insert("Show".to_string(), trait_id);
    lowerer
        .resolver
        .item_names_by_id
        .insert(trait_id, "Show".to_string());
    let impl_id = def_id(43);
    lowerer
        .items
        .insert_impl(HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("dep::module::Foo".to_string()),
            type_name: "Foo".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                id,
                args: Vec::new(),
            }),
            trait_name: Some("Show".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        })
        .unwrap();
    assert!(lowerer
        .selection_service()
        .select_trait_impl(
            &Type::Struct {
                id,
                args: Vec::new(),
            },
            trait_id,
            &[],
        )
        .is_ok());
}
