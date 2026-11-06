use super::*;
use crate::ids::{CrateId, DefId, LocalDefId};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

impl GenericScope {
    pub(crate) fn trait_generics(&self) -> &[String] {
        &self.trait_generics
    }
}

#[test]
fn generic_scope_leave_restores_the_entered_owner_and_params() {
    let outer = def_id(1);
    let inner = def_id(2);
    let mut scope = GenericScope::default();
    scope.enter(outer, vec!["T".to_string()], vec![Kind::Type]);

    let frame = scope.enter(inner, vec!["U".to_string()], vec![Kind::Type]);
    assert_eq!(scope.owner(), Some(inner));
    assert_eq!(scope.params(), ["U".to_string()]);
    scope.leave(frame);

    assert_eq!(scope.owner(), Some(outer));
    assert_eq!(scope.params(), ["T".to_string()]);
    assert_eq!(scope.kinds(), [Kind::Type]);
}

#[test]
fn generic_scope_restore_params_drops_declared_params() {
    let mut scope = GenericScope::default();
    scope.enter(def_id(1), vec!["T".to_string()], vec![Kind::Type]);

    let snapshot = scope.snapshot_params();
    assert_eq!(scope.declare("U".to_string(), Kind::Type), 1);
    scope.restore_params(snapshot);

    assert_eq!(scope.params(), ["T".to_string()]);
    assert_eq!(scope.kinds().len(), 1);
}

#[test]
fn generic_scope_leave_trait_restores_the_outer_trait() {
    let mut scope = GenericScope::default();
    scope.enter_trait("Outer".to_string(), vec!["X".to_string()]);

    let frame = scope.enter_trait("Inner".to_string(), Vec::new());
    assert_eq!(scope.trait_name(), Some("Inner"));
    scope.leave_trait(frame);

    assert_eq!(scope.trait_name(), Some("Outer"));
    assert_eq!(scope.trait_generics(), ["X".to_string()]);
}
