use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::artifact_metadata::{ProductIdentityTable, ProductLinkData};
use super::{ProductCrateId, ProductDefId, ProductIdRemap, ProductLocalDefId};
use crate::hir::{AcceptedHir, AcceptedHirFunction, HirImplFor, HirPhase, HirProgramFor};
use crate::ids::{CrateId, DefId, LocalDefId};

pub(super) struct PreferredTraitDefaultMethods {
    pub(super) by_id: HashMap<DefId, AcceptedHirFunction>,
}

impl PreferredTraitDefaultMethods {
    pub(super) fn new(program: &HirProgramFor<AcceptedHir>) -> Self {
        let mut preferred = Self {
            by_id: HashMap::new(),
        };

        for (_, _, trait_def) in program.traits_by_id() {
            for method in trait_def.methods.values() {
                insert_preferred_default(&mut preferred.by_id, method.id, method.clone());
            }
        }

        preferred
    }

    pub(super) fn get(
        &self,
        id: DefId,
        _trait_name: &str,
        _method_name: &str,
    ) -> AcceptedHirFunction {
        self.by_id
            .get(&id)
            .cloned()
            .expect("trait default method should have a preferred body")
    }
}

fn insert_preferred_default<K>(
    preferred: &mut HashMap<K, AcceptedHirFunction>,
    key: K,
    method: AcceptedHirFunction,
) where
    K: Eq + std::hash::Hash,
{
    preferred
        .entry(key)
        .and_modify(|current| {
            if current.body.stmts.is_empty() && !method.body.stmts.is_empty() {
                *current = method.clone();
            }
        })
        .or_insert(method);
}

pub(super) fn record_name(identity_table: &mut ProductIdentityTable, id: ProductDefId, name: &str) {
    record_display_name(identity_table, id, name);
    record_export_name(identity_table, id, name);
}

pub(super) fn record_display_name(
    identity_table: &mut ProductIdentityTable,
    id: ProductDefId,
    name: &str,
) {
    identity_table.display_names.insert(id, name.to_string());
}

pub(super) fn record_export_name(
    identity_table: &mut ProductIdentityTable,
    id: ProductDefId,
    name: &str,
) {
    if identity_table.ambiguous_export_names.contains(name) {
        return;
    }

    match identity_table.export_names.get(name).copied() {
        Some(existing) if existing != id => {
            identity_table.export_names.remove(name);
            identity_table
                .ambiguous_export_names
                .insert(name.to_string());
        }
        Some(_) => {}
        None => {
            identity_table.export_names.insert(name.to_string(), id);
        }
    }
}

pub(super) fn record_product_aliases(
    out: &mut BTreeMap<String, ProductDefId>,
    aliases: &HashMap<String, DefId>,
    id_remap: &ProductIdRemap,
) {
    for (alias, id) in aliases {
        if let Some(product_id) = product_alias_id(ProductDefId::from(*id), id_remap) {
            out.insert(alias.clone(), product_id);
        }
    }
}

fn product_alias_id(id: ProductDefId, id_remap: &ProductIdRemap) -> Option<ProductDefId> {
    match id_remap.get(&id) {
        Some(ids) if ids.len() == 1 => ids.iter().next().copied(),
        _ => None,
    }
}

pub(super) fn record_id_remap(
    id_remap: &mut ProductIdRemap,
    requested: ProductDefId,
    actual: ProductDefId,
) {
    id_remap.entry(requested).or_default().insert(actual);
}

pub(super) fn remap_link_data(link: ProductLinkData, id_remap: &ProductIdRemap) -> ProductLinkData {
    let records = link
        .records
        .into_iter()
        // Ambiguous original IDs have no safe product owner, so drop them.
        .filter_map(|(id, record)| unambiguous_product_id(id, id_remap).map(|id| (id, record)))
        .collect();

    ProductLinkData {
        object_path: link.object_path,
        records,
    }
}

pub(super) fn unambiguous_product_id(
    id: ProductDefId,
    id_remap: &ProductIdRemap,
) -> Option<ProductDefId> {
    let ids = id_remap.get(&id)?;
    if ids.len() == 1 {
        ids.iter().next().copied()
    } else {
        None
    }
}

pub(super) fn reserve_product_id(
    requested: ProductDefId,
    used_ids: &mut BTreeSet<ProductDefId>,
    fallback_crate: ProductCrateId,
    next_fallback_local: &mut u32,
    force_fallback: bool,
) -> ProductDefId {
    if force_fallback || requested.crate_id == ProductCrateId(u32::MAX) {
        used_ids.insert(requested);
        return fresh_product_id(fallback_crate, next_fallback_local, used_ids);
    }

    if !force_fallback && used_ids.insert(requested) {
        requested
    } else {
        fresh_product_id(fallback_crate, next_fallback_local, used_ids)
    }
}

fn fresh_product_id(
    fallback_crate: ProductCrateId,
    next_fallback_local: &mut u32,
    used_ids: &mut BTreeSet<ProductDefId>,
) -> ProductDefId {
    loop {
        let id = ProductDefId {
            crate_id: fallback_crate,
            local_id: ProductLocalDefId(*next_fallback_local),
        };
        *next_fallback_local = next_fallback_local
            .checked_add(1)
            .expect("product fallback ID generator exhausted u32 ID space");

        if used_ids.insert(id) {
            return id;
        }
    }
}

pub(super) fn impl_display_name<P: HirPhase>(imp: &HirImplFor<P>) -> String {
    match &imp.trait_name {
        Some(trait_name) => format!("{} as {}", imp.type_name, trait_name),
        None => imp.type_name.clone(),
    }
}

pub(super) fn qualify_metadata_name(crate_name: &str, name: &str) -> String {
    if name.contains("::") {
        name.to_string()
    } else {
        format!("{}::{}", crate_name, name)
    }
}

pub(super) fn product_def_id_to_def_id(id: ProductDefId) -> DefId {
    DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0))
}
