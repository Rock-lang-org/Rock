mod crate_declarations;
mod declarations;
mod imports;
mod member_ids;
mod nominal_fields;
mod ordering;
mod path_qualification;
mod qualified_declarations;
mod trait_identities;

use crate::ast;
use crate::collect::context::{collect_exports, CollectContext, LocalCollection};
use crate::collect::CollectedIdEnvironment;
use crate::ids::{DefId, ModuleId};
use nominal_fields::PendingNominalFields;
use std::collections::BTreeMap;

pub(crate) struct LocalCollector {
    context: CollectContext,
    id_environment: CollectedIdEnvironment,
    pending_nominal_fields: BTreeMap<DefId, PendingNominalFields>,
    pending_imports: Vec<imports::PendingImport>,
}

impl LocalCollector {
    pub(crate) fn new_with_id_environment(
        context: CollectContext,
        id_environment: CollectedIdEnvironment,
    ) -> Self {
        Self {
            context,
            id_environment,
            pending_nominal_fields: BTreeMap::new(),
            pending_imports: Vec::new(),
        }
    }

    pub fn collect_local_declarations(&mut self, module: &ast::Module, root_module_id: ModuleId) {
        self.predeclare_nominal_identities(module, root_module_id, None);
        if let Some(crate_name) = self.context.current_crate_name.clone() {
            let root_exports = collect_exports(module);
            self.predeclare_source_backed_trait_identities(
                module,
                root_module_id,
                &crate_name,
                &root_exports,
                crate_name == "std",
            );
        }

        self.collect_declarations(module, root_module_id);
    }

    pub(crate) fn finish(
        mut self,
        item_index: &crate::collect::item_index::ItemIndex,
        root_module_id: crate::ids::ModuleId,
        current_crate_name: Option<&str>,
    ) -> LocalCollection {
        self.complete_nominal_fields();
        self.report_unresolved_imports();
        self.context
            .into_local_collection(item_index, root_module_id, current_crate_name)
    }
}

#[cfg(test)]
mod tests;
