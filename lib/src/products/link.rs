use super::{CompilerProducts, ProductDefId, ProductIdRemap, ProductLinkRecord};

pub(crate) fn attach_product_object_path(
    products: &mut Option<CompilerProducts>,
    object_path: &std::path::Path,
) {
    if let Some(products) = products.as_mut() {
        products.link.object_path = Some(object_path.to_path_buf());
    }
}

pub(crate) fn attach_product_link_records(
    products: &mut Option<CompilerProducts>,
    candidates: &[crate::mir::MirArtifactExport],
    symbol_overrides: &std::collections::HashMap<crate::ids::DefId, String>,
    id_remap: Option<&ProductIdRemap>,
) -> Result<(), String> {
    let Some(products) = products.as_mut() else {
        return Ok(());
    };

    for candidate in candidates {
        if candidate.provided_by_object || !candidate.substitution_empty || !candidate.has_body {
            continue;
        }
        if candidate.is_specialization && !candidate.is_drop_glue {
            continue;
        }

        let def_id = candidate
            .origin_def_id
            .ok_or_else(|| "product link export has no origin definition ID".to_string())?;
        let id_remap = id_remap.ok_or_else(|| {
            "product ID remap is required to attach product link records".to_string()
        })?;
        let product_id = product_link_id_for_candidate(products, id_remap, def_id)?;
        let backend_symbol = symbol_overrides
            .get(&def_id)
            .cloned()
            .unwrap_or_else(|| candidate.backend_symbol.clone());
        products
            .link
            .records
            .insert(product_id, ProductLinkRecord { backend_symbol });
    }

    Ok(())
}

fn product_link_id_for_candidate(
    products: &CompilerProducts,
    id_remap: &ProductIdRemap,
    def_id: crate::ids::DefId,
) -> Result<ProductDefId, String> {
    let callable_ids = id_remap
        .get(&ProductDefId::from(def_id))
        .into_iter()
        .flatten()
        .copied()
        .filter(|id| product_id_is_callable(products, *id))
        .collect::<Vec<_>>();

    match callable_ids.as_slice() {
        [id] => Ok(*id),
        [] => Err(format!(
            "no explicit product callable identity for MIR export origin {:?}",
            def_id
        )),
        _ => Err(format!(
            "ambiguous product callable identities for MIR export origin {:?}",
            def_id
        )),
    }
}

fn product_id_is_callable(products: &CompilerProducts, id: ProductDefId) -> bool {
    products.interface.functions.contains_key(&id)
        || products.interface.traits.values().any(|trait_def| {
            trait_def
                .methods
                .values()
                .any(|method| ProductDefId::from(method.id) == id)
        })
        || products.interface.impls.values().any(|imp| {
            imp.methods
                .values()
                .any(|method| ProductDefId::from(method.id) == id)
        })
}

#[cfg(test)]
mod tests;
