use std::collections::{BTreeSet, HashMap, HashSet};

use crate::ids::DefId;

pub(crate) fn components(
    nodes: &BTreeSet<DefId>,
    edges: &HashMap<DefId, BTreeSet<DefId>>,
) -> HashMap<DefId, DefId> {
    fn visit(
        node: DefId,
        edges: &HashMap<DefId, BTreeSet<DefId>>,
        visited: &mut HashSet<DefId>,
        order: &mut Vec<DefId>,
    ) {
        if !visited.insert(node) {
            return;
        }
        if let Some(neighbors) = edges.get(&node) {
            for neighbor in neighbors {
                visit(*neighbor, edges, visited, order);
            }
        }
        order.push(node);
    }

    fn visit_reverse(
        node: DefId,
        reverse: &HashMap<DefId, BTreeSet<DefId>>,
        assigned: &HashSet<DefId>,
        component: &mut BTreeSet<DefId>,
    ) {
        if assigned.contains(&node) || !component.insert(node) {
            return;
        }
        if let Some(neighbors) = reverse.get(&node) {
            for neighbor in neighbors {
                visit_reverse(*neighbor, reverse, assigned, component);
            }
        }
    }

    let mut order = Vec::with_capacity(nodes.len());
    let mut visited = HashSet::new();
    for node in nodes {
        visit(*node, edges, &mut visited, &mut order);
    }

    let mut reverse = HashMap::<DefId, BTreeSet<DefId>>::new();
    for node in nodes {
        reverse.entry(*node).or_default();
    }
    for (source, targets) in edges {
        for target in targets {
            reverse.entry(*target).or_default().insert(*source);
        }
    }

    let mut result = HashMap::new();
    let mut assigned = HashSet::new();
    for node in order.into_iter().rev() {
        if assigned.contains(&node) {
            continue;
        }
        let mut component = BTreeSet::new();
        visit_reverse(node, &reverse, &assigned, &mut component);
        assigned.extend(component.iter().copied());
        let representative = *component.first().expect("SCC traversal visits its root");
        for member in component {
            result.insert(member, representative);
        }
    }

    result
}

/// Return SCC representatives in dependency-first order.  A call edge points
/// from the caller to its dependency; preference applies only to ready SCCs
/// and their prerequisite closure, never in place of a dependency edge.
pub(crate) fn dependency_order(
    membership: &HashMap<DefId, DefId>,
    edges: &HashMap<DefId, BTreeSet<DefId>>,
    preferred_nodes: &BTreeSet<DefId>,
) -> Vec<DefId> {
    let mut dependencies = HashMap::<DefId, BTreeSet<DefId>>::new();
    for component in membership.values().copied() {
        dependencies.entry(component).or_default();
    }
    for (source, targets) in edges {
        let Some(&source_component) = membership.get(source) else {
            continue;
        };
        for target in targets {
            let Some(&target_component) = membership.get(target) else {
                continue;
            };
            if source_component != target_component {
                dependencies
                    .entry(source_component)
                    .or_default()
                    .insert(target_component);
            }
        }
    }

    // Prefer method bodies and the canonical dependencies needed to make them
    // ready. A total DFS ordinal gives unrelated functions accidental priority
    // over nominal methods whose dot references have no static call edge.
    let mut preferred = HashSet::new();
    let mut pending = preferred_nodes.iter().filter_map(|node| membership.get(node).copied()).collect::<Vec<_>>();
    while let Some(component) = pending.pop() {
        if preferred.insert(component) {
            pending.extend(dependencies[&component].iter().copied());
        }
    }
    let mut dependents = HashMap::<DefId, BTreeSet<DefId>>::new();
    let mut ready = BTreeSet::new();
    for (&component, deps) in &dependencies {
        if deps.is_empty() {
            ready.insert((!preferred.contains(&component), component));
        }
        for dependency in deps {
            dependents.entry(*dependency).or_default().insert(component);
        }
    }
    let mut output = Vec::with_capacity(dependencies.len());
    while let Some((_, component)) = ready.pop_first() {
        output.push(component);
        if let Some(consumers) = dependents.get(&component) {
            for consumer in consumers {
                let remaining = dependencies.get_mut(consumer).expect("indexed SCC consumer");
                remaining.remove(&component);
                if remaining.is_empty() {
                    ready.insert((!preferred.contains(consumer), *consumer));
                }
            }
        }
    }
    assert_eq!(output.len(), dependencies.len(), "SCC condensation is acyclic");
    output
}

#[cfg(test)]
mod tests {
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
            dependency_order(&membership, &edges, &BTreeSet::new()),
            vec![id(3), id(2), id(1)]
        );
    }

    #[test]
    fn dependency_order_prioritizes_methods_without_bypassing_function_dependencies() {
        let id = |local| DefId::new(CrateId(0), LocalDefId(local));
        let nodes = [id(1), id(2), id(3), id(4), id(5)].into_iter().collect();
        let edges = HashMap::from([
            (id(3), BTreeSet::from([id(4)])),
            (id(4), BTreeSet::from([id(2)])),
        ]);
        let membership = components(&nodes, &edges);
        assert_eq!(dependency_order(&membership, &edges, &BTreeSet::from([id(3), id(4), id(5)])),
            vec![id(2), id(4), id(3), id(5), id(1)]);
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
        assert_eq!(dependency_order(&membership, &edges, &BTreeSet::new()), vec![id(1), id(3)]);
    }
}
