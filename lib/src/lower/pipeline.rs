use crate::ast;
use crate::collect::item_index::ModuleKind;
use crate::collect::Declarations;
use crate::crate_system::CrateContext;
use crate::infer::PartialHir;
use crate::lower::body_lowerer::BodyLowerer;
use crate::lower::module_context::ModuleLoweringContext;
use crate::lower::session::LoweringSessionServices;
use crate::lower::traits::conformance::TraitConformancePhase;
use crate::lower::{Lowerer, ResolveError};
use crate::traits::coherence::CoherencePhase;

pub(crate) struct LoweringPipeline<'a> {
    program: &'a ast::Program,
    crate_ctx: &'a CrateContext,
    current_crate_name: Option<&'a str>,
    overflow_checks: bool,
}

impl<'a> LoweringPipeline<'a> {
    pub(crate) fn new(
        program: &'a ast::Program,
        crate_ctx: &'a CrateContext,
        current_crate_name: Option<&'a str>,
    ) -> Self {
        Self {
            program,
            crate_ctx,
            current_crate_name,
            overflow_checks: true,
        }
    }

    pub(crate) fn with_overflow_checks(mut self, enabled: bool) -> Self {
        self.overflow_checks = enabled;
        self
    }

    pub(crate) fn lower_from_declarations(
        self,
        decls: Declarations,
    ) -> Result<PartialHir, Vec<ResolveError>> {
        let mut lowerer = Lowerer::from_declarations(decls)?;
        lowerer.overflow_checks = self.overflow_checks;
        let session = LoweringSessionServices::new(
            self.crate_ctx,
            self.current_crate_name,
            self.program.module.filepath.as_deref(),
        );
        session.prepare_lowerer(&mut lowerer);
        lowerer.refresh_type_display_context();
        self.prepare_traits(&session, &mut lowerer);
        self.lower_dependency_bodies(&session, &mut lowerer);
        self.lower_current_crate_bodies(&mut lowerer);
        self.finish(lowerer)
    }

    fn prepare_traits(&self, session: &LoweringSessionServices<'_>, lowerer: &mut Lowerer) {
        TraitConformancePhase::prepare_explicit_method_selection(lowerer);
        session.lower_dependency_trait_bodies(lowerer);
        let Some(root_module_id) = lowerer
            .item_index
            .modules()
            .iter()
            .find(|module| module.kind == ModuleKind::Root)
            .map(|module| module.module_id)
        else {
            lowerer.diagnostics.push_toolchain(
                "missing indexed root module while lowering trait default bodies".to_string(),
            );
            return;
        };
        ModuleLoweringContext::with_qualified_module_context(
            lowerer,
            &self.program.module,
            None,
            |lowerer| lowerer.lower_trait_default_bodies(&self.program.module, root_module_id),
        );
        lowerer.lower_loaded_module_trait_defaults();
        TraitConformancePhase::run(lowerer);
        CoherencePhase::run(lowerer);
    }

    fn lower_dependency_bodies(
        &self,
        session: &LoweringSessionServices<'_>,
        lowerer: &mut Lowerer,
    ) {
        session.lower_dependency_module_bodies(lowerer);
    }

    fn lower_current_crate_bodies(&self, lowerer: &mut Lowerer) {
        BodyLowerer::new(lowerer).lower_root_and_loaded_modules(&self.program.module);
    }

    fn finish(self, lowerer: Lowerer) -> Result<PartialHir, Vec<ResolveError>> {
        if lowerer.has_errors() {
            return Err(lowerer.diagnostics.into_inner().into_errors());
        }
        let import_aliases = lowerer
            .resolver
            .import_aliases
            .iter()
            .filter_map(|(alias, id)| {
                lowerer
                    .canonical_name_for_def_id(*id)
                    .map(|canonical| (alias.clone(), canonical.to_string()))
            })
            .collect();
        let loaded_module_paths = lowerer.modules.into_loaded_module_paths();
        let items = lowerer.items.into_inner().into_validated_declarations()?;
        let (functions, _, structs, enums, traits, impls, externs, type_aliases) =
            items.into_id_maps();

        Ok(PartialHir {
            functions,
            structs,
            enums,
            traits,
            impls,
            externs,
            type_aliases,
            engine: lowerer.engine.into_inner(),
            function_type_vars: lowerer.function_type_vars,
            import_aliases,
            loaded_module_paths,
            constraint_store: lowerer.constraint_store.into_inner(),
            resolver: lowerer.resolver.into_inner(),
            current_def_ids: lowerer.current_def_ids,
            root_crate_id: lowerer.root_crate_id,
            local_def_ids: lowerer.local_def_ids,
            language_items: lowerer.language_items,
            imported_effective_trait_methods: lowerer.imported_effective_trait_methods,
            inference_sccs: lowerer.inference_sccs.clone(),
            inference_scc_order: lowerer.inference_scc_order,
            source_map: lowerer.source_map,
        })
    }
}

#[cfg(test)]
mod tests;
