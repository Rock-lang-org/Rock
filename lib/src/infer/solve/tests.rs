use super::impl_lookup::impl_exists_for;
use super::*;
use crate::hir::HirImplReceiverPattern;
use crate::infer::constraints::ObligationState;
use crate::types::GenericParamId;

use crate::hir::HirImplOwner;
use crate::ids::{AssocTypeId, CrateId, LocalDefId, TypeVarId};
use crate::language_items::FnOnceLanguageItems;
use crate::lexer::Span;
use crate::types::TraitBound;

#[test]
fn solve_errors_keep_unification_payload_until_rendering() {
    let error = SolveError {
        kind: SolveErrorKind::Unify {
            context: "assignment".to_string(),
            error: UnifyError::Mismatch {
                expected: Type::I64,
                found: Type::Bool,
            },
        },
        span: Span::test(),
    };
    let engine = InferenceEngine::new();
    assert!(matches!(error.kind, SolveErrorKind::Unify { .. }));
    assert!(error.render(&engine).contains("assignment"));
    assert!(error.render(&engine).contains("I64"));
    assert!(error.render(&engine).contains("Bool"));
    assert!(error.contains("Type mismatch"));
}

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn test_impl(type_name: &str, owner_id: DefId, trait_id: DefId) -> HirImpl {
    HirImpl {
        id: def_id(100),
        owner: HirImplOwner::Named(type_name.to_string()),
        type_name: type_name.to_string(),
        type_generics: Vec::new(),
        receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
            id: owner_id,
            args: Vec::new(),
        }),
        trait_name: Some("Show".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::new(),
    }
}

#[test]
fn solve_constraints_returns_generic_bounds_by_typed_type_var_id() {
    let mut engine = InferenceEngine::new();
    let mut store = ConstraintStore::new();
    store.add_trait(
        Type::TypeVar(TypeVarId(0)),
        TraitBound {
            trait_id: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(1)),
            type_args: Vec::new(),
        },
        Span::test(),
        "test",
    );

    let result = solve_constraints_in_place(
        &mut engine,
        &mut store,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &LanguageItems::default(),
    );

    assert_eq!(
        result.generic_bounds.get(&TypeVarId(0)),
        Some(&vec![TraitBound {
            trait_id: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(1)),
            type_args: Vec::new(),
        }])
    );
}

#[test]
fn concrete_trait_bound_matches_struct_impl_by_def_id() {
    let trait_id = def_id(1);
    let struct_id = def_id(2);
    let mut structs = HashMap::new();
    structs.insert(
        struct_id,
        HirStruct {
            id: struct_id,
            name: "Widget".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        },
    );

    assert!(impl_exists_for(
        &Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
        trait_id,
        &[],
        &HashMap::from([(def_id(3), test_impl("Widget", struct_id, trait_id))]),
        &structs,
        &HashMap::new(),
        BuiltinTraitIds::default(),
    ));
}

#[test]
fn constructor_trait_selection_solver_matches_constructor_impl_by_def_id() {
    let trait_id = def_id(20);
    let enum_id = def_id(21);
    let constructor = Type::Constructor {
        id: enum_id,
        flavor: crate::types::NominalTypeKind::Enum,
    };
    let mut imp = test_impl("Maybe", enum_id, trait_id);
    imp.receiver_pattern = crate::hir::HirImplReceiverPattern::Constructor(constructor.clone());
    let impls = HashMap::from([(imp.id, imp)]);
    let enums = HashMap::from([(
        enum_id,
        HirEnum {
            id: enum_id,
            name: "Maybe".to_string(),
            generic_params: vec![crate::types::GenericParamDecl::type_param(
                crate::types::GenericParamId {
                    owner: enum_id,
                    index: 0,
                },
                "T",
            )],
            variants: Vec::new(),
        },
    )]);
    let mut store = ConstraintStore::new();
    store.add_trait(
        constructor,
        TraitBound {
            trait_id,
            type_args: Vec::new(),
        },
        Span::test(),
        "constructor bound",
    );

    let result = solve_constraints_in_place(
        &mut InferenceEngine::new(),
        &mut store,
        &impls,
        &HashMap::new(),
        &enums,
        &HashMap::new(),
        &LanguageItems::default(),
    );

    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn constructor_candidate_with_unsatisfied_impl_bound_is_not_selected() {
    let constructor_trait = def_id(24);
    let required_trait = def_id(25);
    let enum_id = def_id(26);
    let impl_id = def_id(27);
    let generic = crate::types::GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let mut imp = test_impl("Bounded", enum_id, constructor_trait);
    imp.id = impl_id;
    imp.type_generics = vec![crate::types::GenericParamDecl::type_param(generic, "T")];
    imp.receiver_pattern = HirImplReceiverPattern::Constructor(Type::Constructor {
        id: enum_id,
        flavor: crate::types::NominalTypeKind::Enum,
    });
    imp.bounds = HashMap::from([(
        generic,
        vec![TraitBound {
            trait_id: required_trait,
            type_args: Vec::new(),
        }],
    )])
    .into();
    let impls = HashMap::from([(impl_id, imp)]);
    let mut engine = InferenceEngine::new();
    let constructor = engine.fresh_type_var_of_kind(crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    ));
    let Type::TypeVar(constructor_id) = constructor else {
        unreachable!();
    };
    let mut store = ConstraintStore::new();
    let origin = Span {
        file_path: "main.rk".into(),
        start: 14,
        end: 21,
    };
    store.add_trait(
        Type::TypeVar(constructor_id),
        TraitBound {
            trait_id: constructor_trait,
            type_args: Vec::new(),
        },
        origin.clone(),
        "bounded constructor candidate",
    );

    let result = solve_constraints_in_place(
        &mut engine,
        &mut store,
        &impls,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &LanguageItems::default(),
    );

    assert!(result.errors[0].contains("bounded constructor candidate"));
    assert_eq!(result.errors[0].span.file_path, origin.file_path);
    assert_eq!(
        (result.errors[0].span.start, result.errors[0].span.end),
        (origin.start, origin.end)
    );
    assert_eq!(
        engine.resolve(&Type::TypeVar(constructor_id)),
        Type::TypeVar(constructor_id)
    );
}

#[test]
fn structural_worklist_retries_hkt_equality_after_callable_progress() {
    let callable_trait = def_id(40);
    let carrier_id = def_id(41);
    let mut engine = InferenceEngine::new();
    let mut normalization_env = crate::type_services::normalize::TypeNormalizationEnv::new();
    normalization_env.register_nominal(carrier_id, crate::types::NominalTypeKind::Enum, 1);
    engine.set_normalization_env(normalization_env);
    let constructor = engine.fresh_type_var_of_kind(crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    ));
    let payload = engine.fresh_type_var();
    let callable_result = engine.fresh_type_var();
    let callable = Type::function(
        vec![Type::I64],
        Type::Enum {
            id: carrier_id,
            args: vec![Type::Bool],
        },
    );
    let application = Type::Apply {
        constructor: Box::new(constructor.clone()),
        args: vec![payload.clone()],
    };
    let mut store = ConstraintStore::new();
    store.add_equality(
        application,
        callable_result.clone(),
        Span::test(),
        "deferred HKT equality",
    );
    store.add_trait(
        callable,
        TraitBound {
            trait_id: callable_trait,
            type_args: vec![Type::I64, callable_result.clone()],
        },
        Span::test(),
        "callable result",
    );
    let language_items = LanguageItems {
        fn_once: Some(FnOnceLanguageItems {
            trait_id: callable_trait,
            output_id: AssocTypeId(0),
            method_id: def_id(42),
        }),
        ..LanguageItems::default()
    };

    let result = solve_constraints_in_place(
        &mut engine,
        &mut store,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &language_items,
    );

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        engine.resolve(&callable_result).to_string(),
        Type::Enum {
            id: carrier_id,
            args: vec![Type::Bool],
        }
        .to_string()
    );
    assert_eq!(
        engine.resolve(&constructor),
        Type::Constructor {
            id: carrier_id,
            flavor: crate::types::NominalTypeKind::Enum,
        }
    );
    assert_eq!(engine.resolve(&payload), Type::Bool);
}

#[test]
fn structural_worklist_wakes_try_after_carrier_progress() {
    let mut engine = InferenceEngine::new();
    let carrier = engine.fresh_type_var();
    let mut store = ConstraintStore::new();
    let try_obligation = store.add_try(
        carrier.clone(),
        Type::Bool,
        Type::I64,
        Type::I64,
        Span::test(),
    );
    store.add_equality(
        carrier.clone(),
        Type::I64,
        Span::test(),
        "deferred carrier equality",
    );

    let result = solve_constraints_in_place(
        &mut engine,
        &mut store,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &LanguageItems::default(),
    );

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(engine.resolve(&carrier), Type::I64);
    assert_eq!(
        store.obligation_state(try_obligation),
        Some(ObligationState::Solved)
    );
    assert_eq!(store.obligation_attempts(try_obligation), Some(2));
}

#[test]
fn try_obligation_accepts_generalized_constructor_heads() {
    let engine = &mut InferenceEngine::new();
    let generic = Type::Generic(GenericParamId {
        owner: def_id(49),
        index: 0,
    });
    let carrier = Type::Apply {
        constructor: Box::new(generic.clone()),
        args: vec![Type::I64],
    };
    let mut store = ConstraintStore::new();
    let obligation = store.add_try(carrier.clone(), Type::I64, Type::I64, carrier, Span::test());

    let result = solve_constraints_in_place(
        engine,
        &mut store,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &LanguageItems::default(),
    );

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        store.obligation_state(obligation),
        Some(ObligationState::Solved)
    );
}

#[test]
fn structural_worklist_wakes_only_dependent_obligations() {
    let callable_trait = def_id(50);
    let carrier_id = def_id(51);
    let mut engine = InferenceEngine::new();
    let mut normalization_env = crate::type_services::normalize::TypeNormalizationEnv::new();
    normalization_env.register_nominal(carrier_id, crate::types::NominalTypeKind::Enum, 1);
    engine.set_normalization_env(normalization_env);
    let constructor = engine.fresh_type_var_of_kind(crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    ));
    let payload = engine.fresh_type_var();
    let callable_result = engine.fresh_type_var();
    let unrelated = engine.fresh_type_var();
    let application = Type::Apply {
        constructor: Box::new(constructor),
        args: vec![payload],
    };
    let mut store = ConstraintStore::new();
    let dependent = store.add_equality(
        application,
        callable_result.clone(),
        Span::test(),
        "dependent equality",
    );
    let callable = Type::function(
        vec![Type::I64],
        Type::Enum {
            id: carrier_id,
            args: vec![Type::Bool],
        },
    );
    let callable_id = store.add_trait(
        callable,
        TraitBound {
            trait_id: callable_trait,
            type_args: vec![Type::I64, callable_result],
        },
        Span::test(),
        "callable result",
    );
    let unrelated_id = store.add_int_literal(
        match unrelated {
            Type::TypeVar(id) => id,
            _ => unreachable!(),
        },
        Span::test(),
    );
    let language_items = LanguageItems {
        fn_once: Some(FnOnceLanguageItems {
            trait_id: callable_trait,
            output_id: AssocTypeId(0),
            method_id: def_id(52),
        }),
        ..LanguageItems::default()
    };

    let result = solve_constraints_in_place(
        &mut engine,
        &mut store,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &language_items,
    );

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(store.obligation_attempts(dependent), Some(2));
    assert_eq!(store.obligation_attempts(callable_id), Some(1));
    assert_eq!(store.obligation_attempts(unrelated_id), Some(1));
}

#[test]
fn unknown_constructor_heads_remain_ambiguous_at_quiescence() {
    let mut engine = InferenceEngine::new();
    let kind = crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    );
    let left = engine.fresh_type_var_of_kind(kind.clone());
    let right = engine.fresh_type_var_of_kind(kind);
    let mut store = ConstraintStore::new();
    let id = store.add_equality(
        Type::Apply {
            constructor: Box::new(left),
            args: vec![Type::I64],
        },
        Type::Apply {
            constructor: Box::new(right),
            args: vec![Type::Bool],
        },
        Span::test(),
        "unknown constructor heads",
    );

    let result = solve_constraints_in_place(
        &mut engine,
        &mut store,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &LanguageItems::default(),
    );

    assert!(result.errors[0].contains("unknown constructor heads"));
    assert_eq!(store.obligation_state(id), Some(ObligationState::Ambiguous));
}

#[test]
fn multiple_constructor_candidates_remain_ambiguous() {
    let trait_id = def_id(60);
    let first_id = def_id(61);
    let second_id = def_id(62);
    let make_impl = |id: DefId, owner: DefId| HirImpl {
        id,
        owner: HirImplOwner::Named(format!("Constructor{id:?}")),
        type_name: format!("Constructor{id:?}"),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Constructor(Type::Constructor {
            id: owner,
            flavor: crate::types::NominalTypeKind::Enum,
        }),
        trait_name: None,
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::new(),
    };
    let impls = HashMap::from([
        (first_id, make_impl(first_id, first_id)),
        (second_id, make_impl(second_id, second_id)),
    ]);
    let mut engine = InferenceEngine::new();
    let mut normalization_env = crate::type_services::normalize::TypeNormalizationEnv::new();
    normalization_env.register_nominal(first_id, crate::types::NominalTypeKind::Enum, 0);
    normalization_env.register_nominal(second_id, crate::types::NominalTypeKind::Enum, 0);
    engine.set_normalization_env(normalization_env);
    let constructor = engine.fresh_type_var_of_kind(crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    ));
    let Type::TypeVar(constructor_id) = constructor else {
        unreachable!();
    };
    let mut store = ConstraintStore::new();
    let obligation = store.add_trait(
        Type::TypeVar(constructor_id),
        TraitBound {
            trait_id,
            type_args: Vec::new(),
        },
        Span::test(),
        "constructor candidates",
    );

    let result = solve_constraints_in_place(
        &mut engine,
        &mut store,
        &impls,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &LanguageItems::default(),
    );

    assert!(result.errors[0].contains("constructor candidates"));
    assert_eq!(
        store.obligation_state(obligation),
        Some(ObligationState::Ambiguous)
    );
    assert_eq!(
        engine.resolve(&Type::TypeVar(constructor_id)),
        Type::TypeVar(constructor_id)
    );
}

#[test]
fn ambiguous_obligation_reopens_after_new_evidence() {
    let trait_id = def_id(70);
    let first_id = def_id(71);
    let second_id = def_id(72);
    let make_impl = |id: DefId| HirImpl {
        id,
        owner: crate::hir::HirImplOwner::Named(format!("Constructor{id:?}")),
        type_name: format!("Constructor{id:?}"),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Constructor(Type::Constructor {
            id,
            flavor: crate::types::NominalTypeKind::Enum,
        }),
        trait_name: None,
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::new(),
    };
    let impls = HashMap::from([
        (first_id, make_impl(first_id)),
        (second_id, make_impl(second_id)),
    ]);
    let mut engine = InferenceEngine::new();
    let variable = engine.fresh_type_var_of_kind(crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    ));
    let Type::TypeVar(variable_id) = variable else {
        unreachable!();
    };
    let mut store = ConstraintStore::new();
    let obligation = store.add_trait(
        Type::TypeVar(variable_id),
        TraitBound {
            trait_id,
            type_args: Vec::new(),
        },
        Span::test(),
        "ambiguity reopening",
    );
    let result = solve_constraints_in_place(
        &mut engine,
        &mut store,
        &impls,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &LanguageItems::default(),
    );
    assert!(result.errors[0].contains("ambiguity reopening"));
    assert_eq!(
        store.obligation_state(obligation),
        Some(ObligationState::Ambiguous)
    );
    store.reopen_ambiguous(obligation);
    assert_eq!(
        store.obligation_state(obligation),
        Some(ObligationState::Pending)
    );
}

#[test]
fn recursive_supertrait_obligations_terminate_and_are_memoized() {
    let first = def_id(30);
    let second = def_id(31);
    let trait_def = |id: DefId, name: &str, parent: DefId| crate::hir::HirTrait {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        target: None,
        predicates: vec![crate::types::Predicate::Trait {
            subject: Type::Generic(crate::types::GenericParamId {
                owner: id,
                index: 0,
            }),
            trait_id: parent,
            args: Vec::new(),
        }],
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
    };
    let traits = HashMap::from([
        (first, trait_def(first, "First", second)),
        (second, trait_def(second, "Second", first)),
    ]);
    let mut engine = InferenceEngine::new();
    let variable = engine.fresh_type_var();
    let Type::TypeVar(variable_id) = variable.clone() else {
        unreachable!();
    };
    let mut store = ConstraintStore::new();
    store.add_trait(
        variable,
        TraitBound {
            trait_id: first,
            type_args: Vec::new(),
        },
        Span::test(),
        "recursive test",
    );

    let result = solve_constraints_in_place(
        &mut engine,
        &mut store,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &traits,
        &LanguageItems::default(),
    );

    let bounds = &result.generic_bounds[&variable_id];
    assert_eq!(bounds.len(), 2);
    assert!(bounds.iter().any(|bound| bound.trait_id == first));
    assert!(bounds.iter().any(|bound| bound.trait_id == second));
}

#[test]
fn same_named_traits_do_not_satisfy_each_other_by_name() {
    let required_trait_id = def_id(1);
    let wrong_trait_id = def_id(2);
    let struct_id = def_id(3);
    let mut structs = HashMap::new();
    structs.insert(
        struct_id,
        HirStruct {
            id: struct_id,
            name: "Widget".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        },
    );

    assert!(!impl_exists_for(
        &Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
        required_trait_id,
        &[],
        &HashMap::from([(def_id(4), test_impl("Widget", struct_id, wrong_trait_id))]),
        &structs,
        &HashMap::new(),
        BuiltinTraitIds::default(),
    ));
}

#[test]
fn owned_auto_trait_checks_storage_modes_and_nested_payloads_by_id() {
    use crate::types::{CallableKind, CaptureKind, FunctionCapture, FunctionSafety};

    let owned = def_id(701);
    let send = def_id(702);
    let sync = def_id(703);
    let builtin = BuiltinTraitIds { owned: Some(owned), send: Some(send), sync: Some(sync), ..Default::default() };
    let reference = Type::Reference { mutable: false, inner: Box::new(Type::I64) };
    let closure = |kind, ty| Type::function_with_metadata(
        Vec::new(), Type::I64, FunctionSafety::Safe, CallableKind::Fn,
        vec![FunctionCapture::new(kind, ty)],
    );
    let satisfies = |ty: &Type, marker| impl_exists_for(
        ty, marker, &[], &HashMap::new(), &HashMap::new(), &HashMap::new(), builtin,
    );
    for ty in [Type::I64, Type::Tuple(vec![Type::I64]), closure(CaptureKind::Move, Type::I64)] {
        assert!(satisfies(&ty, owned), "{ty:?}");
    }
    for ty in [
        reference.clone(), Type::Tuple(vec![reference.clone()]),
        Type::Array(Box::new(reference.clone()), 2u64.into()),
        closure(CaptureKind::SharedBorrow, Type::I64),
        closure(CaptureKind::MutableBorrow, Type::I64),
        closure(CaptureKind::Move, reference.clone()),
        closure(CaptureKind::Move, closure(CaptureKind::SharedBorrow, Type::I64)),
        Type::function(Vec::new(), Type::I64),
    ] {
        assert!(!satisfies(&ty, owned), "{ty:?}");
    }
    assert!(satisfies(&reference, send));
    assert!(satisfies(&closure(CaptureKind::SharedBorrow, Type::I64), send));
    let item = Type::function_item(Vec::new(), Type::I64, FunctionSafety::Safe);
    assert!(satisfies(&item, owned));
    let erased = Type::function(Vec::new(), Type::I64);
    assert!(!satisfies(&erased, send));
    assert!(!satisfies(&erased, sync));
}
