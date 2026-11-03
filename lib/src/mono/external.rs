use super::hir_types::{
    function_requires_downstream_specialization, hir_function_is_codegen_concrete,
    impl_requires_downstream_specialization,
};
use crate::crate_system::{CrateContext, ExternCrateRef};

use super::Monomorphizer;

impl Monomorphizer {
    fn instantiate_dependency_trait_default(
        &mut self,
        trait_id: crate::ids::DefId,
        trait_generic_count: usize,
        trait_args: &[crate::types::Type],
        local_method: &super::hir_types::HirFunction,
        provider: &super::hir_types::HirFunction,
    ) -> super::hir_types::HirFunction {
        let generic_ids = (0..=trait_generic_count)
            .map(|index| crate::types::GenericParamId {
                owner: trait_id,
                index: index as u32,
            })
            .collect::<Vec<_>>();
        let mut substitution = generic_ids
            .iter()
            .take(trait_generic_count)
            .zip(trait_args)
            .map(|(id, ty)| (*id, self.intern_type(ty)))
            .collect::<std::collections::HashMap<_, _>>();

        for (provider_param, local_param) in provider.params.iter().zip(&local_method.params) {
            self.extract_generics_from_type(
                &provider_param.ty,
                &local_param.ty,
                &generic_ids,
                &mut substitution,
            );
        }
        self.extract_generics_from_type(
            &provider.ret_type,
            &local_method.ret_type,
            &generic_ids,
            &mut substitution,
        );

        let mut instantiated = provider.clone();
        instantiated.id = local_method.id;
        instantiated.name = local_method.name.clone();
        instantiated.generic_params = local_method.generic_params.clone();
        instantiated.generic_bounds = local_method.generic_bounds.clone();
        for (provider_param, local_param) in
            instantiated.params.iter_mut().zip(&local_method.params)
        {
            provider_param.ty = local_param.ty.clone();
        }
        instantiated.ret_type = local_method.ret_type.clone();
        instantiated.is_curried = local_method.is_curried;
        instantiated.is_method = local_method.is_method;
        instantiated.self_receiver = local_method.self_receiver;
        instantiated.is_unsafe = local_method.is_unsafe;
        instantiated.body = self.substitute_block(&provider.body, &substitution);
        instantiated
    }

    /// Process a program with external crate support
    pub(super) fn process_with_crates(
        &mut self,
        mut program: super::hir_types::HirProgram,
        crate_ctx: &CrateContext,
    ) -> super::hir_types::HirProgram {
        self.process_with_crates_impl(&mut program, crate_ctx);
        let mut canonical_names_by_id = Self::canonical_names_from_indexes(&program);
        canonical_names_by_id.extend(
            self.resolver
                .item_names_by_id
                .iter()
                .map(|(id, name)| (*id, name.clone())),
        );
        program.rebuild_indexes_with_canonical_names(&canonical_names_by_id);
        program
    }

    fn process_with_crates_impl(
        &mut self,
        program: &mut super::hir_types::HirProgram,
        crate_ctx: &CrateContext,
    ) {
        self.language_items = program.language_items.clone();
        self.dependency_resolvers = crate_ctx
            .extern_crates()
            .map(|dep| dep.metadata().resolver().clone())
            .collect();
        self.drop_trait_id = program
            .language_items
            .drop
            .as_ref()
            .map(|items| items.trait_id);
        self.drop_method_id = program
            .language_items
            .drop
            .as_ref()
            .map(|items| items.method_id);
        self.load_external_generic_functions(crate_ctx);
        for dependency in crate_ctx.extern_crates() {
            for (trait_id, trait_def) in dependency.body_providers().traits_with_defaults() {
                if program.traits.contains_key(trait_id) {
                    program.traits.insert(*trait_id, trait_def.clone());
                }
            }
            for (impl_id, imp) in dependency.body_providers().generic_impls() {
                if program.impls.contains_key(impl_id) {
                    program.impls.insert(*impl_id, imp.clone());
                }
            }
        }
        let mut impl_ids = program.impls.keys().copied().collect::<Vec<_>>();
        impl_ids.sort();
        for impl_id in impl_ids {
            let Some(imp) = program.impls.get_mut(&impl_id) else {
                continue;
            };
            let Some(trait_id) = imp.trait_id else {
                continue;
            };
            let Some(trait_def) = program.traits.get(&trait_id) else {
                continue;
            };
            let trait_generic_count = trait_def.generic_params.len();
            let trait_args = imp.trait_arg_types.clone();
            let method_names = Self::method_names_in_id_order(&imp.methods);
            for name in method_names {
                let Some(method) = imp.methods.get_mut(&name) else {
                    continue;
                };
                if !method.body.stmts.is_empty() {
                    continue;
                }
                let Some(default) = trait_def.methods.get(&name) else {
                    continue;
                };
                if method.id == default.id {
                    imp.methods.remove(&name);
                } else {
                    *method = self.instantiate_dependency_trait_default(
                        trait_id,
                        trait_generic_count,
                        &trait_args,
                        method,
                        default,
                    );
                }
            }
        }
        self.register_imported_function_instances(crate_ctx);
        self.register_effective_trait_methods(program);
        self.register_nominal_field_types(program);
        let mut impls = program
            .impls_in_order()
            .map(|(_, imp)| imp.clone())
            .collect::<Vec<_>>();
        impls.sort_by_key(|imp| imp.id);
        self.collect_impls_with_crate_capabilities(&impls, crate_ctx);
        self.register_imported_impl_instances(&impls, crate_ctx);

        let mut all_funcs = program
            .functions_by_id()
            .map(|(id, name, func)| (id, name.to_string(), func.clone()))
            .collect::<Vec<_>>();
        all_funcs.sort_by_key(|(id, _, _)| *id);
        program.functions.clear();
        for (_, name, func) in all_funcs {
            if crate_ctx.extern_crates().any(|dependency| {
                dependency
                    .body_providers()
                    .generic_function(func.id)
                    .is_some()
            }) {
                continue;
            }
            let origin = self.function_instance_origin(&func);
            let key = crate::mono::InstanceKey::new(origin, Vec::new());
            let provided_by_object = self
                .instances
                .get(&key)
                .and_then(|id| self.instances.record(id))
                .is_some_and(|record| record.provided_by_object);
            if provided_by_object {
                continue;
            }

            let is_entrypoint = name == "main" && func.generic_params.is_empty();
            if is_entrypoint
                || (!function_requires_downstream_specialization(&func)
                    && hir_function_is_codegen_concrete(&func))
            {
                self.register_concrete_function(name.clone(), func);
            } else {
                self.register_generic_function(name, func);
            }
        }

        let mut function_ids = self.concrete_functions.keys().copied().collect::<Vec<_>>();
        function_ids.sort();
        for id in function_ids {
            if let Some(func) = self.concrete_functions.remove(&id) {
                let processed = self.process_function(func);
                self.concrete_functions.insert(id, processed);
            }
        }

        let mut concrete_methods = program
            .impls
            .iter()
            .filter(|(_, imp)| !crate_ctx.provides_dependency_impl_body(imp))
            .flat_map(|(_, imp)| {
                imp.methods.iter().filter_map(|(name, method)| {
                    hir_function_is_codegen_concrete(method).then_some((
                        imp.id,
                        method.id,
                        imp.clone(),
                        name.clone(),
                        method.clone(),
                    ))
                })
            })
            .collect::<Vec<_>>();
        concrete_methods.sort_by_key(|(impl_id, method_id, _, _, _)| (*impl_id, *method_id));
        for (_, _, imp, name, method) in concrete_methods {
            self.register_impl_method_instance(&imp, &name, &method);
        }

        let mut impl_ids = program.impls.keys().copied().collect::<Vec<_>>();
        impl_ids.sort();
        for impl_id in impl_ids {
            let Some(imp) = program.impls.get_mut(&impl_id) else {
                continue;
            };
            if crate_ctx.provides_dependency_impl_body(imp) {
                continue;
            }

            let method_names = Self::method_names_in_id_order(&imp.methods);
            for name in method_names {
                if let Some(func) = imp.methods.remove(&name) {
                    if !hir_function_is_codegen_concrete(&func) {
                        imp.methods.insert(name, func);
                        continue;
                    }
                    let processed = self.process_function(func);
                    let instance_key = crate::mono::InstanceKey::new(
                        self.method_instance_origin(imp, &processed),
                        Vec::new(),
                    );
                    if let Some(instance_id) = self.instances.get(&instance_key) {
                        self.instances
                            .replace_pre_mir_body(instance_id, processed.clone());
                    } else if Self::should_register_method_instance(&processed) {
                        self.register_impl_method_instance(imp, &name, &processed);
                    }
                    imp.methods.insert(name, processed);
                }
            }
        }

        self.process_and_register_concrete_trait_defaults(program);

        program.functions.clear();
        program.names.functions_by_name.clear();
        let mut functions = std::mem::take(&mut self.concrete_functions)
            .into_iter()
            .collect::<Vec<_>>();
        functions.sort_by_key(|(id, _)| *id);
        for (id, func) in functions {
            let name = self
                .function_names_by_id
                .get(&id)
                .expect("registered function must retain canonical display metadata")
                .clone();
            if Self::should_register_function_instance(&func) {
                self.register_function_instance(&name, &func);
            }
            program.names.functions_by_name.insert(name, id);
            program.functions.insert(id, func);
        }
        self.materialize_registered_body_edges();
        self.validate_materialized_body_edges();
    }

    fn register_imported_function_instances(&mut self, crate_ctx: &CrateContext) {
        for dep in crate_ctx.extern_crates() {
            let metadata = dep.metadata();

            for (name, func) in metadata.interface().function_items() {
                let declared = accepted_function_declaration(&func);
                let Some(backend_symbol) = dep.imported_function_symbol(&name, &declared) else {
                    continue;
                };

                let origin = self.function_instance_origin(&declared);
                let instance_key = crate::mono::InstanceKey::new(origin.clone(), Vec::new());
                let mut declared = declared;
                declared.name = name.clone();

                self.instances
                    .intern(instance_key, |id| crate::mono::InstanceRecord {
                        id,
                        origin: origin.clone(),
                        substitution: Vec::new(),
                        symbols: crate::mono::InstanceSymbols::new(
                            name.clone(),
                            backend_symbol.clone(),
                        ),
                        declared: Some(declared.clone()),
                        provided_by_object: true,
                        is_specialization: false,
                    });
            }
        }
    }

    fn register_imported_impl_instances(
        &mut self,
        program_impls: &[super::hir_types::HirImpl],
        crate_ctx: &CrateContext,
    ) {
        for dep in crate_ctx.extern_crates() {
            let crate_name = dep.name();
            let metadata = dep.metadata();

            for imp in metadata.interface().impl_items() {
                let receiver_pattern = metadata
                    .interface()
                    .impls
                    .get(&imp.id)
                    .map(|imp| &imp.receiver_pattern)
                    .expect("validated artifact impl must have canonical receiver authority");
                let imp = accepted_impl_declaration(&imp, receiver_pattern);
                self.record_imported_impl(crate_name, &imp, Some(dep));
            }
        }

        for imp in program_impls {
            if let Some(dep) = crate_ctx.impl_body_provider(imp) {
                self.record_imported_impl(dep.name(), imp, Some(dep));
            }
        }
    }

    fn record_imported_impl(
        &mut self,
        crate_name: &str,
        imp: &super::hir_types::HirImpl,
        dep: Option<ExternCrateRef<'_>>,
    ) {
        let Some(dep) = dep else {
            return;
        };

        for method_name in Self::method_names_in_id_order(&imp.methods) {
            let Some(method) = imp.methods.get(&method_name) else {
                continue;
            };
            if !dep.provides_impl_method_body(imp, method) {
                continue;
            }

            let Some(backend_symbol) = dep.imported_impl_method_symbol(imp, &method_name, method)
            else {
                continue;
            };
            let origin = self.method_instance_origin(imp, method);
            let instance_key = crate::mono::InstanceKey::new(origin.clone(), Vec::new());
            let mut declared = method.clone();
            declared.name = format!("{}::{}", crate_name, method_name);

            self.instances
                .intern(instance_key, |id| crate::mono::InstanceRecord {
                    id,
                    origin: origin.clone(),
                    substitution: Vec::new(),
                    symbols: crate::mono::InstanceSymbols::new(
                        declared.name.clone(),
                        backend_symbol.clone(),
                    ),
                    declared: Some(declared),
                    provided_by_object: true,
                    is_specialization: false,
                });
        }
    }

    /// Load external generic function payloads into the canonical ID-keyed store.
    fn load_external_generic_functions(&mut self, crate_ctx: &CrateContext) {
        for dep in crate_ctx.extern_crates() {
            let metadata = dep.metadata();

            for imp in metadata.interface().impl_items() {
                let receiver_pattern = metadata
                    .interface()
                    .impls
                    .get(&imp.id)
                    .map(|imp| &imp.receiver_pattern)
                    .expect("validated artifact impl must have canonical receiver authority");
                let imp = accepted_impl_declaration(&imp, receiver_pattern);
                if imp.trait_name.is_some()
                    && (dep.provides_impl_body(&imp) || dep.provides_any_impl_method_body(&imp))
                {
                    let Some(trait_id) = imp.trait_id else {
                        continue;
                    };
                    self.trait_impls
                        .entry(trait_id)
                        .or_insert_with(Vec::new)
                        .push(imp);
                }
            }

            for (id, func) in dep.body_providers().generic_functions() {
                let provider_name = dep
                    .metadata()
                    .interface()
                    .canonical_name(*id)
                    .unwrap_or(&func.name)
                    .to_string();
                self.register_external_generic_function(provider_name, func.clone());
            }

            for imp in dep.body_providers().generic_impls().values() {
                if imp.trait_name.is_some() {
                    let Some(trait_id) = imp.trait_id else {
                        continue;
                    };
                    self.trait_impls
                        .entry(trait_id)
                        .or_insert_with(Vec::new)
                        .push(imp.clone());
                } else if impl_requires_downstream_specialization(imp) {
                    self.generic_impls.insert(imp.id, imp.clone());
                }
            }
        }
    }
}

fn accepted_function_declaration(
    function: &crate::hir::HirFunction,
) -> super::hir_types::HirFunction {
    crate::hir::HirFunctionFor {
        id: function.id,
        name: function.name.clone(),
        generic_params: function.generic_params.clone(),
        generic_bounds: function.generic_bounds.clone(),
        params: function.params.clone(),
        ret_type: function.ret_type.clone(),
        body: crate::hir::HirBlockFor {
            stmts: Vec::new(),
            ty: function.ret_type.clone(),
        },
        is_curried: function.is_curried,
        is_method: function.is_method,
        self_receiver: function.self_receiver,
        is_unsafe: function.is_unsafe,
    }
}

fn accepted_impl_declaration(
    imp: &crate::hir::HirImpl,
    receiver_pattern: &crate::hir::HirImplReceiverPattern,
) -> super::hir_types::HirImpl {
    crate::hir::HirImplFor {
        id: imp.id,
        owner: imp.owner.clone(),
        type_name: imp.type_name.clone(),
        type_generics: imp.type_generics.clone(),
        receiver_pattern: receiver_pattern.clone(),
        trait_name: imp.trait_name.clone(),
        trait_id: imp.trait_id,
        trait_generics: imp.trait_generics.clone(),
        trait_arg_types: imp.trait_arg_types.clone(),
        associated_types: imp.associated_types.clone(),
        bounds: imp.bounds.clone(),
        methods: imp
            .methods
            .iter()
            .map(|(name, method)| (name.clone(), accepted_function_declaration(method)))
            .collect(),
    }
}

#[cfg(test)]
mod tests;
