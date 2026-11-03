use super::*;
use crate::ids::TypeVarId;

#[test]
fn obligations_have_stable_ids_and_pending_initial_state() {
    let mut store = ConstraintStore::new();
    let first = store.add_int_literal(TypeVarId(0), Span::test());
    let second = store.add_float_literal(TypeVarId(1), Span::test());

    assert_eq!(first.raw(), 0);
    assert_eq!(second.raw(), 1);
    assert_eq!(
        store.obligation_state(first),
        Some(ObligationState::Pending)
    );
    assert_eq!(
        store.obligation_state(second),
        Some(ObligationState::Pending)
    );
    assert_eq!(
        store.obligation_ids().collect::<Vec<_>>(),
        vec![first, second]
    );
}

#[test]
fn dependency_index_tracks_all_type_variable_representatives() {
    let mut store = ConstraintStore::new();
    let id = store.add_equality(
        Type::Apply {
            constructor: Box::new(Type::TypeVar(TypeVarId(0))),
            args: vec![Type::TypeVar(TypeVarId(1))],
        },
        Type::TypeVar(TypeVarId(2)),
        Span::test(),
        "dependency test",
    );
    assert!(store.dependencies_for(TypeVarId(0)).contains(&id));
    assert!(store.dependencies_for(TypeVarId(1)).contains(&id));
    assert!(store.dependencies_for(TypeVarId(2)).contains(&id));
}

#[test]
fn try_obligation_tracks_all_linked_type_variables() {
    let mut store = ConstraintStore::new();
    let id = store.add_try(
        Type::TypeVar(TypeVarId(0)),
        Type::TypeVar(TypeVarId(1)),
        Type::TypeVar(TypeVarId(2)),
        Type::TypeVar(TypeVarId(3)),
        Span::test(),
    );
    for var in 0..=3 {
        assert!(store.dependencies_for(TypeVarId(var)).contains(&id));
    }
}

#[test]
fn obligation_state_transitions_are_stable() {
    let mut store = ConstraintStore::new();
    let id = store.add_int_literal(TypeVarId(0), Span::test());

    store.set_state(id, ObligationState::Solved);
    assert_eq!(store.obligation_state(id), Some(ObligationState::Solved));
    store.set_state(id, ObligationState::Failed);
    assert_eq!(store.obligation_state(id), Some(ObligationState::Solved));
    assert_eq!(store.obligation_state(id), Some(ObligationState::Solved));

    let failed = store.add_int_literal(TypeVarId(1), Span::test());
    store.set_state(failed, ObligationState::Failed);
    store.set_state(failed, ObligationState::Ambiguous);
    assert_eq!(
        store.obligation_state(failed),
        Some(ObligationState::Failed)
    );
}

#[test]
fn obligations_record_body_owner_without_cross_owner_visibility() {
    let mut store = ConstraintStore::new();
    let first = store.add_int_literal(TypeVarId(0), Span::test());
    let previous = store.replace_owner(ConstraintOwner::Body(crate::ids::DefId::new(
        crate::ids::CrateId(0),
        crate::ids::LocalDefId(1),
    )));
    let second = store.add_int_literal(TypeVarId(1), Span::test());

    assert_eq!(previous, ConstraintOwner::Global);
    assert_eq!(store.owner(first), Some(ConstraintOwner::Global));
    assert_eq!(
        store.owner(second),
        Some(ConstraintOwner::Body(crate::ids::DefId::new(
            crate::ids::CrateId(0),
            crate::ids::LocalDefId(1),
        )))
    );
    let owners = HashSet::from([ConstraintOwner::Global]);
    assert!(store.includes_owner(first, Some(&owners)));
    assert!(!store.includes_owner(second, Some(&owners)));
}
