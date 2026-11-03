use std::collections::HashMap;

use crate::hir::{
    HirImplOwner, HirImplReceiverPattern, HirMethodCallTarget, HirParam, HirVarTarget,
};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::lexer::Span;
use crate::mono::hir_types::{HirBlock, HirExpr, HirExprKind, HirFunction, HirImpl, HirStmt};
use crate::types::ReceiverMode;
use crate::types::Type;
use crate::types::{GenericParamDecl, GenericParamId};

use crate::mono::{InstanceImplOwner, InstanceOrigin};

use super::Monomorphizer;

fn empty_body(ty: Type) -> HirBlock {
    HirBlock { stmts: vec![], ty }
}

fn enum_ty(type_name: &str, args: Vec<Type>) -> Type {
    let local = match type_name {
        "Option" | "std::option::Option" => 10,
        "Result" => 11,
        _ => 12,
    };
    Type::Enum {
        id: DefId::new(CrateId(0), LocalDefId(local)),
        args,
    }
}

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn type_generic(owner: DefId, name: &str) -> Vec<GenericParamDecl> {
    vec![GenericParamDecl::type_param(
        GenericParamId { owner, index: 0 },
        name,
    )]
}

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

fn option_map_method(impl_id: DefId, method_id: DefId) -> HirFunction {
    let impl_generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_generic = GenericParamId {
        owner: method_id,
        index: 0,
    };
    HirFunction {
        id: method_id,
        name: "map".to_string(),
        generic_params: vec![
            GenericParamDecl::type_param(impl_generic, "T"),
            GenericParamDecl::type_param(method_generic, "U"),
        ],
        generic_bounds: HashMap::new().into(),
        params: vec![
            HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: enum_ty("Option", vec![Type::Generic(impl_generic)]),
                mutable: false,
                is_ref: false,
            },
            HirParam {
                name: "f".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::function(
                    vec![Type::Generic(impl_generic)],
                    Type::Generic(method_generic),
                ),
                mutable: false,
                is_ref: false,
            },
        ],
        ret_type: enum_ty("Option", vec![Type::Generic(method_generic)]),
        body: empty_body(Type::Unit),
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    }
}

fn map_method_for_type(type_name: &str, impl_id: DefId, method_id: DefId) -> HirFunction {
    let mut method = option_map_method(impl_id, method_id);
    method.params[0].ty = enum_ty(
        type_name,
        vec![Type::Generic(GenericParamId {
            owner: impl_id,
            index: 0,
        })],
    );
    method.ret_type = enum_ty(
        type_name,
        vec![Type::Generic(GenericParamId {
            owner: method_id,
            index: 0,
        })],
    );
    method
}

fn map_expr_for_type(type_name: &str) -> (HirExpr, HirExpr, HirExpr) {
    let recv = HirExpr {
        kind: HirExprKind::Var("value".to_string()),
        ty: enum_ty(type_name, vec![Type::I64]),
        span: Span::test(),
    };
    let func_arg = HirExpr {
        kind: HirExprKind::Var("inc".to_string()),
        ty: Type::function(vec![Type::I64], Type::I64),
        span: Span::test(),
    };
    let expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "map".to_string(),
            vec![func_arg.clone()],
            Some(ReceiverMode::Move),
            HirMethodCallTarget::trait_method(
                def_id(999),
                def_id(998),
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: enum_ty(type_name, vec![Type::I64]),
        span: Span::test(),
    };

    (recv, func_arg, expr)
}

fn generic_map_impl(type_name: &str, impl_id: DefId, method_id: DefId) -> HirImpl {
    HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named(type_name.to_string()),
        type_name: type_name.to_string(),
        type_generics: type_generic(impl_id, "T"),
        receiver_pattern: vec![Type::Generic(GenericParamId {
            owner: impl_id,
            index: 0,
        })]
        .into(),
        trait_name: None,
        trait_id: None,
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "map".to_string(),
            map_method_for_type(type_name, impl_id, method_id),
        )]),
    }
}

fn seed_generic_enum_receiver_pattern(mono: &mut Monomorphizer, impl_id: DefId, type_name: &str) {
    mono.set_impl_receiver_pattern_for_test(
        impl_id,
        crate::hir::HirImplReceiverPattern::Exact(enum_ty(
            type_name,
            vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })],
        )),
    );
}

#[test]
fn monomorphize_standalone_method_call_without_selected_target_does_not_specialize_by_name() {
    let mut mono = Monomorphizer::new();
    let impl_id = def_id(90);
    let method_id = def_id(91);
    mono.generic_impls
        .insert(impl_id, generic_map_impl("Option", impl_id, method_id));
    let (recv, func_arg, mut expr) = map_expr_for_type("Option");

    let error = mono
        .monomorphize_standalone_method_call("map", &[recv, func_arg], &mut expr)
        .expect_err("a targetless standalone method must be rejected");
    assert!(matches!(
        error.kind,
        crate::mono::MonoErrorKind::InvalidBindings { .. }
    ));

    assert!(
        matches!(expr.kind, HirExprKind::MethodCall(..)),
        "targetless standalone methods must not specialize by type and method name"
    );
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn monomorphize_trait_method_call_without_selected_target_does_not_specialize_by_name() {
    let mut mono = Monomorphizer::new();
    let impl_id = def_id(92);
    let trait_id = def_id(93);
    let method_id = def_id(94);
    let method = option_println_method(impl_id, method_id);
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
    let recv = HirExpr {
        kind: HirExprKind::Var("some".to_string()),
        ty: enum_ty("Option", vec![Type::I64]),
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

    assert!(
        matches!(expr.kind, HirExprKind::MethodCall(..)),
        "targetless trait methods must not specialize by receiver and method name"
    );
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn monomorphize_trait_method_call_rejects_target_without_trait_identity() {
    let mut mono = Monomorphizer::new();
    let impl_id = def_id(96);
    let method_id = def_id(97);
    let method = option_println_method(impl_id, method_id);
    mono.trait_impls.insert(
        def_id(98),
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
            trait_id: Some(def_id(98)),
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
            HirMethodCallTarget::impl_method(impl_id, method_id, None),
        ),
        ty: Type::I32,
        span: Span::test(),
    };

    let error = mono
        .monomorphize_trait_method_call("println", &[recv], &mut expr)
        .expect_err("an impl target without trait identity must be rejected");

    assert!(matches!(
        error.kind,
        crate::mono::MonoErrorKind::InvalidBindings { .. }
    ));

    assert!(matches!(expr.kind, HirExprKind::MethodCall(..)));
    assert!(mono.instances.records().next().is_none());
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

fn option_println_method(impl_id: DefId, method_id: DefId) -> HirFunction {
    let impl_generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    HirFunction {
        id: method_id,
        name: "println".to_string(),
        generic_params: vec![GenericParamDecl::type_param(impl_generic, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: enum_ty("Option", vec![Type::Generic(impl_generic)]),
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

fn byte_slice_println_method() -> HirFunction {
    HirFunction {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        name: "println".to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(Type::U8))),
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

fn qualified_option_receiver() -> HirExpr {
    HirExpr {
        kind: HirExprKind::Var("some".to_string()),
        ty: enum_ty("std::option::Option", vec![Type::I64]),
        span: Span::test(),
    }
}

fn seed_owner(mono: &mut Monomorphizer, name: &str) {
    let local = match name {
        "Option" | "std::option::Option" => 10,
        "Result" => 11,
        _ => 0,
    };
    let def_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(local));
    mono.resolver.item_paths.insert(name.to_string(), def_id);
    mono.resolver
        .item_names_by_id
        .insert(def_id, name.to_string());
}

fn selected_impl_target(impl_id: DefId, method_id: DefId, owner_ty: Type) -> HirMethodCallTarget {
    let mut target = HirMethodCallTarget::impl_method(impl_id, method_id, None);
    target.owner_substitution = vec![crate::hir::HirTypeBinding {
        param: GenericParamId {
            owner: impl_id,
            index: 0,
        },
        ty: owner_ty,
    }];
    target
}

fn selected_map_target(
    impl_id: DefId,
    method_id: DefId,
    owner_ty: Type,
    method_ty: Type,
) -> HirMethodCallTarget {
    let mut target = selected_impl_target(impl_id, method_id, owner_ty);
    target.method_substitution = vec![crate::hir::HirTypeBinding {
        param: GenericParamId {
            owner: method_id,
            index: 0,
        },
        ty: method_ty,
    }];
    target
}

#[test]
fn monomorphize_same_named_methods_on_different_impls_use_distinct_method_origins() {
    let mut mono = Monomorphizer::new();
    let option_impl_id = DefId::new(CrateId(0), LocalDefId(10));
    let result_impl_id = DefId::new(CrateId(0), LocalDefId(11));
    let option_method_id = DefId::new(CrateId(0), LocalDefId(12));
    let result_method_id = DefId::new(CrateId(0), LocalDefId(13));
    mono.generic_impls.insert(
        option_impl_id,
        generic_map_impl("Option", option_impl_id, option_method_id),
    );
    mono.generic_impls.insert(
        result_impl_id,
        generic_map_impl("Result", result_impl_id, result_method_id),
    );
    seed_generic_enum_receiver_pattern(&mut mono, option_impl_id, "Option");
    seed_generic_enum_receiver_pattern(&mut mono, result_impl_id, "Result");

    let (option_recv, option_func_arg, mut option_expr) = map_expr_for_type("Option");
    if let HirExprKind::MethodCall(_, _, _, _, target) = &mut option_expr.kind {
        *target = selected_map_target(option_impl_id, option_method_id, Type::I64, Type::I64);
    }
    let _ = mono.monomorphize_standalone_method_call(
        "map",
        &[option_recv, option_func_arg],
        &mut option_expr,
    );
    let (result_recv, result_func_arg, mut result_expr) = map_expr_for_type("Result");
    if let HirExprKind::MethodCall(_, _, _, _, target) = &mut result_expr.kind {
        *target = selected_map_target(result_impl_id, result_method_id, Type::I64, Type::I64);
    }
    let _ = mono.monomorphize_standalone_method_call(
        "map",
        &[result_recv, result_func_arg],
        &mut result_expr,
    );

    let origins = mono
        .instances
        .records()
        .map(|record| record.origin.clone())
        .collect::<Vec<_>>();
    assert!(origins.contains(&InstanceOrigin::ImplMethod {
        owner: InstanceImplOwner::Named(option_impl_id),
        method: option_method_id,
    }));
    assert!(origins.contains(&InstanceOrigin::ImplMethod {
        owner: InstanceImplOwner::Named(result_impl_id),
        method: result_method_id,
    }));
    assert_eq!(origins.len(), 2);
}

#[test]
fn monomorphize_same_owner_distinct_methods_use_distinct_method_origins() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(20));
    let map_method_id = DefId::new(CrateId(0), LocalDefId(21));
    let fold_method_id = DefId::new(CrateId(0), LocalDefId(22));
    let mut fold_method = map_method_for_type("Option", impl_id, fold_method_id);
    fold_method.name = "fold".to_string();
    mono.generic_impls.insert(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Option".to_string()),
            type_name: "Option".to_string(),
            type_generics: type_generic(impl_id, "T"),
            receiver_pattern: vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })]
            .into(),
            trait_name: None,
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([
                (
                    "map".to_string(),
                    map_method_for_type("Option", impl_id, map_method_id),
                ),
                ("fold".to_string(), fold_method),
            ]),
        },
    );
    seed_generic_enum_receiver_pattern(&mut mono, impl_id, "Option");

    let (map_recv, map_func_arg, mut map_expr) = map_expr_for_type("Option");
    if let HirExprKind::MethodCall(_, _, _, _, target) = &mut map_expr.kind {
        *target = selected_map_target(impl_id, map_method_id, Type::I64, Type::I64);
    }
    let _ =
        mono.monomorphize_standalone_method_call("map", &[map_recv, map_func_arg], &mut map_expr);
    let (fold_recv, fold_func_arg, mut fold_expr) = map_expr_for_type("Option");
    if let HirExprKind::MethodCall(_, _, _, _, target) = &mut fold_expr.kind {
        *target = selected_map_target(impl_id, fold_method_id, Type::I64, Type::I64);
    }
    let _ = mono.monomorphize_standalone_method_call(
        "fold",
        &[fold_recv, fold_func_arg],
        &mut fold_expr,
    );

    let origins = mono
        .instances
        .records()
        .map(|record| record.origin.clone())
        .collect::<Vec<_>>();
    assert!(origins.contains(&InstanceOrigin::ImplMethod {
        owner: InstanceImplOwner::Named(impl_id),
        method: map_method_id,
    }));
    assert!(origins.contains(&InstanceOrigin::ImplMethod {
        owner: InstanceImplOwner::Named(impl_id),
        method: fold_method_id,
    }));
    assert_eq!(origins.len(), 2);
}

#[test]
fn standalone_method_call_uses_canonical_receiver_pattern_for_qualified_receiver() {
    let mut mono = Monomorphizer::new();
    seed_owner(&mut mono, "Option");
    let impl_id = DefId::new(CrateId(0), LocalDefId(60));
    let method_id = DefId::new(CrateId(0), LocalDefId(61));
    let method = option_map_method(impl_id, method_id);
    mono.generic_impls.insert(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Option".to_string()),
            type_name: "Option".to_string(),
            type_generics: type_generic(impl_id, "T"),
            receiver_pattern: vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })]
            .into(),
            trait_name: None,
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("map".to_string(), method)]),
        },
    );
    seed_generic_enum_receiver_pattern(&mut mono, impl_id, "Option");

    let recv = qualified_option_receiver();
    let func_arg = HirExpr {
        kind: HirExprKind::Var("inc".to_string()),
        ty: Type::function(vec![Type::I64], Type::I64),
        span: Span::test(),
    };
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "map".to_string(),
            vec![func_arg.clone()],
            Some(ReceiverMode::Move),
            selected_map_target(impl_id, method_id, Type::I64, Type::I64),
        ),
        ty: enum_ty("std::option::Option", vec![Type::I64]),
        span: Span::test(),
    };

    let _ = mono.monomorphize_standalone_method_call("map", &[recv, func_arg], &mut expr);

    assert!(matches!(expr.kind, HirExprKind::Call(_, _, _)));
}

#[test]
fn trait_method_call_uses_canonical_receiver_pattern_for_qualified_receiver() {
    let mut mono = Monomorphizer::new();
    seed_owner(&mut mono, "Option");
    let impl_id = DefId::new(CrateId(0), LocalDefId(62));
    let trait_id = DefId::new(CrateId(0), LocalDefId(63));
    let method_id = DefId::new(CrateId(0), LocalDefId(64));
    let method = option_println_method(impl_id, method_id);
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
    seed_generic_enum_receiver_pattern(&mut mono, impl_id, "Option");

    let recv = qualified_option_receiver();
    let mut selected_target = selected_impl_target(impl_id, method_id, Type::I64);
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

    assert!(matches!(expr.kind, HirExprKind::Call(_, _, _)));
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
fn test_collect_impls_keeps_object_backed_concrete_trait_impls_available_for_dispatch() {
    let mut mono = Monomorphizer::new();
    let concrete_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::BuiltinSlice,
        type_name: "&[U8]".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![Type::U8].into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(def_id(500)),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("println".to_string(), byte_slice_println_method())]),
    };
    mono.collect_impls(&[concrete_impl]);

    assert_eq!(mono.trait_impls[&def_id(500)].len(), 1);
}

#[test]
fn collect_impls_keeps_same_named_traits_separate_by_def_id() {
    let mut mono = Monomorphizer::new();
    let first_trait_id = def_id(610);
    let second_trait_id = def_id(611);
    let first_method = option_println_method(def_id(614), def_id(612));
    let second_method = option_println_method(def_id(615), def_id(613));

    mono.collect_impls(&[
        HirImpl {
            id: def_id(614),
            owner: HirImplOwner::Named("Option".to_string()),
            type_name: "Option".to_string(),
            type_generics: type_generic(def_id(614), "T"),
            receiver_pattern: vec![Type::Generic(GenericParamId {
                owner: def_id(614),
                index: 0,
            })]
            .into(),
            trait_name: Some("Show".to_string()),
            trait_id: Some(first_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("println".to_string(), first_method)]),
        },
        HirImpl {
            id: def_id(615),
            owner: HirImplOwner::Named("Option".to_string()),
            type_name: "Option".to_string(),
            type_generics: type_generic(def_id(615), "T"),
            receiver_pattern: vec![Type::Generic(GenericParamId {
                owner: def_id(615),
                index: 0,
            })]
            .into(),
            trait_name: Some("Show".to_string()),
            trait_id: Some(second_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("println".to_string(), second_method)]),
        },
    ]);

    assert_eq!(mono.trait_impls.len(), 2);
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

#[test]
fn specialize_impl_method_preserves_concrete_nominal_self_type_without_impl_args() {
    let mut mono = Monomorphizer::new();
    let foo_id = def_id(77);
    let method = HirFunction {
        id: def_id(78),
        name: "id".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(0),
                index: 0,
            },
            "U",
        )],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Struct {
                id: foo_id,
                args: Vec::new(),
            },
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::Var("self".to_string()),
                ty: Type::Generic(crate::types::GenericParamId {
                    owner: crate::ids::DefId::new(
                        crate::ids::CrateId(0),
                        crate::ids::LocalDefId(0),
                    ),
                    index: 0,
                }),
                span: Span::test(),
            })],
            ty: Type::I64,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };

    let i64_id = mono.intern_type(&Type::I64);
    let specialized = mono.specialize_impl_method(
        &method,
        &[i64_id],
        &[],
        "Foo_id_I64",
        DefId::new(CrateId(0), LocalDefId(0)),
        None,
    );

    let HirStmt::Expr(HirExpr { ty, .. }) = &specialized.body.stmts[0] else {
        panic!("expected self expression");
    };
    assert_eq!(
        ty,
        &Type::Struct {
            id: foo_id,
            args: Vec::new(),
        }
    );
}

#[test]
fn specialize_impl_method_preserves_nested_nominal_receiver_shape() {
    let mut mono = Monomorphizer::new();
    let impl_id = def_id(79);
    let option_id = def_id(80);
    let generic = Type::Generic(crate::types::GenericParamId {
        owner: impl_id,
        index: 0,
    });
    let receiver = Type::Enum {
        id: option_id,
        args: vec![Type::Enum {
            id: option_id,
            args: vec![generic],
        }],
    };
    let method = HirFunction {
        id: def_id(81),
        name: "flatten".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: receiver.clone(),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::Var("self".to_string()),
                ty: receiver,
                span: Span::test(),
            })],
            ty: Type::I64,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };
    let i64_id = mono.intern_type(&Type::I64);

    let specialized = mono.specialize_impl_method(
        &method,
        &[i64_id],
        &[GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        "Option_Option_I64_flatten",
        impl_id,
        None,
    );
    let expected = Type::Enum {
        id: option_id,
        args: vec![Type::Enum {
            id: option_id,
            args: vec![Type::I64],
        }],
    };
    let HirStmt::Expr(HirExpr { ty, .. }) = &specialized.body.stmts[0] else {
        panic!("expected self expression");
    };

    assert_eq!(specialized.params[0].ty, expected);
    assert_eq!(ty, &expected);
}
