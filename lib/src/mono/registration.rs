use std::collections::HashMap;

use crate::hir;
use crate::ids::DefId;
use crate::types::Type;

use super::hir_types::{
    HirExpr, HirExprKind, HirFunction, HirImpl, HirImplOwner, HirProgram, HirVarRef, HirVarTarget,
};
use super::{
    InstanceId, InstanceImplOwner, InstanceKey, InstanceOrigin, InstanceRecord, InstanceSymbols,
    Monomorphizer,
};

impl Monomorphizer {
    pub(super) fn function_instance_origin(&self, func: &HirFunction) -> InstanceOrigin {
        InstanceOrigin::Function(func.id)
    }

    pub(super) fn register_concrete_function(&mut self, name: String, func: HirFunction) {
        self.function_names_by_id.entry(func.id).or_insert(name);
        self.concrete_functions.insert(func.id, func);
    }

    pub(super) fn concrete_function_by_id(&self, id: DefId) -> Option<HirFunction> {
        self.concrete_functions.get(&id).cloned()
    }

    pub(super) fn register_generic_function(&mut self, name: String, func: HirFunction) {
        self.function_names_by_id.entry(func.id).or_insert(name);
        self.generic_functions.insert(func.id, func);
    }

    pub(super) fn register_external_generic_function(&mut self, name: String, func: HirFunction) {
        self.register_generic_function(name, func);
    }

    pub(super) fn method_names_in_id_order(methods: &HashMap<String, HirFunction>) -> Vec<String> {
        let mut methods = methods
            .iter()
            .map(|(name, method)| (method.id, name.clone()))
            .collect::<Vec<_>>();
        methods.sort_by_key(|(id, _)| *id);
        methods.into_iter().map(|(_, name)| name).collect()
    }

    pub(super) fn generic_function_by_id(&self, id: DefId) -> Option<(String, HirFunction)> {
        let name = self.function_names_by_id.get(&id)?.clone();
        let func = self.generic_functions.get(&id)?.clone();
        Some((name, func))
    }

    fn impl_owner_identity(&self, imp: &HirImpl) -> InstanceImplOwner {
        match &imp.owner {
            HirImplOwner::BuiltinSlice => InstanceImplOwner::BuiltinSlice,
            HirImplOwner::Named(_) => InstanceImplOwner::Named(imp.id),
        }
    }

    pub(super) fn lookup_instance_function(&self, instance_id: InstanceId) -> Option<HirFunction> {
        self.instances.pre_mir_body(instance_id).cloned()
    }

    pub(super) fn instance_callable_expr(
        &self,
        display_name: String,
        instance_id: InstanceId,
        ty: Type,
        span: crate::lexer::Span,
    ) -> HirExpr {
        HirExpr {
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: display_name,
                target: HirVarTarget::Instance(instance_id),
            }),
            ty,
            span,
        }
    }

    pub(super) fn method_instance_origin(
        &self,
        imp: &HirImpl,
        method: &HirFunction,
    ) -> InstanceOrigin {
        InstanceOrigin::ImplMethod {
            owner: self.impl_owner_identity(imp),
            method: method.id,
        }
    }

    pub(super) fn register_function_instance(
        &mut self,
        source_name: &str,
        func: &HirFunction,
    ) -> InstanceId {
        let origin = InstanceOrigin::Function(func.id);
        let key = InstanceKey::new(origin.clone(), Vec::new());
        let body = func.clone();

        let instance_id = self.instances.intern(key, |id| InstanceRecord {
            id,
            origin,
            substitution: Vec::new(),
            symbols: InstanceSymbols::pending(source_name),
            declared: None,
            provided_by_object: false,
            is_specialization: false,
        });
        self.instances.insert_pre_mir_body(instance_id, body);
        instance_id
    }

    pub(super) fn should_register_function_instance(func: &HirFunction) -> bool {
        func.generic_params.is_empty() && hir::hir_function_is_codegen_concrete(func)
    }

    pub(super) fn register_impl_method_instance(
        &mut self,
        imp: &HirImpl,
        method_name: &str,
        method: &HirFunction,
    ) -> InstanceId {
        let origin = self.method_instance_origin(imp, method);
        let key = InstanceKey::new(origin.clone(), Vec::new());
        if let Some(instance_id) = self.instances.get(&key) {
            return instance_id;
        }
        let backend_symbol = String::new();
        let body = method.clone();

        let instance_id = self.instances.intern(key, |id| InstanceRecord {
            id,
            origin,
            substitution: Vec::new(),
            symbols: InstanceSymbols::new(
                format!("{}::{}", imp.type_name, method_name),
                backend_symbol,
            ),
            declared: None,
            provided_by_object: false,
            is_specialization: false,
        });
        self.instances.insert_pre_mir_body(instance_id, body);
        instance_id
    }

    pub(super) fn should_register_method_instance(method: &HirFunction) -> bool {
        method.generic_params.is_empty() && hir::hir_function_is_codegen_concrete(method)
    }

    fn register_trait_default_instance(
        &mut self,
        trait_id: DefId,
        trait_name: &str,
        method_name: &str,
        method: &HirFunction,
    ) -> InstanceId {
        let origin = InstanceOrigin::TraitDefault {
            trait_id,
            method: method.id,
        };
        let key = InstanceKey::new(origin.clone(), Vec::new());
        if let Some(instance_id) = self.instances.get(&key) {
            return instance_id;
        }
        let backend_symbol = String::new();
        let body = method.clone();

        let instance_id = self.instances.intern(key, |id| InstanceRecord {
            id,
            origin,
            substitution: Vec::new(),
            symbols: InstanceSymbols::new(
                format!("{}::{}", trait_name, method_name),
                backend_symbol,
            ),
            declared: None,
            provided_by_object: false,
            is_specialization: false,
        });
        self.instances.insert_pre_mir_body(instance_id, body);
        instance_id
    }

    pub(super) fn process_and_register_concrete_trait_defaults(
        &mut self,
        program: &mut HirProgram,
    ) {
        let mut traits = program
            .traits_by_id()
            .map(|(trait_id, trait_name, _)| (trait_id, trait_name.to_string()))
            .collect::<Vec<_>>();
        traits.sort_by_key(|(trait_id, _)| *trait_id);

        for (trait_id, trait_name) in &traits {
            let methods = program
                .traits
                .get(trait_id)
                .map(|trait_def| {
                    let mut methods = trait_def
                        .methods
                        .iter()
                        .filter_map(|(name, method)| {
                            hir::hir_function_is_codegen_concrete(method).then_some((
                                method.id,
                                name.clone(),
                                method.clone(),
                            ))
                        })
                        .collect::<Vec<_>>();
                    methods.sort_by_key(|(id, _, _)| *id);
                    methods
                        .into_iter()
                        .map(|(_, name, method)| (name, method))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for (method_name, method) in methods {
                self.register_trait_default_instance(*trait_id, trait_name, &method_name, &method);
            }
        }

        for (trait_id, trait_name) in traits {
            let method_names = program
                .traits
                .get(&trait_id)
                .map(|trait_def| Self::method_names_in_id_order(&trait_def.methods))
                .unwrap_or_default();

            for method_name in method_names {
                let Some(method) = program
                    .traits
                    .get_mut(&trait_id)
                    .and_then(|trait_def| trait_def.methods.remove(&method_name))
                else {
                    continue;
                };

                if !hir::hir_function_signature_is_codegen_concrete(&method) {
                    if let Some(trait_def) = program.traits.get_mut(&trait_id) {
                        trait_def.methods.insert(method_name, method);
                    }
                    continue;
                }

                let processed = self.process_function(method);
                let key = InstanceKey::new(
                    InstanceOrigin::TraitDefault {
                        trait_id,
                        method: processed.id,
                    },
                    Vec::new(),
                );
                if let Some(instance_id) = self.instances.get(&key) {
                    self.instances
                        .replace_pre_mir_body(instance_id, processed.clone());
                } else if Self::should_register_method_instance(&processed) {
                    self.register_trait_default_instance(
                        trait_id,
                        &trait_name,
                        &method_name,
                        &processed,
                    );
                }

                if let Some(trait_def) = program.traits.get_mut(&trait_id) {
                    trait_def.methods.insert(method_name, processed);
                }
            }
        }
    }
}
