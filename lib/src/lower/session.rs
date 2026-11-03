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
mod tests;
