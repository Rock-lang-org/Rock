use std::path::Path;

use crate::crate_system::CrateContext;
use crate::lower::Lowerer;

pub(crate) struct LoweringSessionServices<'a> {
    ctx: &'a CrateContext,
    current_crate_name: Option<&'a str>,
    root_filepath: Option<&'a Path>,
}

impl<'a> LoweringSessionServices<'a> {
    pub(crate) fn new(
        ctx: &'a CrateContext,
        current_crate_name: Option<&'a str>,
        root_filepath: Option<&'a Path>,
    ) -> Self {
        Self {
            ctx,
            current_crate_name,
            root_filepath,
        }
    }

    pub(crate) fn prepare_lowerer(&self, lowerer: &mut Lowerer) {
        self.register_crate_resolvers(lowerer);
        self.register_dependency_authority(lowerer);
        lowerer
            .modules
            .configure_current_crate(self.current_crate_name, self.root_filepath);
        lowerer
            .modules
            .associate_current_crate_module_ids(&lowerer.item_index);
        self.capture_loaded_prelude_exports(lowerer);
        self.apply_loaded_prelude(lowerer);
    }

    fn register_dependency_authority(&self, lowerer: &mut Lowerer) {
        for dependency in self.ctx.extern_crates() {
            let interface = dependency.metadata().interface();
            lowerer.imported_effective_trait_methods.extend(
                interface
                    .effective_trait_methods
                    .iter()
                    .map(|(&key, &method_id)| (key, method_id)),
            );
        }
    }

    pub(crate) fn lower_dependency_trait_bodies(&self, lowerer: &mut Lowerer) {
        lowerer.lower_crate_trait_bodies(self.ctx);
    }

    pub(crate) fn lower_dependency_module_bodies(&self, lowerer: &mut Lowerer) {
        lowerer.lower_crate_module_bodies(self.ctx);
    }

    fn register_crate_resolvers(&self, lowerer: &mut Lowerer) {
        for message in self.ctx.dependency_errors_for_phase("lowering") {
            lowerer.diagnostics.push_toolchain_once(message);
        }

        for dep in self.ctx.extern_crates() {
            let resolver = dep.metadata().resolver().clone();
            lowerer.resolver.merge_global_inputs(&resolver);
            lowerer
                .dependency_resolvers
                .insert(dep.name().to_string(), resolver);
        }
    }

    fn capture_loaded_prelude_exports(&self, lowerer: &mut Lowerer) {
        for dep in self.ctx.extern_crates() {
            lowerer
                .prelude
                .capture_loaded_prelude_exports(dep.name(), dep.prelude_exports());
        }
    }

    fn apply_loaded_prelude(&self, lowerer: &mut Lowerer) {
        if !lowerer.prelude.is_enabled() || !self.ctx.has_extern_crate("std") {
            return;
        }

        let errors = lowerer.prelude.inject_loaded_prelude(
            &mut lowerer.scope,
            &mut lowerer.items,
            &mut lowerer.resolver,
        );
        for error in errors {
            lowerer.diagnostics.push_toolchain_once(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::crate_artifact::{ArtifactCrateInterface, ArtifactExport};
    use crate::crate_system::{
        ExternCrateBodies, ExternCrateLink, ExternCrateMetadata, ExternCrateRecord,
    };
    use crate::hir::{HirBlock, HirFunction};
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::lower::Lowerer;
    use crate::types::Type;

    use super::*;

    fn test_function(id: DefId, name: &str) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: Default::default(),
            params: Vec::new(),
            ret_type: Type::I64,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    #[test]
    fn session_services_seed_resolvers_and_prelude_before_body_lowering() {
        let export_id = DefId::new(CrateId(7), LocalDefId(1));
        let mut interface = ArtifactCrateInterface::default();
        let qualified_name = "std::prelude::answer".to_string();
        let function = test_function(export_id, "answer");
        interface.insert_function(qualified_name.clone(), function.clone());

        let mut resolver = crate::collect::resolver::ResolverTables::default();
        resolver
            .item_paths
            .insert("std::prelude::answer".to_string(), export_id);
        resolver
            .item_names_by_id
            .insert(export_id, "std::prelude::answer".to_string());

        let prelude_exports = BTreeMap::from([(
            "answer".to_string(),
            ArtifactExport {
                source: qualified_name.clone(),
                id: export_id,
            },
        )]);
        let mut ctx = CrateContext::new();
        ctx.add_extern_crate(ExternCrateRecord::new(
            CrateId(7),
            "std".to_string(),
            ExternCrateMetadata::new(interface, resolver, prelude_exports),
            ExternCrateBodies::default(),
            ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .unwrap();

        let mut lowerer = Lowerer::with_options(true);
        lowerer.items.insert_function(function);
        LoweringSessionServices::new(&ctx, Some("demo"), None).prepare_lowerer(&mut lowerer);

        assert!(lowerer.dependency_resolvers.contains_key("std"));
        assert_eq!(
            lowerer.resolver.import_aliases.get("answer"),
            Some(&export_id)
        );
        assert!(lowerer.scope.binding_is_alias("answer"));
    }

    #[test]
    fn session_services_preserve_non_std_prelude_provider_without_injection() {
        let export_id = DefId::new(CrateId(9), LocalDefId(1));
        let mut interface = ArtifactCrateInterface::default();
        let qualified_name = "tools::prelude::answer".to_string();
        let function = test_function(export_id, "answer");
        interface.insert_function(qualified_name.clone(), function.clone());

        let mut resolver = crate::collect::resolver::ResolverTables::default();
        resolver
            .item_paths
            .insert("tools::prelude::answer".to_string(), export_id);
        resolver
            .item_names_by_id
            .insert(export_id, "tools::prelude::answer".to_string());

        let prelude_exports = BTreeMap::from([(
            "answer".to_string(),
            ArtifactExport {
                source: qualified_name.clone(),
                id: export_id,
            },
        )]);
        let mut ctx = CrateContext::new();
        ctx.add_extern_crate(ExternCrateRecord::new(
            CrateId(9),
            "tools".to_string(),
            ExternCrateMetadata::new(interface, resolver, prelude_exports),
            ExternCrateBodies::default(),
            ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .unwrap();

        let mut lowerer = Lowerer::with_options(true);
        lowerer.items.insert_function(function);
        LoweringSessionServices::new(&ctx, Some("demo"), None).prepare_lowerer(&mut lowerer);

        assert!(lowerer.dependency_resolvers.contains_key("tools"));
        assert!(lowerer.resolver.import_aliases.get("answer").is_none());
        assert!(!lowerer.scope.binding_is_alias("answer"));
    }
}
