use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::types::{ObjectAssociatedType, ObjectBinding, TraitBound};

use super::*;

fn id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}
fn object(principal: Vec<Type>, guarantees: Vec<Type>) -> Type {
    let mut object = ObjectType::new(TraitBound {
        trait_id: id(1),
        type_args: principal,
    });
    object
        .guarantees
        .extend(guarantees.into_iter().map(|ty| TraitBound {
            trait_id: id(2),
            type_args: vec![ty],
        }));
    Type::Object(Box::new(object))
}

#[test]
fn object_unification_matches_unordered_views_and_collapsed_duplicates() {
    for actual_guarantees in [vec![Type::I64, Type::Bool], vec![Type::I64]] {
        let mut engine = InferenceEngine::new();
        let variable = engine.fresh_type_var();
        let expected = object(vec![], vec![variable.clone(), Type::I64]);
        let actual = object(vec![], actual_guarantees.clone());
        engine.unify(&expected, &actual).unwrap();
        let expected_argument = if actual_guarantees.len() == 1 {
            Type::I64
        } else {
            Type::Bool
        };
        assert_eq!(engine.resolve(&variable), expected_argument);
        assert_eq!(engine.resolve(&expected), actual);
    }
}

#[test]
fn ambiguous_object_matching_does_not_commit_branch_guesses() {
    let mut engine = InferenceEngine::new();
    let principal = engine.fresh_type_var();
    let first = engine.fresh_type_var();
    let second = engine.fresh_type_var();
    let expected = object(vec![principal.clone()], vec![first.clone(), second.clone()]);
    let actual = object(vec![Type::I64], vec![Type::I64, Type::Bool]);
    assert!(matches!(
        engine.unify(&expected, &actual),
        Err(UnifyError::ObjectMatchPending { .. })
    ));
    assert_eq!(engine.resolve(&principal), Type::I64);
    assert_eq!(engine.resolve(&first), first);
    assert_eq!(engine.resolve(&second), second);
    engine.unify(&first, &Type::I64).unwrap();
    engine.unify(&expected, &actual).unwrap();
    assert_eq!(engine.resolve(&second), Type::Bool);
}

#[test]
fn object_associated_bindings_are_invariant_even_for_numeric_types() {
    let mut left = ObjectType::new(TraitBound {
        trait_id: id(1),
        type_args: vec![],
    });
    let binding = |ty| ObjectBinding {
        key: ObjectAssociatedType {
            trait_ref: left.principal.clone(),
            member: AssocTypeId(0),
        },
        ty,
    };
    let first = binding(Type::I32);
    let second = binding(Type::I64);
    left.bindings.insert(first);
    let mut right = left.clone();
    right.bindings.clear();
    right.bindings.insert(second);
    assert!(InferenceEngine::new()
        .unify(
            &Type::Object(Box::new(left)),
            &Type::Object(Box::new(right))
        )
        .is_err());
}

#[test]
fn object_self_cannot_escape_through_a_global_inference_variable() {
    let mut engine = InferenceEngine::new();
    let variable = engine.fresh_type_var();
    assert!(matches!(
        engine.unify(&variable, &Type::ObjectSelf { depth: 0 }),
        Err(UnifyError::ObjectBinderEscape { .. })
    ));
    assert_eq!(engine.resolve(&variable), variable);
    engine
        .unify(
            &variable,
            &object(vec![Type::ObjectSelf { depth: 0 }], vec![]),
        )
        .unwrap();
}

#[test]
fn pending_object_equations_wake_after_safe_principal_progress() {
    use crate::infer::constraints::ConstraintStore;
    let mut engine = InferenceEngine::new();
    let principal = engine.fresh_type_var();
    let first = engine.fresh_type_var();
    let second = engine.fresh_type_var();
    let mut constraints = ConstraintStore::new();
    constraints.add_equality(
        object(vec![principal.clone()], vec![first.clone(), second.clone()]),
        object(vec![Type::I64], vec![Type::I64, Type::Bool]),
        crate::lexer::Span::test(),
        "object equation",
    );
    constraints.add_equality(
        first.clone(),
        principal,
        crate::lexer::Span::test(),
        "outside equation",
    );
    let result = crate::infer::solve::solve_constraints_in_place(
        &mut engine,
        &mut constraints,
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(engine.resolve(&first), Type::I64);
    assert_eq!(engine.resolve(&second), Type::Bool);
}
