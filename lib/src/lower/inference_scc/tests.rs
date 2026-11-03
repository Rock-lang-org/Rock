use super::*;
use crate::ids::{CrateId, LocalDefId};

#[test]
fn components_merge_recursive_edges_deterministically() {
    let id = |local| DefId::new(CrateId(0), LocalDefId(local));
    let nodes = [id(1), id(2), id(3)].into_iter().collect();
    let edges = HashMap::from([
        (id(1), BTreeSet::from([id(2)])),
        (id(2), BTreeSet::from([id(1), id(3)])),
        (id(3), BTreeSet::new()),
    ]);
    let result = components(&nodes, &edges);
    assert_eq!(result[&id(1)], id(1));
    assert_eq!(result[&id(2)], id(1));
    assert_eq!(result[&id(3)], id(3));
}

#[test]
fn dependency_order_places_callees_before_callers() {
    let id = |local| DefId::new(CrateId(0), LocalDefId(local));
    let nodes = [id(1), id(2), id(3)].into_iter().collect();
    let edges = HashMap::from([
        (id(1), BTreeSet::from([id(2)])),
        (id(2), BTreeSet::from([id(3)])),
        (id(3), BTreeSet::new()),
    ]);
    let membership = components(&nodes, &edges);
    assert_eq!(
        dependency_order(&membership, &edges),
        vec![id(3), id(2), id(1)]
    );
}

#[test]
fn dependency_order_keeps_recursive_components_monomorphic() {
    let id = |local| DefId::new(CrateId(0), LocalDefId(local));
    let nodes = [id(1), id(2), id(3)].into_iter().collect();
    let edges = HashMap::from([
        (id(1), BTreeSet::from([id(2)])),
        (id(2), BTreeSet::from([id(1)])),
        (id(3), BTreeSet::new()),
    ]);
    let membership = components(&nodes, &edges);
    assert_eq!(membership[&id(1)], membership[&id(2)]);
    assert_ne!(membership[&id(1)], membership[&id(3)]);
    assert_eq!(dependency_order(&membership, &edges), vec![id(1), id(3)]);
}
