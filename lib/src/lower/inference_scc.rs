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
/// from the caller to its dependency, so dependencies are visited before the
/// component that consumes them.
pub(crate) fn dependency_order(
    membership: &HashMap<DefId, DefId>,
    edges: &HashMap<DefId, BTreeSet<DefId>>,
) -> Vec<DefId> {
    fn visit(
        component: DefId,
        dependencies: &HashMap<DefId, BTreeSet<DefId>>,
        visiting: &mut HashSet<DefId>,
        visited: &mut HashSet<DefId>,
        output: &mut Vec<DefId>,
    ) {
        if visited.contains(&component) || !visiting.insert(component) {
            return;
        }
        if let Some(deps) = dependencies.get(&component) {
            for dependency in deps {
                visit(*dependency, dependencies, visiting, visited, output);
            }
        }
        visiting.remove(&component);
        visited.insert(component);
        output.push(component);
    }

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

    let mut components = dependencies.keys().copied().collect::<Vec<_>>();
    components.sort();
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut output = Vec::with_capacity(components.len());
    for component in components {
        visit(
            component,
            &dependencies,
            &mut visiting,
            &mut visited,
            &mut output,
        );
    }
    output
}

#[cfg(test)]
mod tests;
