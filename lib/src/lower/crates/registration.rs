//! Crate registration methods

#![allow(dead_code)]

use crate::crate_system::{CrateContext, ExternCrateRef};
use crate::lower::Lowerer;
use crate::types::Type;

pub(crate) struct LowerCrateRegistration<'a> {
    ctx: &'a CrateContext,
}

impl<'a> LowerCrateRegistration<'a> {
    pub(crate) fn new(ctx: &'a CrateContext) -> Self {
        Self { ctx }
    }

    pub(crate) fn register_crate_resolvers(&self, lowerer: &mut Lowerer) {
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

    /// Register functions from external crates in the scope
    ///
    /// Functions are registered with qualified names only
    /// (e.g., `std::sqrt`, `mycrate::function`).
    /// Unqualified access must come from normal imports or std prelude injection.
    pub(crate) fn register_crate_functions(&self, lowerer: &mut Lowerer) {
        self.register_crate_resolvers(lowerer);

        for dep in self.ctx.extern_crates() {
            self.register_extern_crate(lowerer, dep);
        }
        lowerer.refresh_inference_normalization_env();
    }

    pub(crate) fn register_extern_crate(&self, lowerer: &mut Lowerer, dep: ExternCrateRef<'_>) {
        let crate_name = dep.name();
        let metadata = dep.metadata();
        let interface = metadata.interface();

        lowerer.imported_effective_trait_methods.extend(
            interface
                .effective_trait_methods
                .iter()
                .map(|(&key, &method_id)| (key, method_id)),
        );

        lowerer
            .prelude
            .capture_loaded_prelude_exports(crate_name, dep.prelude_exports());

        for (name, func) in interface.function_items() {
            let param_types: Vec<Type> = func.params.iter().map(|param| param.ty.clone()).collect();
            let func_type = Type::function_with_safety(
                param_types,
                func.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            );
            lowerer
                .scope
                .define_top_level(name.clone(), func_type, false);
            lowerer.items.insert_function(func.clone());
        }

        for ext in interface.extern_items() {
            let Some(name) = lowerer.canonical_name_for_def_id(ext.id).map(str::to_owned) else {
                lowerer.diagnostics.push_toolchain_once(format!(
                    "missing canonical extern declaration name for DefId {:?}",
                    ext.id
                ));
                lowerer.items.insert_extern(ext);
                continue;
            };
            let func_type = Type::function_with_safety(
                ext.params.clone(),
                ext.ret.clone(),
                crate::types::FunctionSafety::from_is_unsafe(ext.is_unsafe),
            );
            lowerer.scope.define_top_level(name, func_type, false);
            lowerer.items.insert_extern(ext);
        }

        for (_, strukt) in interface.struct_items() {
            lowerer.items.insert_structure(strukt.clone());
        }

        for (_, enum_) in interface.enum_items() {
            lowerer.items.insert_enumeration(enum_.clone());
        }

        for (_, alias) in interface.type_alias_items() {
            lowerer.items.insert_type_alias(alias);
        }

        for (_, trait_) in interface.trait_items() {
            lowerer.items.insert_trait_def(trait_.clone());
        }

        for imp in interface.impl_items() {
            if let Err(error) = lowerer.items.insert_impl(imp.clone()) {
                lowerer.diagnostics.push_toolchain(error.message);
            }
        }

        for (name, precedence) in &interface.infix_precedence {
            lowerer.infix_precedence.insert(name.clone(), *precedence);
        }
    }
}

#[cfg(test)]
mod tests;
