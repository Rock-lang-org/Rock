use super::super::hir_types::HirImpl;
use super::*;
use crate::hir::{HirCallTarget, HirImplOwner, HirMethodCallTarget, HirParam};
use crate::ids::{CrateId, DefId, HirLocalId, LocalDefId};
use crate::mono::{
    InstanceImplOwner, InstanceKey, InstanceOrigin, InstanceRecord, InstanceSymbols,
};
use crate::types::ReceiverMode;
use crate::types::{GenericParamDecl, GenericParamId, Type};
use std::collections::HashMap;

fn empty_function(receiver: Option<ReceiverMode>) -> HirFunction {
    HirFunction {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        name: "f".to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![],
        ret_type: crate::types::Type::Unit,
        body: HirBlock {
            stmts: vec![],
            ty: crate::types::Type::Unit,
        },
        is_curried: false,
        is_method: true,
        self_receiver: receiver,
        is_unsafe: false,
    }
}

fn function_with_id(id: DefId, receiver: Option<ReceiverMode>) -> HirFunction {
    let mut function = empty_function(receiver);
    function.id = id;
    function
}

#[test]
fn test_lookup_self_receiver_uses_targeted_trait_impl() {
    let mut mono = Monomorphizer::new();
    let trait_id = DefId::new(CrateId(0), LocalDefId(11));
    let method_id = DefId::new(CrateId(0), LocalDefId(12));

    let first_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::Named("Foo".to_string()),
        trait_name: Some("TraitA".to_string()),
        trait_id: Some(DefId::new(CrateId(0), LocalDefId(10))),
        type_name: "Foo".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("other".to_string(), empty_function(None))]),
    };
    let second_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::Named("Foo".to_string()),
        trait_name: Some("TraitB".to_string()),
        trait_id: Some(trait_id),
        type_name: "Foo".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "target".to_string(),
            function_with_id(method_id, Some(ReceiverMode::Mut)),
        )]),
    };

    mono.trait_impls
        .insert(DefId::new(CrateId(0), LocalDefId(10)), vec![first_impl]);
    mono.trait_impls.insert(trait_id, vec![second_impl]);

    assert_eq!(
        mono.lookup_self_receiver(Some(&HirMethodCallTarget::trait_method(
            trait_id,
            method_id,
            Vec::new(),
            crate::hir::HirTraitDispatchKind::TraitBound,
        ))),
        Some(ReceiverMode::Mut)
    );
}

#[test]
fn test_lookup_self_receiver_does_not_pick_first_same_named_trait_method() {
    let mut mono = Monomorphizer::new();
    let first_trait_id = DefId::new(CrateId(0), LocalDefId(10));
    let second_trait_id = DefId::new(CrateId(0), LocalDefId(11));
    let first_method_id = DefId::new(CrateId(0), LocalDefId(30));
    let second_method_id = DefId::new(CrateId(0), LocalDefId(31));

    let first_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(20)),
        owner: HirImplOwner::Named("Foo".to_string()),
        trait_name: Some("TraitA".to_string()),
        trait_id: Some(first_trait_id),
        type_name: "Foo".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "target".to_string(),
            function_with_id(first_method_id, Some(ReceiverMode::Move)),
        )]),
    };
    let second_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(21)),
        owner: HirImplOwner::Named("Foo".to_string()),
        trait_name: Some("TraitB".to_string()),
        trait_id: Some(second_trait_id),
        type_name: "Foo".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "target".to_string(),
            function_with_id(second_method_id, Some(ReceiverMode::Mut)),
        )]),
    };

    mono.trait_impls.insert(first_trait_id, vec![first_impl]);
    mono.trait_impls.insert(second_trait_id, vec![second_impl]);

    assert_eq!(
        mono.lookup_self_receiver(Some(&HirMethodCallTarget::trait_method(
            second_trait_id,
            second_method_id,
            Vec::new(),
            crate::hir::HirTraitDispatchKind::TraitBound,
        ))),
        Some(ReceiverMode::Mut)
    );
}

fn generic_self_method(_type_name: &str, impl_id: DefId, method_id: DefId) -> HirFunction {
    let generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    HirFunction {
        id: method_id,
        name: "map".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(700)),
                args: vec![Type::Generic(generic)],
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
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    }
}

fn generic_impl_for_process_test(
    type_name: &str,
    impl_id: DefId,
    trait_id: Option<DefId>,
    method_id: DefId,
) -> HirImpl {
    HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named(type_name.to_string()),
        type_name: type_name.to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: vec![Type::Generic(GenericParamId {
            owner: impl_id,
            index: 0,
        })]
        .into(),
        trait_name: trait_id.map(|_| "MapTrait".to_string()),
        trait_id,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "map".to_string(),
            generic_self_method(type_name, impl_id, method_id),
        )]),
    }
}

fn selected_process_method_target(impl_id: DefId, method_id: DefId) -> HirMethodCallTarget {
    let mut target = HirMethodCallTarget::impl_method(impl_id, method_id, None);
    target.owner_substitution = vec![crate::hir::HirTypeBinding {
        param: GenericParamId {
            owner: impl_id,
            index: 0,
        },
        ty: Type::I64,
    }];
    target
}

fn seed_box_receiver_pattern(mono: &mut Monomorphizer, impl_id: DefId) {
    mono.set_impl_receiver_pattern_for_test(
        impl_id,
        crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(700)),
            args: vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })],
        }),
    );
}

fn generic_static_impl_for_process_test(
    type_name: &str,
    impl_id: DefId,
    method_id: DefId,
) -> HirImpl {
    let generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named(type_name.to_string()),
        type_name: type_name.to_string(),
        type_generics: vec![GenericParamDecl::type_param(generic, "T")],
        receiver_pattern: vec![Type::Generic(generic)].into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "make".to_string(),
            HirFunction {
                id: method_id,
                name: "make".to_string(),
                generic_params: Vec::new(),
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "value".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(generic),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: Type::Generic(generic),
                body: HirBlock {
                    stmts: Vec::new(),
                    ty: Type::Generic(generic),
                },
                is_curried: false,
                is_method: false,
                self_receiver: None,
                is_unsafe: false,
            },
        )]),
    }
}

fn generic_identity_for_process_test(id: DefId, name: &str) -> HirFunction {
    let generic_id = GenericParamId {
        owner: id,
        index: 0,
    };
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: vec![GenericParamDecl::type_param(generic_id, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(generic_id),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(generic_id),
        body: HirBlock {
            stmts: vec![HirStmt::Return(Some(HirExpr {
                kind: HirExprKind::Var("value".to_string()),
                ty: Type::Generic(generic_id),
                span: crate::lexer::Span::test(),
            }))],
            ty: Type::Generic(generic_id),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn callback_acceptor_for_process_test(id: DefId, name: &str, callback_ty: Type) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "callback".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: callback_ty,
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
    }
}

#[test]
fn process_targetless_generic_function_call_does_not_specialize_by_name() {
    let mut mono = Monomorphizer::new();
    let id = DefId::new(CrateId(0), LocalDefId(910));
    let generic = HirFunction {
        id,
        name: "identity".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: id,
                index: 0,
            },
            "T",
        )],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(GenericParamId {
                owner: id,
                index: 0,
            }),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(GenericParamId {
            owner: id,
            index: 0,
        }),
        body: HirBlock {
            stmts: vec![HirStmt::Return(Some(HirExpr {
                kind: HirExprKind::Var("value".to_string()),
                ty: Type::Generic(GenericParamId {
                    owner: id,
                    index: 0,
                }),
                span: crate::lexer::Span::test(),
            }))],
            ty: Type::Generic(GenericParamId {
                owner: id,
                index: 0,
            }),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    mono.register_generic_function("identity".to_string(), generic);
    mono.resolver.item_paths.insert("identity".to_string(), id);
    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::Var("identity".to_string()),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::IntLiteral(21),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }],
            None,
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("expression should remain a call");
    };
    assert!(matches!(&callee.kind, HirExprKind::Var(name) if name == "identity"));
    assert_eq!(target, &None);
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn process_targetless_generic_function_value_does_not_specialize_by_name() {
    let mut mono = Monomorphizer::new();
    let id = DefId::new(CrateId(0), LocalDefId(911));
    let generic = generic_identity_for_process_test(id, "identity");
    mono.register_generic_function("identity".to_string(), generic);
    mono.resolver.item_paths.insert("identity".to_string(), id);

    let mut expr = HirExpr {
        kind: HirExprKind::Var("identity".to_string()),
        ty: Type::function(vec![Type::I64], Type::I64),
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "identity"));
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn process_resolved_generic_function_value_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    let identity_id = DefId::new(CrateId(0), LocalDefId(970));
    let wrong_id = DefId::new(CrateId(0), LocalDefId(971));
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_generic_function("identity".to_string(), identity.clone());
    mono.register_generic_function("wrong_alias".to_string(), wrong.clone());
    mono.resolver
        .item_paths
        .insert("identity".to_string(), identity_id);
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), wrong_id);

    let mut expr = HirExpr {
        kind: HirExprKind::ResolvedVar(HirVarRef {
            name: "wrong_alias".to_string(),
            target: HirVarTarget::Function(identity_id),
        }),
        ty: Type::function(vec![Type::I64], Type::I64),
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::ResolvedVar(reference) = &expr.kind else {
        panic!("function value should remain a resolved var");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("resolved generic function value should target an instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
}

#[test]
fn process_resolved_generic_call_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    let identity_id = DefId::new(CrateId(0), LocalDefId(972));
    let wrong_id = DefId::new(CrateId(0), LocalDefId(973));
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_generic_function("identity".to_string(), identity.clone());
    mono.register_generic_function("wrong_alias".to_string(), wrong.clone());
    mono.resolver
        .item_paths
        .insert("identity".to_string(), identity_id);
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), wrong_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Function(identity_id),
                }),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::IntLiteral(42),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(identity_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("callee should be instance-backed");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("callee should target a monomorphized instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
}

#[test]
fn process_external_generic_function_value_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    mono.register_test_dependency_identity(CrateId(1));
    let identity_id = DefId::new(CrateId(1), LocalDefId(984));
    let wrong_id = DefId::new(CrateId(1), LocalDefId(985));
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_external_generic_function("dep::identity".to_string(), identity);
    mono.register_external_generic_function("wrong_alias".to_string(), wrong);
    mono.resolver
        .item_paths
        .insert("dep::identity".to_string(), identity_id);
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), wrong_id);

    let mut expr = HirExpr {
        kind: HirExprKind::ResolvedVar(HirVarRef {
            name: "wrong_alias".to_string(),
            target: HirVarTarget::Extern(identity_id),
        }),
        ty: Type::function(vec![Type::I64], Type::I64),
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::ResolvedVar(reference) = &expr.kind else {
        panic!("external function value should remain a resolved var");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("external generic function value should target an instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
}

#[test]
fn process_external_generic_call_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    mono.register_test_dependency_identity(CrateId(1));
    let identity_id = DefId::new(CrateId(1), LocalDefId(986));
    let wrong_id = DefId::new(CrateId(1), LocalDefId(987));
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_external_generic_function("dep::identity".to_string(), identity);
    mono.register_external_generic_function("wrong_alias".to_string(), wrong);
    mono.resolver
        .item_paths
        .insert("dep::identity".to_string(), identity_id);
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), wrong_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Extern(identity_id),
                }),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::IntLiteral(42),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(identity_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("callee should be instance-backed");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("callee should target a monomorphized instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
}

#[test]
fn process_object_backed_concrete_function_call_uses_registered_instance() {
    let mut mono = Monomorphizer::new();
    let answer_id = DefId::new(CrateId(1), LocalDefId(994));
    let answer = callback_acceptor_for_process_test(answer_id, "answer", Type::Unit);
    let key = InstanceKey::new(InstanceOrigin::Function(answer_id), Vec::new());
    let instance_id = mono.instances.intern(key.clone(), |id| InstanceRecord {
        id,
        origin: key.origin.clone(),
        substitution: key.substitution.clone(),
        symbols: InstanceSymbols::new("dep::answer", "dep::answer"),
        declared: Some(answer.clone()),
        provided_by_object: true,
        is_specialization: false,
    });
    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "answer".to_string(),
                    target: HirVarTarget::Function(answer_id),
                }),
                ty: Type::function(Vec::new(), Type::I64),
                span: crate::lexer::Span::test(),
            }),
            Vec::new(),
            Some(HirCallTarget::Function(answer_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("callee should remain resolved");
    };
    assert_eq!(reference.target, HirVarTarget::Instance(instance_id));
    assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
}

#[test]
fn process_object_backed_static_impl_method_call_uses_registered_instance() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(1), LocalDefId(995));
    let method_id = DefId::new(CrateId(1), LocalDefId(996));
    let method = callback_acceptor_for_process_test(method_id, "dealloc", Type::Unit);
    let key = InstanceKey::new(
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: method_id,
        },
        Vec::new(),
    );
    let instance_id = mono.instances.intern(key.clone(), |id| InstanceRecord {
        id,
        origin: key.origin.clone(),
        substitution: key.substitution.clone(),
        symbols: InstanceSymbols::new("__rock_Global_dealloc", "__rock_Global_dealloc"),
        declared: Some(method.clone()),
        provided_by_object: true,
        is_specialization: false,
    });
    mono.generic_impls.insert(
        impl_id,
        HirImpl {
            id: impl_id,
            owner: crate::hir::HirImplOwner::Named("Global".to_string()),
            type_name: "Global".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("dealloc".to_string(), method)]),
        },
    );
    mono.set_impl_receiver_pattern_for_test(
        impl_id,
        crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
    );

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "dealloc".to_string(),
                    target: HirVarTarget::Function(method_id),
                }),
                ty: Type::function(Vec::new(), Type::I64),
                span: crate::lexer::Span::test(),
            }),
            Vec::new(),
            Some(HirCallTarget::StaticMethod(
                crate::hir::HirStaticMethodTarget {
                    owner_ty: Type::Unit,
                    method: HirMethodCallTarget::impl_method(impl_id, method_id, None),
                },
            )),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("callee should remain resolved");
    };
    assert_eq!(reference.target, HirVarTarget::Instance(instance_id));
    assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
}

#[test]
fn generic_function_by_id_requires_def_id_name_metadata() {
    let mut mono = Monomorphizer::new();
    let id = DefId::new(CrateId(0), LocalDefId(977));
    let function = generic_identity_for_process_test(id, "identity");

    mono.generic_functions.insert(id, function);

    assert!(mono.generic_function_by_id(id).is_none());
}

#[test]
fn generic_function_payloads_with_matching_display_names_remain_distinct_by_def_id() {
    let mut mono = Monomorphizer::new();
    let first_id = DefId::new(CrateId(0), LocalDefId(978));
    let second_id = DefId::new(CrateId(1), LocalDefId(978));

    mono.register_generic_function(
        "shared".to_string(),
        generic_identity_for_process_test(first_id, "stale-payload-name"),
    );
    mono.register_generic_function(
        "shared".to_string(),
        generic_identity_for_process_test(second_id, "stale-payload-name"),
    );
    assert_eq!(mono.generic_functions.len(), 2);
    assert_eq!(
        mono.generic_function_by_id(second_id)
            .expect("selected DefId should retain its payload")
            .1
            .id,
        second_id
    );
}

#[test]
fn local_type_tracking_uses_hir_local_id_not_source_name() {
    let mut mono = Monomorphizer::new();
    let mut function = empty_function(None);
    function.body = HirBlock {
        stmts: vec![
            HirStmt::Let {
                name: "value".to_string(),
                local_id: HirLocalId(0),
                ty: Type::I64,
                value: HirExpr {
                    kind: HirExprKind::IntLiteral(1),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                },
                mutable: false,
            },
            HirStmt::Let {
                name: "value".to_string(),
                local_id: HirLocalId(1),
                ty: Type::Bool,
                value: HirExpr {
                    kind: HirExprKind::BoolLiteral(true),
                    ty: Type::Bool,
                    span: crate::lexer::Span::test(),
                },
                mutable: false,
            },
            HirStmt::Expr(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "value".to_string(),
                    target: HirVarTarget::Local(HirLocalId(0)),
                }),
                ty: Type::Unit,
                span: crate::lexer::Span::test(),
            }),
        ],
        ty: Type::Unit,
    };

    let processed = mono.process_function(function);
    let HirStmt::Expr(expr) = &processed.body.stmts[2] else {
        panic!("expected expression statement");
    };

    assert_eq!(expr.ty, Type::I64);
}

#[test]
fn parameter_type_tracking_uses_hir_local_id_not_source_name() {
    let mut mono = Monomorphizer::new();
    let mut function = empty_function(None);
    function.params = vec![HirParam {
        name: "value".to_string(),
        local_id: HirLocalId(0),
        ty: Type::I64,
        mutable: false,
        is_ref: false,
    }];
    function.ret_type = Type::I64;
    function.body = HirBlock {
        stmts: vec![
            HirStmt::Let {
                name: "value".to_string(),
                local_id: HirLocalId(1),
                ty: Type::Bool,
                value: HirExpr {
                    kind: HirExprKind::BoolLiteral(true),
                    ty: Type::Bool,
                    span: crate::lexer::Span::test(),
                },
                mutable: false,
            },
            HirStmt::Expr(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "value".to_string(),
                    target: HirVarTarget::Local(HirLocalId(0)),
                }),
                ty: Type::Bool,
                span: crate::lexer::Span::test(),
            }),
        ],
        ty: Type::I64,
    };

    let processed = mono.process_function(function);
    let HirStmt::Expr(expr) = &processed.body.stmts[1] else {
        panic!("expected expression statement");
    };

    assert_eq!(expr.ty, Type::I64);
}

#[test]
fn process_resolved_callee_param_lookup_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    let apply_id = DefId::new(CrateId(0), LocalDefId(978));
    let identity_id = DefId::new(CrateId(0), LocalDefId(979));
    let wrong_id = DefId::new(CrateId(0), LocalDefId(980));
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let apply = callback_acceptor_for_process_test(
        apply_id,
        "apply",
        Type::function(vec![Type::I64], Type::I64),
    );
    let wrong = callback_acceptor_for_process_test(
        wrong_id,
        "wrong_alias",
        Type::function(vec![Type::I32], Type::I32),
    );

    mono.register_concrete_function("apply".to_string(), apply);
    mono.register_generic_function("identity".to_string(), identity);
    mono.register_generic_function("wrong_alias".to_string(), wrong);
    mono.resolver
        .item_paths
        .insert("apply".to_string(), apply_id);
    mono.resolver
        .item_paths
        .insert("identity".to_string(), identity_id);
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), wrong_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Function(apply_id),
                }),
                ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::Var("identity".to_string()),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(apply_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    assert_eq!(args[0].ty, Type::function(vec![Type::I64], Type::I64));
}

#[test]
fn process_resolved_generic_function_argument_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    let apply_id = DefId::new(CrateId(0), LocalDefId(981));
    let identity_id = DefId::new(CrateId(0), LocalDefId(982));
    let wrong_id = DefId::new(CrateId(0), LocalDefId(983));
    let apply = callback_acceptor_for_process_test(
        apply_id,
        "apply",
        Type::function(vec![Type::I64], Type::I64),
    );
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_concrete_function("apply".to_string(), apply);
    mono.register_generic_function("identity".to_string(), identity);
    mono.register_generic_function("wrong_alias".to_string(), wrong);
    mono.resolver
        .item_paths
        .insert("apply".to_string(), apply_id);
    mono.resolver
        .item_paths
        .insert("identity".to_string(), identity_id);
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), wrong_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "apply".to_string(),
                    target: HirVarTarget::Function(apply_id),
                }),
                ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Function(identity_id),
                }),
                ty: Type::function(
                    vec![Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    })],
                    Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    }),
                ),
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(apply_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &args[0].kind else {
        panic!("function argument should be instance-backed");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("function argument should target a monomorphized instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
    assert_eq!(args[0].ty, Type::function(vec![Type::I64], Type::I64));
}

#[test]
fn process_targetless_generic_function_argument_does_not_specialize_by_name() {
    let mut mono = Monomorphizer::new();
    let apply_id = DefId::new(CrateId(0), LocalDefId(991));
    let identity_id = DefId::new(CrateId(0), LocalDefId(992));
    let apply = callback_acceptor_for_process_test(
        apply_id,
        "apply",
        Type::function(vec![Type::I64], Type::I64),
    );
    let identity = generic_identity_for_process_test(identity_id, "identity");

    mono.register_concrete_function("apply".to_string(), apply);
    mono.register_generic_function("identity".to_string(), identity);
    mono.resolver
        .item_paths
        .insert("apply".to_string(), apply_id);
    mono.resolver
        .item_paths
        .insert("identity".to_string(), identity_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "apply".to_string(),
                    target: HirVarTarget::Function(apply_id),
                }),
                ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::Var("identity".to_string()),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(apply_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    assert!(matches!(&args[0].kind, HirExprKind::Var(name) if name == "identity"));
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn process_targetless_concrete_callee_does_not_specialize_argument_by_name() {
    let mut mono = Monomorphizer::new();
    let apply_id = DefId::new(CrateId(0), LocalDefId(1988));
    let identity_id = DefId::new(CrateId(0), LocalDefId(1989));
    let apply = callback_acceptor_for_process_test(
        apply_id,
        "apply",
        Type::function(vec![Type::I64], Type::I64),
    );
    let identity = generic_identity_for_process_test(identity_id, "identity");

    mono.register_concrete_function("apply".to_string(), apply);
    mono.register_generic_function("identity".to_string(), identity);
    mono.resolver
        .item_paths
        .insert("identity".to_string(), identity_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::Var("apply".to_string()),
                ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "identity".to_string(),
                    target: HirVarTarget::Function(identity_id),
                }),
                ty: Type::function(
                    vec![Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    })],
                    Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    }),
                ),
                span: crate::lexer::Span::test(),
            }],
            None,
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    assert!(matches!(args[0].kind, HirExprKind::ResolvedVar(_)));
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn process_external_generic_function_argument_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    mono.register_test_dependency_identity(CrateId(1));
    let apply_id = DefId::new(CrateId(0), LocalDefId(988));
    let identity_id = DefId::new(CrateId(1), LocalDefId(989));
    let wrong_id = DefId::new(CrateId(1), LocalDefId(990));
    let apply = callback_acceptor_for_process_test(
        apply_id,
        "apply",
        Type::function(vec![Type::I64], Type::I64),
    );
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_concrete_function("apply".to_string(), apply);
    mono.register_external_generic_function("dep::identity".to_string(), identity);
    mono.register_external_generic_function("wrong_alias".to_string(), wrong);
    mono.resolver
        .item_paths
        .insert("apply".to_string(), apply_id);
    mono.resolver
        .item_paths
        .insert("dep::identity".to_string(), identity_id);
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), wrong_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "apply".to_string(),
                    target: HirVarTarget::Function(apply_id),
                }),
                ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Extern(identity_id),
                }),
                ty: Type::function(
                    vec![Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    })],
                    Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    }),
                ),
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(apply_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &args[0].kind else {
        panic!("function argument should be instance-backed");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("function argument should target a monomorphized instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
}

#[test]
fn process_external_callee_param_lookup_uses_def_id() {
    let mut mono = Monomorphizer::new();
    mono.register_test_dependency_identity(CrateId(1));
    let apply_id = DefId::new(CrateId(1), LocalDefId(991));
    let identity_id = DefId::new(CrateId(1), LocalDefId(992));
    let wrong_id = DefId::new(CrateId(1), LocalDefId(993));
    let apply_generic_id = GenericParamId {
        owner: apply_id,
        index: 0,
    };
    let apply = callback_acceptor_for_process_test(
        apply_id,
        "apply",
        Type::function(vec![Type::I64], Type::I64),
    );
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_external_generic_function("dep::apply".to_string(), apply);
    mono.register_external_generic_function("dep::identity".to_string(), identity);
    mono.register_external_generic_function("wrong_alias".to_string(), wrong);
    mono.resolver
        .item_paths
        .insert("dep::apply".to_string(), apply_id);
    mono.resolver
        .item_paths
        .insert("dep::identity".to_string(), identity_id);
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), wrong_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "dep::apply".to_string(),
                    target: HirVarTarget::Extern(apply_id),
                }),
                ty: Type::function(
                    vec![Type::function(vec![Type::I64], Type::I64)],
                    Type::Generic(apply_generic_id),
                ),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Extern(identity_id),
                }),
                ty: Type::function(
                    vec![Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    })],
                    Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    }),
                ),
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(apply_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &args[0].kind else {
        panic!("function argument should be instance-backed");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("function argument should target a monomorphized instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
}

#[test]
fn process_instance_callee_does_not_fall_back_to_static_impl_name_lookup() {
    let mut mono = Monomorphizer::new();
    let free_id = DefId::new(CrateId(0), LocalDefId(974));
    let impl_id = DefId::new(CrateId(0), LocalDefId(975));
    let method_id = DefId::new(CrateId(0), LocalDefId(976));
    let free_function = generic_identity_for_process_test(free_id, "Box_make");

    mono.register_generic_function("Box_make".to_string(), free_function);
    mono.generic_impls.insert(
        impl_id,
        generic_static_impl_for_process_test("Box", impl_id, method_id),
    );
    mono.resolver
        .item_paths
        .insert("Box_make".to_string(), free_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "Box_make".to_string(),
                    target: HirVarTarget::Function(free_id),
                }),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::IntLiteral(42),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(free_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("callee should be instance-backed");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("callee should target a monomorphized instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
    assert_eq!(record.origin, InstanceOrigin::Function(free_id));
    assert_ne!(
        record.origin,
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: method_id,
        }
    );
}

#[test]
fn process_static_impl_function_target_is_not_materialized_without_authority() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(950));
    let method_id = DefId::new(CrateId(0), LocalDefId(951));
    let colliding_function_id = DefId::new(CrateId(0), LocalDefId(952));
    let generic_id = GenericParamId {
        owner: colliding_function_id,
        index: 0,
    };
    mono.generic_impls.insert(
        impl_id,
        generic_static_impl_for_process_test("Box", impl_id, method_id),
    );
    mono.generic_functions.insert(
        colliding_function_id,
        HirFunction {
            id: colliding_function_id,
            name: "wrong_alias".to_string(),
            generic_params: vec![GenericParamDecl::type_param(generic_id, "T")],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(generic_id),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Generic(generic_id),
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Generic(generic_id),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        },
    );
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), colliding_function_id);
    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Function(method_id),
                }),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::IntLiteral(7),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(method_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("static impl call should remain a call");
    };
    assert!(matches!(
        &callee.kind,
        HirExprKind::ResolvedVar(HirVarRef {
            target: HirVarTarget::Function(id),
            ..
        }) if *id == method_id
    ));
    assert_eq!(target, &Some(HirCallTarget::Function(method_id)));
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn process_static_impl_function_value_is_not_materialized_without_authority() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(960));
    let method_id = DefId::new(CrateId(0), LocalDefId(961));
    mono.generic_impls.insert(
        impl_id,
        generic_static_impl_for_process_test("Box", impl_id, method_id),
    );
    let mut expr = HirExpr {
        kind: HirExprKind::ResolvedVar(HirVarRef {
            name: "Box_make".to_string(),
            target: HirVarTarget::Function(method_id),
        }),
        ty: Type::function(vec![Type::I64], Type::I64),
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::ResolvedVar(reference) = &expr.kind else {
        panic!("static impl function value should remain a resolved var");
    };
    assert_eq!(reference.target, HirVarTarget::Function(method_id));
    assert!(mono.instances.records().next().is_none());
    assert_eq!(expr.ty, Type::function(vec![Type::I64], Type::I64));
}

#[test]
fn process_call_does_not_resolve_callee_by_backend_symbol() {
    let mut mono = Monomorphizer::new();
    let apply_id = DefId::new(CrateId(0), LocalDefId(915));
    let identity_id = DefId::new(CrateId(0), LocalDefId(916));
    let generic_id = GenericParamId {
        owner: identity_id,
        index: 0,
    };
    let generic_identity = HirFunction {
        id: identity_id,
        name: "identity".to_string(),
        generic_params: vec![GenericParamDecl::type_param(generic_id, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(generic_id),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(generic_id),
        body: HirBlock {
            stmts: vec![HirStmt::Return(Some(HirExpr {
                kind: HirExprKind::Var("value".to_string()),
                ty: Type::Generic(generic_id),
                span: crate::lexer::Span::test(),
            }))],
            ty: Type::Generic(generic_id),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    mono.register_generic_function("identity".to_string(), generic_identity);
    mono.resolver
        .item_paths
        .insert("identity".to_string(), identity_id);

    let apply_substitution = mono.intern_types(&[Type::I64]);
    let apply_key =
        crate::mono::InstanceKey::new(InstanceOrigin::Function(apply_id), apply_substitution);
    let apply_body = HirFunction {
        id: apply_id,
        name: "apply".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "callback".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::function(vec![Type::I64], Type::I64),
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
    let apply_instance =
        mono.instances
            .intern(apply_key.clone(), |id| crate::mono::InstanceRecord {
                id,
                origin: apply_key.origin.clone(),
                substitution: apply_key.substitution.clone(),
                symbols: crate::mono::InstanceSymbols::new("apply", "apply_mono_0"),
                declared: None,
                provided_by_object: false,
                is_specialization: true,
            });
    mono.instances
        .insert_pre_mir_body(apply_instance, apply_body);
    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::Var("apply_mono_0".to_string()),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::Var("identity".to_string()),
                ty: Type::function(vec![Type::Generic(generic_id)], Type::Generic(generic_id)),
                span: crate::lexer::Span::test(),
            }],
            None,
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    assert_eq!(mono.instances.len(), 1);
    assert!(
        matches!(&args[0].kind, HirExprKind::Var(name) if name == "identity"),
        "backend symbols must not recover instance bodies for argument specialization"
    );
}

#[test]
fn process_selected_inherent_method_skips_trait_specialization_path() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(701));
    let inherent_method_id = DefId::new(CrateId(0), LocalDefId(702));
    let trait_method_id = DefId::new(CrateId(0), LocalDefId(703));
    let trait_id = DefId::new(CrateId(0), LocalDefId(704));
    mono.resolver
        .item_names_by_id
        .insert(DefId::new(CrateId(0), LocalDefId(700)), "Box".to_string());
    mono.generic_impls.insert(
        impl_id,
        generic_impl_for_process_test("Box", impl_id, None, inherent_method_id),
    );
    mono.trait_impls.insert(
        trait_id,
        vec![generic_impl_for_process_test(
            "Box",
            impl_id,
            Some(trait_id),
            trait_method_id,
        )],
    );
    seed_box_receiver_pattern(&mut mono, impl_id);
    let recv = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(700)),
            args: vec![Type::I64],
        },
        span: crate::lexer::Span::test(),
    };
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv),
            "map".to_string(),
            Vec::new(),
            Some(ReceiverMode::Move),
            selected_process_method_target(impl_id, inherent_method_id),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let origins = mono
        .instances
        .records()
        .map(|record| record.origin.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        origins,
        vec![InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: inherent_method_id,
        }]
    );
}

#[test]
fn process_selected_generic_method_call_uses_instance_target() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(921));
    let method_id = DefId::new(CrateId(0), LocalDefId(922));
    mono.resolver
        .item_names_by_id
        .insert(DefId::new(CrateId(0), LocalDefId(700)), "Box".to_string());
    mono.generic_impls.insert(
        impl_id,
        generic_impl_for_process_test("Box", impl_id, None, method_id),
    );
    seed_box_receiver_pattern(&mut mono, impl_id);
    let recv = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(700)),
            args: vec![Type::I64],
        },
        span: crate::lexer::Span::test(),
    };
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv),
            "map".to_string(),
            Vec::new(),
            Some(ReceiverMode::Move),
            selected_process_method_target(impl_id, method_id),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, _) = &expr.kind else {
        panic!("method call should lower to a direct call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("method callee should be an instance-backed resolved var");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("method callee should target an instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(
        record.origin,
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: method_id,
        }
    );
}

#[test]
fn process_targetless_generic_method_call_does_not_specialize_by_name() {
    let mut mono = Monomorphizer::new();
    let source_span = crate::lexer::Span {
        file_path: "/test.rk".into(),
        start: 0,
        end: 3,
    };
    let impl_id = DefId::new(CrateId(0), LocalDefId(941));
    let method_id = DefId::new(CrateId(0), LocalDefId(942));
    mono.resolver
        .item_names_by_id
        .insert(DefId::new(CrateId(0), LocalDefId(700)), "Box".to_string());
    mono.generic_impls.insert(
        impl_id,
        generic_impl_for_process_test("Box", impl_id, None, method_id),
    );
    seed_box_receiver_pattern(&mut mono, impl_id);
    let recv = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(700)),
            args: vec![Type::I64],
        },
        span: source_span.clone(),
    };
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv),
            "map".to_string(),
            Vec::new(),
            Some(ReceiverMode::Move),
            HirMethodCallTarget::trait_method(
                DefId::new(CrateId(0), LocalDefId(999)),
                DefId::new(CrateId(0), LocalDefId(998)),
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::I64,
        span: source_span,
    };

    mono.process_expr(&mut expr);

    assert!(
        matches!(expr.kind, HirExprKind::MethodCall(..)),
        "targetless semantic method dispatch must not be rediscovered by name"
    );
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn process_reused_generic_method_call_keeps_instance_target() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(931));
    let method_id = DefId::new(CrateId(0), LocalDefId(932));
    mono.resolver
        .item_names_by_id
        .insert(DefId::new(CrateId(0), LocalDefId(700)), "Box".to_string());
    mono.generic_impls.insert(
        impl_id,
        generic_impl_for_process_test("Box", impl_id, None, method_id),
    );
    seed_box_receiver_pattern(&mut mono, impl_id);

    let mut first = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKind::Var("first".to_string()),
                ty: Type::Struct {
                    id: DefId::new(CrateId(0), LocalDefId(700)),
                    args: vec![Type::I64],
                },
                span: crate::lexer::Span::test(),
            }),
            "map".to_string(),
            Vec::new(),
            Some(ReceiverMode::Move),
            selected_process_method_target(impl_id, method_id),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };
    let mut second = first.clone();

    mono.process_expr(&mut first);
    mono.process_expr(&mut second);

    let first_id = match &first.kind {
        HirExprKind::Call(callee, _, _) => match &callee.kind {
            HirExprKind::ResolvedVar(reference) => match reference.target {
                HirVarTarget::Instance(id) => id,
                _ => panic!("first call should target an instance"),
            },
            _ => panic!("first call callee should be resolved"),
        },
        _ => panic!("first method call should lower to a direct call"),
    };
    let second_id = match &second.kind {
        HirExprKind::Call(callee, _, _) => match &callee.kind {
            HirExprKind::ResolvedVar(reference) => match reference.target {
                HirVarTarget::Instance(id) => id,
                _ => panic!("second call should target an instance"),
            },
            _ => panic!("second call callee should be resolved"),
        },
        _ => panic!("second method call should lower to a direct call"),
    };

    assert_eq!(first_id, second_id);
}
