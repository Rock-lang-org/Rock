//! Demand concrete instances from roots and from references encountered while
//! draining the body queue. An available dependency symbol is not an instance
//! until something actually references it.

use crate::ids::{DefId, InstanceId};
use crate::CrateKind;

use super::hir_types::{HirFunction, HirProgram};
use super::{InstanceKey, InstanceOrigin, InstanceRecord, InstanceSymbols, Monomorphizer};

pub(super) struct ObjectInstance {
    pub(super) symbols: InstanceSymbols,
    pub(super) declaration: HirFunction,
}

impl Monomorphizer {
    pub(super) fn demand_object(&mut self, key: &InstanceKey) -> Option<InstanceId> {
        if let Some(id) = self.instances.get(key) {
            return Some(id);
        }
        let object = self.object_instances.remove(key)?;
        Some(self.instances.intern(key.clone(), |id| InstanceRecord {
            id,
            origin: key.origin.clone(),
            substitution: key.substitution.clone(),
            symbols: object.symbols,
            declared: Some(object.declaration),
            provided_by_object: true,
            is_specialization: false,
        }))
    }

    pub(super) fn demand_function(&mut self, id: DefId) -> Option<InstanceId> {
        let key = InstanceKey::new(InstanceOrigin::Function(id), Vec::new());
        if let Some(instance) = self.demand_object(&key) {
            return Some(instance);
        }
        let function = self.concrete_functions.get(&id)?.clone();
        let name = self.function_names_by_id.get(&id)?.clone();
        Some(self.register_function_instance(&name, &function))
    }

    pub(super) fn seed_function_roots(&mut self) {
        let mut roots = self
            .concrete_functions
            .keys()
            .copied()
            .filter(|id| {
                id.crate_id == self.root_crate_id
                    && (self.crate_kind == CrateKind::Lib
                        || self
                            .function_names_by_id
                            .get(id)
                            .is_some_and(|name| name == "main"))
            })
            .collect::<Vec<_>>();
        roots.sort();
        for id in roots {
            self.demand_function(id);
        }
    }

    pub(super) fn seed_method_roots(&mut self, program: &HirProgram) {
        if self.crate_kind != CrateKind::Lib {
            return;
        }
        let mut methods = program
            .impls
            .values()
            .filter(|imp| {
                imp.id.crate_id == self.root_crate_id
                    && imp.type_generics.is_empty()
                    && imp
                        .trait_arg_types
                        .iter()
                        .all(crate::type_services::facts::TypeFacts::is_codegen_concrete)
            })
            .flat_map(|imp| {
                imp.methods.iter().filter_map(move |(name, method)| {
                    Self::should_register_method_instance(method).then_some((imp, name, method))
                })
            })
            .collect::<Vec<_>>();
        methods.sort_by_key(|(imp, _, method)| (imp.id, method.id));
        for (imp, name, method) in methods {
            self.register_impl_method_instance(imp, name, method);
        }
        let mut defaults = program
            .traits
            .values()
            .filter(|tr| tr.id.crate_id == self.root_crate_id)
            .flat_map(|tr| {
                tr.methods.iter().filter_map(move |(name, method)| {
                    Self::should_register_method_instance(method).then_some((tr, name, method))
                })
            })
            .collect::<Vec<_>>();
        defaults.sort_by_key(|(tr, _, method)| (tr.id, method.id));
        for (tr, name, method) in defaults {
            self.register_trait_default_instance(tr.id, &tr.name, name, method);
        }
    }

    /// Keep declaration metadata separate from the processed instance bodies.
    pub(super) fn finish_demanded_functions(&mut self, program: &mut HirProgram) {
        program.functions.clear();
        program.names.functions_by_name.clear();
        for record in self.instances.records() {
            if record.provided_by_object || !record.substitution.is_empty() {
                continue;
            }
            let InstanceOrigin::Function(id) = record.origin else {
                continue;
            };
            if !self.concrete_functions.contains_key(&id) {
                continue;
            }
            if let Some(body) = self.instances.pre_mir_body(record.id) {
                program.functions.insert(id, body.clone());
                program
                    .names
                    .functions_by_name
                    .insert(record.symbols.source_name.clone(), id);
            }
        }
    }
}
