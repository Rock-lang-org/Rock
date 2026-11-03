//! Injection of trait default methods into impls.

use super::default_method_calls::retarget_generated_default_trait_method_calls;
use super::default_method_types::{
    apply_generated_default_self_type, resolve_generated_default_types,
};
use super::generic_owner_remap::remap_function_generic_owner;
use super::trait_impl_substitution::substitute_trait_impl_types_in_function;
use crate::hir::*;
use crate::ids::{CrateId, DefId, IdGen, LocalDefId};
use crate::infer::InferenceEngine;
use crate::lower::ResolveError;
use crate::types::{GenericParamId, Type};
use std::collections::{BTreeSet, HashMap};

pub(crate) struct DefaultMethodInjectionContext<'a> {
    pub(super) trait_name: &'a str,
}

pub(crate) struct TraitDefaultMethodInjector<'a> {
    engine: &'a mut InferenceEngine,
    root_crate_id: CrateId,
    local_def_ids: &'a mut IdGen<LocalDefId>,
    current_def_ids: &'a mut BTreeSet<DefId>,
    source_map: &'a crate::source_map::SemanticSourceMap,
    diagnostics: &'a mut Vec<ResolveError>,
}

impl<'a> TraitDefaultMethodInjector<'a> {
    pub(crate) fn new(
        engine: &'a mut InferenceEngine,
        root_crate_id: CrateId,
        local_def_ids: &'a mut IdGen<LocalDefId>,
        current_def_ids: &'a mut BTreeSet<DefId>,
        source_map: &'a crate::source_map::SemanticSourceMap,
        diagnostics: &'a mut Vec<ResolveError>,
    ) -> Self {
        Self {
            engine,
            root_crate_id,
            local_def_ids,
            current_def_ids,
            source_map,
            diagnostics,
        }
    }

    pub(crate) fn prepare_missing_default_method(
        &mut self,
        imp: &mut HirImpl,
        method_name: &str,
        default_func: &HirFunction,
        self_type: &Type,
        context: DefaultMethodInjectionContext<'_>,
        prepare_func: impl FnOnce(&mut HirFunction),
    ) -> HirFunction {
        let mut func = default_func.clone();
        let default_method_id = func.id;
        let generated_method_id = DefId::new(self.root_crate_id, self.local_def_ids.fresh());
        self.current_def_ids.insert(generated_method_id);
        func.id = generated_method_id;
        remap_function_generic_owner(&mut func, default_method_id, generated_method_id);

        prepare_func(&mut func);
        retarget_generated_default_trait_method_calls(&mut func, imp);

        if let Err(err) = self.engine.unify(&func.body.ty, &func.ret_type) {
            let message = format!(
                "default method '{}.{}' return type mismatch: {}",
                context.trait_name,
                method_name,
                err.render(&self.engine)
            );
            if let Some(span) = self
                .source_map
                .definition_declaration_span(default_method_id)
                .cloned()
            {
                self.diagnostics.push(ResolveError::with_span_code(
                    message,
                    span,
                    crate::diagnostic::DiagnosticCode::Type,
                ));
            } else {
                self.diagnostics.push(ResolveError::non_source(message));
            }
        }
        resolve_generated_default_types(self.engine, &mut func);
        apply_generated_default_self_type(&mut func, self_type);

        func
    }
}

pub(super) fn prepare_default_method(
    func: &mut HirFunction,
    self_type: &Type,
    impl_trait_id: Option<DefId>,
    trait_arg_types: &[Type],
    impl_associated_types: &[HirAssociatedTypeDef],
    trait_generic_subst: &HashMap<GenericParamId, Type>,
) {
    // Only substitute the self-related TypeVars from the default body.
    if func.self_receiver.is_some() {
        if let Some(source_self_ty) = func.params.first().map(|param| param.ty.clone()) {
            crate::hir::substitute_typevars_in_function(func, &source_self_ty, self_type);
        }
    }
    substitute_trait_impl_types_in_function(
        func,
        self_type,
        impl_trait_id,
        trait_arg_types,
        impl_associated_types,
        trait_generic_subst,
    );
    apply_generated_default_self_type(func, self_type);
}
