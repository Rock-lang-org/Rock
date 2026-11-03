//! Trait conformance service types.

use crate::collect::resolver::ResolverTables;
use crate::ids::{AssocTypeId, CrateId, DefId, IdGen, LocalDefId};
use crate::infer::InferenceEngine;
use crate::lower::items::LowerItems;
use crate::lower::ResolveError;
use std::collections::{BTreeSet, HashMap};

#[derive(Default)]
pub(crate) struct TraitConformanceOutput {
    pub(crate) diagnostics: Vec<ResolveError>,
    pub(crate) effective_trait_methods: HashMap<(DefId, DefId), DefId>,
}

pub(super) fn collect_effective_trait_methods(
    items: &LowerItems,
    root_crate_id: CrateId,
    output: &mut TraitConformanceOutput,
) {
    for imp in items
        .impl_defs_in_order()
        .map(|(_, imp)| imp)
        .filter(|imp| imp.id.crate_id == root_crate_id)
    {
        let Some(trait_id) = imp.trait_id else {
            continue;
        };
        let Some(trait_def) = items.trait_def(trait_id) else {
            continue;
        };

        for (member_name, member_id) in trait_def
            .methods
            .iter()
            .map(|(name, method)| (name, method.id))
            .chain(
                trait_def
                    .signatures
                    .iter()
                    .filter(|(name, _)| !trait_def.methods.contains_key(*name))
                    .map(|(name, signature)| (name, signature.id)),
            )
        {
            let Some(method_id) = imp.methods.get(member_name).map(|method| method.id) else {
                continue;
            };
            let key = (imp.id, member_id);
            if let Some(previous) = output.effective_trait_methods.insert(key, method_id) {
                if previous != method_id {
                    output.diagnostics.push(ResolveError::non_source(format!(
                        "trait implementation {:?} has conflicting effective bodies {:?} and {:?} for member {:?}",
                        imp.id, previous, method_id, member_id
                    )));
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct IndexProtocolIds {
    pub(super) index_trait: DefId,
    pub(super) index_output: AssocTypeId,
    pub(super) index_mut_trait: DefId,
    pub(super) index_mut_output: AssocTypeId,
}

pub(crate) struct TraitConformanceContext<'a> {
    pub(super) items: &'a mut LowerItems,
    pub(super) engine: &'a mut InferenceEngine,
    pub(super) resolver: &'a ResolverTables,
    pub(super) dependency_resolvers: &'a HashMap<String, ResolverTables>,
    pub(super) root_crate_id: CrateId,
    pub(super) local_def_ids: &'a mut IdGen<LocalDefId>,
    pub(super) current_def_ids: &'a mut BTreeSet<DefId>,
    pub(super) source_map: &'a crate::source_map::SemanticSourceMap,
    pub(super) index_protocol_ids: Option<IndexProtocolIds>,
}

pub(crate) struct TraitConformanceService<'a> {
    pub(super) items: &'a mut LowerItems,
    pub(super) engine: &'a mut InferenceEngine,
    pub(super) resolver: &'a ResolverTables,
    pub(super) dependency_resolvers: &'a HashMap<String, ResolverTables>,
    pub(super) root_crate_id: CrateId,
    pub(super) local_def_ids: &'a mut IdGen<LocalDefId>,
    pub(super) current_def_ids: &'a mut BTreeSet<DefId>,
    pub(super) source_map: &'a crate::source_map::SemanticSourceMap,
    pub(super) index_protocol_ids: Option<IndexProtocolIds>,
    pub(super) output: TraitConformanceOutput,
}

pub(crate) struct TraitConformancePhase;

impl<'a> TraitConformanceService<'a> {
    pub(crate) fn new(context: TraitConformanceContext<'a>) -> Self {
        Self {
            items: context.items,
            engine: context.engine,
            resolver: context.resolver,
            dependency_resolvers: context.dependency_resolvers,
            root_crate_id: context.root_crate_id,
            local_def_ids: context.local_def_ids,
            current_def_ids: context.current_def_ids,
            source_map: context.source_map,
            index_protocol_ids: context.index_protocol_ids,
            output: TraitConformanceOutput::default(),
        }
    }

    pub(crate) fn finish(self) -> TraitConformanceOutput {
        self.output
    }
}
