use std::collections::BTreeSet;

use super::artifact_metadata::ProductLanguageItems;
use super::identity_recording::unambiguous_product_id;
use super::{ProductDefId, ProductIdRemap};
use crate::hir::{HirPhase, HirProgramFor};
use crate::ids::DefId;
use crate::infer::ResolvedHirProgram;
use crate::language_items::{
    DropLanguageItems, FnLanguageItems, FnMutLanguageItems, FnOnceLanguageItems, FoldLanguageItems,
    IndexLanguageItems, IndexMutLanguageItems, OwnedLanguageItems, RangeLanguageItems, SendLanguageItems,
    SizedLanguageItems, SyncLanguageItems, TryLanguageItems,
};

pub(super) fn is_current_crate_def(hir: &ResolvedHirProgram, id: DefId) -> bool {
    hir.current_def_ids.contains(&id)
}

pub(super) fn product_language_items_from_program<P: HirPhase>(
    program: &HirProgramFor<P>,
    current_def_ids: &BTreeSet<DefId>,
    id_remap: &ProductIdRemap,
) -> Result<ProductLanguageItems, String> {
    let language_items = &program.language_items;
    let sized = language_items
        .sized
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle("sized", &[items.trait_id], current_def_ids, id_remap)
                .map(|ids| ids.map(|ids| SizedLanguageItems { trait_id: ids[0] }))
        })
        .transpose()?
        .flatten();
    let drop = language_items
        .drop
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "drop",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| DropLanguageItems {
                    trait_id: ids[0],
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    let fold = language_items
        .fold
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "fold",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| FoldLanguageItems {
                    trait_id: ids[0],
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    let index = language_items
        .index
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "index",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| IndexLanguageItems {
                    trait_id: ids[0],
                    output_id: items.output_id,
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    let index_mut = language_items
        .index_mut
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "index_mut",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| IndexMutLanguageItems {
                    trait_id: ids[0],
                    output_id: items.output_id,
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    if language_items.index_mut.is_some() && index.is_some() != index_mut.is_some() {
        return Err(
            "language item index/index_mut provider pair mixes current-crate and dependency definitions"
                .to_string(),
        );
    }
    let fn_once = language_items
        .fn_once
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "fn_once",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| FnOnceLanguageItems {
                    trait_id: ids[0],
                    output_id: items.output_id,
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    let fn_mut = language_items
        .fn_mut
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "fn_mut",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| FnMutLanguageItems {
                    trait_id: ids[0],
                    output_id: items.output_id,
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    let fn_trait = language_items
        .fn_trait
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "fn",
                &[items.trait_id, items.method_id],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| FnLanguageItems {
                    trait_id: ids[0],
                    output_id: items.output_id,
                    method_id: ids[1],
                })
            })
        })
        .transpose()?
        .flatten();
    let send = language_items
        .send
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle("send", &[items.trait_id], current_def_ids, id_remap)
                .map(|ids| ids.map(|ids| SendLanguageItems { trait_id: ids[0] }))
        })
        .transpose()?
        .flatten();
    let owned = language_items
        .owned
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle("owned", &[items.trait_id], current_def_ids, id_remap)
                .map(|ids| ids.map(|ids| OwnedLanguageItems { trait_id: ids[0] }))
        })
        .transpose()?
        .flatten();
    let sync = language_items
        .sync
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle("sync", &[items.trait_id], current_def_ids, id_remap)
                .map(|ids| ids.map(|ids| SyncLanguageItems { trait_id: ids[0] }))
        })
        .transpose()?
        .flatten();
    let try_protocol = language_items
        .try_protocol
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle(
                "try",
                &[
                    items.try_trait_id,
                    items.branch_method_id,
                    items.from_residual_trait_id,
                    items.from_residual_method_id,
                    items.control_flow_enum_id,
                ],
                current_def_ids,
                id_remap,
            )
            .map(|ids| {
                ids.map(|ids| TryLanguageItems {
                    try_trait_id: ids[0],
                    output_id: items.output_id,
                    residual_id: items.residual_id,
                    branch_method_id: ids[1],
                    from_residual_trait_id: ids[2],
                    from_residual_method_id: ids[3],
                    control_flow_enum_id: ids[4],
                    break_variant_id: items.break_variant_id,
                    continue_variant_id: items.continue_variant_id,
                })
            })
        })
        .transpose()?
        .flatten();
    let range = language_items
        .range
        .as_ref()
        .map(|items| {
            map_product_language_item_bundle("range", &[items.enum_id], current_def_ids, id_remap)
                .map(|ids| {
                    ids.map(|ids| RangeLanguageItems {
                        enum_id: ids[0],
                        full_variant_id: items.full_variant_id,
                        from_variant_id: items.from_variant_id,
                        to_variant_id: items.to_variant_id,
                        to_inclusive_variant_id: items.to_inclusive_variant_id,
                        exclusive_variant_id: items.exclusive_variant_id,
                        inclusive_variant_id: items.inclusive_variant_id,
                    })
                })
        })
        .transpose()?
        .flatten();

    Ok(ProductLanguageItems {
        fold,
        sized,
        drop,
        index,
        index_mut,
        fn_once,
        fn_mut,
        fn_trait,
        send,
        owned,
        sync,
        try_protocol,
        range,
    })
}

fn map_product_language_item_bundle(
    bundle: &str,
    ids: &[DefId],
    current_def_ids: &BTreeSet<DefId>,
    id_remap: &ProductIdRemap,
) -> Result<Option<Vec<ProductDefId>>, String> {
    let current = ids
        .iter()
        .map(|id| current_def_ids.contains(id))
        .collect::<Vec<_>>();
    if current.iter().all(|current| *current) {
        return ids
            .iter()
            .map(|id| {
                unambiguous_product_id(ProductDefId::from(*id), id_remap).ok_or_else(|| {
                    format!(
                        "language item {bundle} bundle has no unambiguous product ID for {id:?}"
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some);
    }
    if current.iter().all(|current| !*current) {
        return Ok(None);
    }

    Err(format!(
        "language item {bundle} bundle mixes current-crate and dependency definitions"
    ))
}
