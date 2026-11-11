use super::*;
use crate::ids::{CrateId, DefId, LocalDefId, TypeVarId};
use crate::infer::constraints::ConstraintStore;

fn test_def(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn constructor(id: DefId, flavor: crate::types::NominalTypeKind) -> Type {
    Type::Constructor { id, flavor }
}

fn apply(constructor: Type, args: Vec<Type>) -> Type {
    Type::Apply {
        constructor: Box::new(constructor),
        args,
    }
}

#[test]
fn constructor_variable_unifies_with_rigid_nominal_spine() {
    let option = test_def(100);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(option, crate::types::NominalTypeKind::Enum, 1);
    let mut engine = InferenceEngine::new();
    engine.set_normalization_env(env);
    let constructor_var = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));

    engine
        .unify(
            &apply(constructor_var.clone(), vec![Type::I64]),
            &Type::Enum {
                id: option,
                args: vec![Type::I64],
            },
        )
        .unwrap();

    assert_eq!(
        engine.resolve(&constructor_var),
        constructor(option, crate::types::NominalTypeKind::Enum)
    );
}

#[test]
fn unification_keeps_mismatch_types_until_rendering() {
    let mut engine = InferenceEngine::new();
    let error = engine.unify(&Type::Bool, &Type::Char).unwrap_err();

    match &error {
        UnifyError::Mismatch { expected, found } => {
            assert_eq!(expected, &Type::Bool);
            assert_eq!(found, &Type::Char);
        }
        other => panic!("expected structured mismatch, got {other:?}"),
    }
    assert!(error.render(&engine).contains("Bool"));
    assert!(error.render(&engine).contains("Char"));
}

#[test]
fn unification_keeps_ambiguous_constructor_heads_structured() {
    let mut engine = InferenceEngine::new();
    let kind = Kind::arrow(Kind::Type, Kind::Type);
    let left = apply(engine.fresh_type_var_of_kind(kind.clone()), vec![Type::I64]);
    let right = apply(engine.fresh_type_var_of_kind(kind), vec![Type::I64]);

    let error = engine.unify(&left, &right).unwrap_err();

    assert!(matches!(
        error,
        UnifyError::AmbiguousConstructorHeads { .. }
    ));
}

#[test]
fn equal_unresolved_constructor_heads_unify_their_arguments_structurally() {
    let mut engine = InferenceEngine::new();
    let constructor_var = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));
    let argument = engine.fresh_type_var();

    engine
        .unify(
            &apply(constructor_var.clone(), vec![argument.clone()]),
            &apply(constructor_var.clone(), vec![Type::I64]),
        )
        .unwrap();

    assert_eq!(engine.resolve(&constructor_var), constructor_var);
    assert_eq!(engine.resolve(&argument), Type::I64);
}

#[test]
fn nested_constructor_variables_unify_from_aligned_rigid_spines() {
    let option = test_def(101);
    let vector = test_def(102);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(option, crate::types::NominalTypeKind::Enum, 1);
    env.register_nominal(vector, crate::types::NominalTypeKind::Struct, 1);
    let mut engine = InferenceEngine::new();
    engine.set_normalization_env(env);
    let unary = Kind::arrow(Kind::Type, Kind::Type);
    let f = engine.fresh_type_var_of_kind(unary.clone());
    let g = engine.fresh_type_var_of_kind(unary);
    let a = engine.fresh_type_var();

    engine
        .unify(
            &apply(f.clone(), vec![apply(g.clone(), vec![a.clone()])]),
            &Type::Struct {
                id: vector,
                args: vec![Type::Enum {
                    id: option,
                    args: vec![Type::I64],
                }],
            },
        )
        .unwrap();

    assert_eq!(
        engine.resolve(&f),
        constructor(vector, crate::types::NominalTypeKind::Struct)
    );
    assert_eq!(
        engine.resolve(&g),
        constructor(option, crate::types::NominalTypeKind::Enum)
    );
    assert_eq!(engine.resolve(&a), Type::I64);
}

#[test]
fn constructor_variable_infers_a_rigid_partial_application() {
    let result = test_def(103);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(result, crate::types::NominalTypeKind::Enum, 2);
    let mut engine = InferenceEngine::new();
    engine.set_normalization_env(env);
    let f = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));
    let a = engine.fresh_type_var();

    engine
        .unify(
            &apply(f.clone(), vec![a.clone()]),
            &Type::Enum {
                id: result,
                args: vec![Type::Bool, Type::I64],
            },
        )
        .unwrap();

    assert_eq!(
        engine.resolve(&f),
        Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::Enum {
                id: result,
                args: vec![
                    Type::BoundVar {
                        depth: 0,
                        index: 0,
                        kind: Kind::Type,
                    },
                    Type::I64
                ],
            }),
        }
    );
    assert_eq!(engine.resolve(&a), Type::Bool);
}

#[test]
fn constructor_variable_matches_a_unique_rigid_section() {
    let result = test_def(104);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(result, crate::types::NominalTypeKind::Enum, 2);
    let mut engine = InferenceEngine::new();
    engine.set_normalization_env(env);
    let f = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));
    let a = engine.fresh_type_var();

    engine
        .unify(
            &apply(f.clone(), vec![a.clone()]),
            &Type::Enum {
                id: result,
                args: vec![Type::I64, Type::Bool],
            },
        )
        .unwrap();

    assert!(matches!(engine.resolve(&f), Type::Lambda { .. }));
    assert_eq!(engine.resolve(&a), Type::I64);
}

#[test]
fn repeated_constructor_hole_mismatch_is_rejected() {
    let result = test_def(106);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(result, crate::types::NominalTypeKind::Enum, 2);
    let mut engine = InferenceEngine::new();
    engine.set_normalization_env(env);
    let constructor =
        engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::arrow(Kind::Type, Kind::Type)));
    let hole = engine.fresh_type_var();

    let error = engine
        .unify(
            &apply(constructor.clone(), vec![hole.clone(), hole]),
            &Type::Enum {
                id: result,
                args: vec![Type::I64, Type::Bool],
            },
        )
        .unwrap_err();

    assert!(error.contains("cannot infer a type constructor"));
    assert!(matches!(engine.resolve(&constructor), Type::TypeVar(_)));
}

#[test]
fn constructor_application_arity_mismatch_is_rejected() {
    let result = test_def(107);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(result, crate::types::NominalTypeKind::Enum, 0);
    let mut engine = InferenceEngine::new();
    engine.set_normalization_env(env);
    let constructor = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));

    let error = engine
        .unify(
            &apply(constructor.clone(), vec![Type::I64]),
            &Type::Enum {
                id: result,
                args: Vec::new(),
            },
        )
        .unwrap_err();

    assert!(error.contains("cannot infer a type constructor"));
    assert!(matches!(engine.resolve(&constructor), Type::TypeVar(_)));
}

#[test]
fn explicit_constructor_lambda_beta_reduces_before_unification() {
    let result = test_def(105);
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(result, crate::types::NominalTypeKind::Enum, 2);
    let mut engine = InferenceEngine::new();
    engine.set_normalization_env(env);
    let f = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));
    let a = engine.fresh_type_var();
    let e = engine.fresh_type_var();
    let section = Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(Type::Enum {
            id: result,
            args: vec![
                Type::BoundVar {
                    depth: 0,
                    index: 0,
                    kind: Kind::Type,
                },
                e.clone(),
            ],
        }),
    };

    engine.unify(&f, &section).unwrap();
    engine
        .unify(
            &apply(f, vec![a.clone()]),
            &Type::Enum {
                id: result,
                args: vec![a, e],
            },
        )
        .unwrap();
}

#[test]
fn constructor_unification_checks_kinds_occurs_and_binder_escape() {
    let mut engine = InferenceEngine::new();
    let unary = Kind::arrow(Kind::Type, Kind::Type);
    let f = engine.fresh_type_var_of_kind(unary.clone());
    assert!(engine
        .unify(&f, &Type::I64)
        .unwrap_err()
        .contains("kind mismatch"));

    let recursive = engine.fresh_type_var_of_kind(unary);
    let recursive_lambda = Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(Type::Tuple(vec![apply(
            recursive.clone(),
            vec![Type::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            }],
        )])),
    };
    assert!(engine
        .unify(&recursive, &recursive_lambda)
        .unwrap_err()
        .contains("Infinite type"));

    let ordinary = engine.fresh_type_var();
    assert!(engine
        .unify(
            &ordinary,
            &Type::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            }
        )
        .unwrap_err()
        .contains("binder escapes"));
}

#[test]
fn constructor_variables_are_not_numeric_defaulted_and_finalize_strictly() {
    let mut engine = InferenceEngine::new();
    let constructor_var = engine.fresh_type_var_of_kind(Kind::arrow(Kind::Type, Kind::Type));
    let Type::TypeVar(id) = constructor_var else {
        unreachable!();
    };
    let mut constraints = ConstraintStore::new();
    constraints.add_int_literal(id, Span::test());

    engine.apply_numeric_defaults(&constraints);
    assert_eq!(engine.resolve(&Type::TypeVar(id)), Type::TypeVar(id));

    let mut errors = Vec::new();
    assert_eq!(
        engine.finalize_strict(&Type::TypeVar(id), &mut errors),
        Type::Error
    );
    assert!(errors[0].contains("ambiguous type constructor"));
}

#[test]
fn probe_progress_does_not_mutate_production_generation() {
    let mut engine = InferenceEngine::new();
    let variable = engine.fresh_type_var();
    let before = engine.substitution_generation();
    let mut probe = engine.clone_for_probe();
    probe.unify(&variable, &Type::I64).unwrap();

    assert_eq!(engine.substitution_generation(), before);
    assert!(matches!(engine.resolve(&variable), Type::TypeVar(_)));
    engine.commit_probe(probe);
    assert!(engine.substitution_generation() > before);
    assert_eq!(engine.resolve(&variable), Type::I64);
}

#[test]
fn fresh_type_var_returns_typed_type_var_id() {
    let mut engine = InferenceEngine::new();

    assert_eq!(engine.fresh_type_var(), Type::TypeVar(TypeVarId(0)));
    assert_eq!(engine.fresh_type_var(), Type::TypeVar(TypeVarId(1)));
}

#[test]
fn fresh_type_var_at_records_span_by_typed_type_var_id() {
    let mut engine = InferenceEngine::new();
    let span = Span::test();

    assert_eq!(
        engine.fresh_type_var_at(span.clone()),
        Type::TypeVar(TypeVarId(0))
    );
    assert_eq!(engine.var_spans.get(&TypeVarId(0)), Some(&span));
}

#[test]
fn numeric_defaults_do_not_default_marker_bound_type_variables() {
    let mut engine = InferenceEngine::new();
    let marker_bound = TraitBound {
        trait_id: DefId::new(CrateId(0), LocalDefId(1)),
        type_args: Vec::new(),
    };
    let ty = engine.fresh_type_var();
    let Type::TypeVar(var_id) = ty else {
        panic!("fresh type variable must be a type variable");
    };
    engine.add_bound(var_id, marker_bound);

    engine.apply_numeric_defaults(&crate::infer::constraints::ConstraintStore::new());

    assert_eq!(engine.resolve(&ty), ty);
}

#[test]
fn numeric_defaults_apply_integer_literal_evidence() {
    let mut engine = InferenceEngine::new();
    let ty = engine.fresh_type_var();
    let Type::TypeVar(var_id) = ty else {
        panic!("fresh type variable must be a type variable");
    };
    let mut constraints = crate::infer::constraints::ConstraintStore::new();
    constraints.add_int_literal(var_id, Span::test());

    engine.apply_numeric_defaults(&constraints);

    assert_eq!(engine.resolve(&ty), Type::I64);
}

#[test]
fn numeric_defaults_apply_integer_evidence_to_the_entire_alias_class() {
    for unify_literal_with_alias in [true, false] {
        let mut engine = InferenceEngine::new();
        let literal = engine.fresh_type_var();
        let alias = engine.fresh_type_var();
        let (Type::TypeVar(literal_id), Type::TypeVar(_)) = (&literal, &alias) else {
            panic!("fresh type variables must be type variables");
        };
        let mut constraints = crate::infer::constraints::ConstraintStore::new();
        constraints.add_int_literal(*literal_id, Span::test());

        if unify_literal_with_alias {
            engine.unify(&literal, &alias).unwrap();
        } else {
            engine.unify(&alias, &literal).unwrap();
        }
        engine.apply_numeric_defaults(&constraints);

        assert_eq!(engine.resolve(&literal), Type::I64);
        assert_eq!(engine.resolve(&alias), Type::I64);
    }
}

#[test]
fn numeric_defaults_apply_float_evidence_to_the_entire_alias_class() {
    for unify_literal_with_alias in [true, false] {
        let mut engine = InferenceEngine::new();
        let literal = engine.fresh_type_var();
        let alias = engine.fresh_type_var();
        let (Type::TypeVar(literal_id), Type::TypeVar(_)) = (&literal, &alias) else {
            panic!("fresh type variables must be type variables");
        };
        let mut constraints = crate::infer::constraints::ConstraintStore::new();
        constraints.add_float_literal(*literal_id, Span::test());

        if unify_literal_with_alias {
            engine.unify(&literal, &alias).unwrap();
        } else {
            engine.unify(&alias, &literal).unwrap();
        }
        engine.apply_numeric_defaults(&constraints);

        assert_eq!(engine.resolve(&literal), Type::F64);
        assert_eq!(engine.resolve(&alias), Type::F64);
    }
}

#[test]
fn numeric_defaults_are_stable_for_same_kind_evidence_across_aliases() {
    for unify_first_with_second in [true, false] {
        let mut engine = InferenceEngine::new();
        let first = engine.fresh_type_var();
        let second = engine.fresh_type_var();
        let (Type::TypeVar(first_id), Type::TypeVar(second_id)) = (&first, &second) else {
            panic!("fresh type variables must be type variables");
        };
        let mut constraints = crate::infer::constraints::ConstraintStore::new();
        constraints.add_int_literal(*first_id, Span::test());
        constraints.add_int_literal(*second_id, Span::test());

        if unify_first_with_second {
            engine.unify(&first, &second).unwrap();
        } else {
            engine.unify(&second, &first).unwrap();
        }
        engine.apply_numeric_defaults(&constraints);

        assert_eq!(engine.resolve(&first), Type::I64);
        assert_eq!(engine.resolve(&second), Type::I64);
    }
}

#[test]
fn numeric_defaults_leave_mixed_literal_evidence_across_aliases_unresolved() {
    for unify_integer_with_float in [true, false] {
        let mut engine = InferenceEngine::new();
        let integer = engine.fresh_type_var();
        let float = engine.fresh_type_var();
        let (Type::TypeVar(integer_id), Type::TypeVar(float_id)) = (&integer, &float) else {
            panic!("fresh type variables must be type variables");
        };
        let mut constraints = crate::infer::constraints::ConstraintStore::new();
        constraints.add_int_literal(*integer_id, Span::test());
        constraints.add_float_literal(*float_id, Span::test());

        if unify_integer_with_float {
            engine.unify(&integer, &float).unwrap();
        } else {
            engine.unify(&float, &integer).unwrap();
        }
        engine.apply_numeric_defaults(&constraints);

        let integer_resolved = engine.resolve(&integer);
        let float_resolved = engine.resolve(&float);
        assert!(matches!(integer_resolved, Type::TypeVar(_)));
        assert_eq!(integer_resolved, float_resolved);
    }
}

#[test]
fn numeric_defaults_do_not_use_marker_bounds_from_aliases() {
    let mut engine = InferenceEngine::new();
    let bounded = engine.fresh_type_var();
    let alias = engine.fresh_type_var();
    let (Type::TypeVar(bounded_id), Type::TypeVar(_)) = (&bounded, &alias) else {
        panic!("fresh type variables must be type variables");
    };
    engine.add_bound(
        *bounded_id,
        TraitBound {
            trait_id: DefId::new(CrateId(0), LocalDefId(2)),
            type_args: Vec::new(),
        },
    );
    engine.unify(&bounded, &alias).unwrap();

    engine.apply_numeric_defaults(&crate::infer::constraints::ConstraintStore::new());

    let bounded_resolved = engine.resolve(&bounded);
    assert!(matches!(bounded_resolved, Type::TypeVar(_)));
    assert_eq!(bounded_resolved, engine.resolve(&alias));
}

#[test]
fn strict_finalization_rejects_unspanned_type_var() {
    let mut engine = InferenceEngine::new();
    let unspanned = engine.fresh_type_var();
    let mut errors = Vec::new();

    assert_eq!(engine.finalize_strict(&unspanned, &mut errors), Type::Error);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("ambiguous type"));
}

#[test]
fn strict_finalization_rejects_unresolved_alias_class() {
    let mut engine = InferenceEngine::new();
    let origin = Span {
        file_path: "main.rk".into(),
        start: 12,
        end: 17,
    };
    let spanned = engine.fresh_type_var_at(origin.clone());
    let unspanned = engine.fresh_type_var();
    engine.unify(&spanned, &unspanned).unwrap();

    let mut errors = Vec::new();
    assert_eq!(engine.finalize_strict(&spanned, &mut errors), Type::Error);
    assert_eq!(errors.len(), 1);
    let span = errors[0].span.as_ref().expect("origin span");
    assert_eq!(span.file_path, origin.file_path);
    assert_eq!((span.start, span.end), (origin.start, origin.end));

    let mut engine = InferenceEngine::new();
    let spanned = engine.fresh_type_var_at(origin.clone());
    let unspanned = engine.fresh_type_var();
    engine.unify(&unspanned, &spanned).unwrap();

    let mut errors = Vec::new();
    assert_eq!(engine.finalize_strict(&unspanned, &mut errors), Type::Error);
    assert_eq!(errors.len(), 1);
    let span = errors[0].span.as_ref().expect("origin span");
    assert_eq!(span.file_path, origin.file_path);
    assert_eq!((span.start, span.end), (origin.start, origin.end));
}

#[test]
fn same_crate_nominal_struct_ids_do_not_unify() {
    let mut engine = InferenceEngine::new();
    let first = Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(1)),
        args: vec![Type::I64],
    };
    let second = Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(2)),
        args: vec![Type::I64],
    };

    assert!(engine.unify(&first, &second).is_err());
}

#[test]
fn function_unification_uses_contravariant_params_for_safety() {
    let mut engine = InferenceEngine::new();
    let accepts_safe = Type::function(vec![Type::function(Vec::new(), Type::I64)], Type::I64);
    let accepts_unsafe = Type::function(
        vec![Type::unsafe_function(Vec::new(), Type::I64)],
        Type::I64,
    );

    assert!(engine.unify(&accepts_safe, &accepts_unsafe).is_err());

    let mut engine = InferenceEngine::new();
    assert!(engine.unify(&accepts_unsafe, &accepts_safe).is_ok());
}

#[test]
fn function_unification_preserves_closure_metadata_through_type_variables() {
    let mut engine = InferenceEngine::new();
    let variable = engine.fresh_type_var();
    let default_signature = Type::function(vec![Type::I64], Type::Bool);
    let closure = Type::function_with_metadata(
        vec![Type::I64],
        Type::Bool,
        crate::types::FunctionSafety::Safe,
        crate::types::CallableKind::FnMut,
        vec![crate::types::FunctionCapture::new(
            crate::types::CaptureKind::MutableBorrow,
            Type::I64,
        )],
    );

    engine.unify(&variable, &default_signature).unwrap();
    engine.unify(&variable, &closure).unwrap();

    assert_eq!(engine.resolve(&variable), closure);
}

#[test]
fn function_signature_unification_treats_default_metadata_as_wildcard() {
    let default_signature = Type::function(Vec::new(), Type::I64);
    let closure = Type::function_with_metadata(
        Vec::new(),
        Type::I64,
        crate::types::FunctionSafety::Safe,
        crate::types::CallableKind::FnOnce,
        vec![crate::types::FunctionCapture::new(
            crate::types::CaptureKind::Move,
            Type::I64,
        )],
    );
    let mut engine = InferenceEngine::new();

    assert!(engine.unify(&default_signature, &closure).is_ok());

    let mut engine = InferenceEngine::new();
    assert!(engine
        .unify(
            &Type::Reference {
                mutable: true,
                inner: Box::new(default_signature),
            },
            &Type::Reference {
                mutable: true,
                inner: Box::new(closure),
            },
        )
        .is_ok());
}

#[test]
fn mutable_ref_and_pointer_unification_are_invariant_for_safety() {
    let safe_fn = Type::function(Vec::new(), Type::I64);
    let unsafe_fn = Type::unsafe_function(Vec::new(), Type::I64);

    let mut engine = InferenceEngine::new();
    assert!(engine
        .unify(
            &Type::Reference {
                mutable: true,
                inner: Box::new(safe_fn.clone()),
            },
            &Type::Reference {
                mutable: true,
                inner: Box::new(unsafe_fn.clone()),
            },
        )
        .is_err());

    let mut engine = InferenceEngine::new();
    assert!(engine
        .unify(
            &Type::Pointer(Box::new(safe_fn)),
            &Type::Pointer(Box::new(unsafe_fn)),
        )
        .is_err());
}

#[test]
fn mutable_ref_and_pointer_unification_are_invariant_for_integer_types() {
    let mut engine = InferenceEngine::new();
    assert!(engine
        .unify(
            &Type::Reference {
                mutable: true,
                inner: Box::new(Type::I8),
            },
            &Type::Reference {
                mutable: true,
                inner: Box::new(Type::I64),
            },
        )
        .is_err());

    let mut engine = InferenceEngine::new();
    assert!(engine
        .unify(
            &Type::Pointer(Box::new(Type::I8)),
            &Type::Pointer(Box::new(Type::I64))
        )
        .is_err());
}

#[test]
fn invariant_unification_rejects_arrays_of_different_concrete_lengths() {
    let mut engine = InferenceEngine::new();
    let two = Type::Array(Box::new(Type::I64), 2u64.into());
    let three = Type::Array(Box::new(Type::I64), 3u64.into());

    assert!(engine.unify_invariant(&two, &three).is_err());
    assert!(engine.unify_invariant(&two, &two).is_ok());
}

#[test]
fn invariant_unification_binds_length_variable_to_concrete_length() {
    let mut engine = InferenceEngine::new();
    let var = engine.fresh_type_var_of_kind(Kind::Usize);
    let Type::TypeVar(var_id) = var else {
        panic!("expected a type variable");
    };
    let open = Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Var(var_id));
    let two = Type::Array(Box::new(Type::I64), 2u64.into());
    let three = Type::Array(Box::new(Type::I64), 3u64.into());

    engine.unify_invariant(&open, &two).unwrap();
    assert!(engine.unify_invariant(&open, &three).is_err());
}
