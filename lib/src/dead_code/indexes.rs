//! Filter derived indexes without rebuilding aliases or cloning member names.

use std::collections::HashMap;

use crate::hir::HirDefinitionIndexes;
use crate::ids::DefId;

pub(super) fn retain(indexes: &mut HirDefinitionIndexes, keep: &impl Fn(&DefId) -> bool) {
    indexes.functions_by_id.retain(|id, _| keep(id));
    indexes.structs_by_id.retain(|id, _| keep(id));
    indexes.enums_by_id.retain(|id, _| keep(id));
    indexes.traits_by_id.retain(|id, _| keep(id));
    indexes.type_aliases_by_id.retain(|id, _| keep(id));
    retain_positions(&mut indexes.impls_by_id, keep);
    retain_positions(&mut indexes.externs_by_id, keep);
    indexes.impl_owner_ids_by_id.retain(|id, _| keep(id));
    indexes.methods_by_id.retain(|id, _| keep(id));
    indexes.fields_by_id.retain(|(owner, _), _| keep(owner));
    indexes.variants_by_id.retain(|(owner, _), _| keep(owner));
    indexes
        .associated_type_decls_by_id
        .retain(|(owner, _), _| keep(owner));
    indexes
        .associated_type_defs_by_id
        .retain(|(owner, _), _| keep(owner));
    indexes
        .effective_trait_methods
        .retain(|(owner, _), _| keep(owner));
}

/// These indexes contain dense positions in the original declaration order.
/// Compact in that same order without sorting or rescanning each declaration.
fn retain_positions(index: &mut HashMap<DefId, usize>, keep: &impl Fn(&DefId) -> bool) {
    let mut ordered = vec![None; index.len()];
    for (id, position) in index.iter() {
        if keep(id) {
            ordered[*position] = Some(*id);
        }
    }
    index.clear();
    for (position, id) in ordered.into_iter().flatten().enumerate() {
        index.insert(id, position);
    }
}
