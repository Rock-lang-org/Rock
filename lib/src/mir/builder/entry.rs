use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::ids::{DefId, InstanceId};
use crate::source_map::SemanticSourceMap;
use crate::type_context::TypeContext;
use crate::types::Type;

use crate::mir::{
    MirFunctionId, MirInstanceBodies, MirInstanceBody, MirOwnershipMetadata, MirProgram,
};

use super::*;

impl<'a> MirBuilder<'a> {
    pub fn new(program: &'a HirProgram, type_context: &'a RefCell<TypeContext>) -> Self {
        Self {
            program,
            type_context,
            blocks: Vec::new(),
            locals: Vec::new(),
            current_block: None,
            var_map: HashMap::new(),
            loop_stack: Vec::new(),
            scope_locals: Vec::new(),
            moved_places: HashSet::new(),
            statically_moved_cleanup_places: HashSet::new(),
            drop_flags: HashMap::new(),
            projection_drop_flags: HashMap::new(),
            closure_captures: Vec::new(),
            lambda_counter: 0,
            current_function_id: None,
            current_function_name: None,
            nested_functions: Vec::new(),
            pending_closure_body_captures: Vec::new(),
            callable_instances_by_def_id: HashMap::new(),
            method_def_ids: program.indexes.methods_by_id.keys().copied().collect(),
            method_instance_receiver_modes: HashMap::new(),
            direct_drop_types: HashSet::new(),
            ownership: MirOwnershipMetadata::default(),
            source_map: SemanticSourceMap::default(),
            current_source_owner: None,
            cleanup_mode: false,
        }
    }

    pub(super) fn with_instance_metadata(
        program: &'a HirProgram,
        type_context: &'a RefCell<TypeContext>,
        method_instance_receiver_modes: HashMap<InstanceId, Option<crate::types::ReceiverMode>>,
        direct_drop_types: HashSet<Type>,
    ) -> Self {
        Self {
            method_instance_receiver_modes,
            direct_drop_types,
            ..Self::new(program, type_context)
        }
    }

    pub(super) fn with_source_map(mut self, source_map: &SemanticSourceMap) -> Self {
        self.source_map = source_map.clone();
        self
    }

    pub fn build(program: &HirProgram) -> MirProgram {
        let mut functions = std::collections::BTreeMap::new();
        let mut initial_type_context = TypeContext::new();
        let _ = crate::hir::collect_hir_type_ids(program, &mut initial_type_context);
        let type_context = RefCell::new(initial_type_context);

        for (id, name, func) in program.functions_by_id() {
            let mut builder = MirBuilder::new(program, &type_context);
            let mir_id = MirFunctionId::Function(id);
            let mir_func =
                builder.build_function(mir_id.clone(), name, func, false, Some(id), None);
            functions.insert(mir_id, mir_func);
            for nested in builder.take_nested_functions() {
                functions.insert(nested.id.clone(), nested);
            }
        }

        let type_context = type_context.into_inner();
        let mut backend_contract = crate::mir::MirBackendContract::default();
        Self::populate_backend_contract_function_bodies(
            &mut backend_contract,
            &functions,
            &type_context,
        );
        Self::populate_backend_contract_runtime_requirements(&mut backend_contract, &functions);

        MirProgram {
            functions,
            type_context,
            backend_contract,
        }
    }

    pub fn build_monomorphized(program: &crate::mono::MonomorphizedProgram) -> MirProgram {
        let mut program = program.clone();
        let bodies = Self::take_mir_instance_bodies(&mut program);
        Self::build_monomorphized_with_instance_bodies(&program, &bodies)
    }

    pub fn take_mir_instance_bodies(
        program: &mut crate::mono::MonomorphizedProgram,
    ) -> MirInstanceBodies {
        let type_context = RefCell::new(program.type_context.clone());
        let callable_instances_by_def_id = Self::callable_instances_by_def_id_for_program(program);
        let method_def_ids = program
            .instances
            .values()
            .filter_map(|record| match record.origin {
                crate::mono::InstanceOrigin::ImplMethod { method, .. }
                | crate::mono::InstanceOrigin::TraitDefault { method, .. } => Some(method),
                crate::mono::InstanceOrigin::Function(_) => None,
            })
            .collect::<HashSet<_>>();
        let method_instance_receiver_modes = Self::method_instance_receiver_modes_for_program(
            &program.instances,
            &program.pre_mir_instance_bodies,
        );
        let direct_drop_types = Self::direct_drop_types_for_program(program);
        let mut bodies = MirInstanceBodies::new();
        for instance_id in Self::drop_glue_instance_roots(program) {
            bodies.insert_runtime_instance_root(instance_id);
        }

        for record in program.instances.values() {
            if record.provided_by_object {
                continue;
            }
            let Some(body) = program.pre_mir_instance_bodies.take(record.id) else {
                continue;
            };

            let mut builder = MirBuilder::with_instance_metadata(
                &program.program,
                &type_context,
                method_instance_receiver_modes.clone(),
                direct_drop_types.clone(),
            )
            .with_source_map(&program.source_map);
            builder.callable_instances_by_def_id = callable_instances_by_def_id.clone();
            builder
                .method_def_ids
                .extend(method_def_ids.iter().copied());
            let mir_id = MirFunctionId::Instance(record.id);
            let skip_self_cleanup = Self::instance_is_drop_method(program, record);
            let mir_func = builder.build_function(
                mir_id,
                &record.symbols.backend_symbol,
                &body,
                skip_self_cleanup,
                Some(Self::instance_origin_callable_def_id(&record.origin)),
                None,
            );
            bodies.insert(
                record.id,
                MirInstanceBody {
                    function: mir_func,
                    is_method: body.is_method,
                },
            );
            for nested in builder.take_nested_functions() {
                bodies.push_nested(nested);
            }
        }

        program.type_context = type_context.into_inner();
        bodies
    }

    fn drop_glue_instance_roots(
        program: &crate::mono::MonomorphizedProgram,
    ) -> std::collections::BTreeSet<InstanceId> {
        program
            .generated_drop_instances
            .values()
            .map(|generated| generated.instance_id)
            .collect()
    }

    pub(super) fn instance_is_drop_method(
        program: &crate::mono::MonomorphizedProgram,
        record: &crate::mono::InstanceRecord,
    ) -> bool {
        program
            .generated_drop_instances
            .values()
            .any(|generated| generated.instance_id == record.id)
    }

    fn direct_drop_types_for_program(program: &crate::mono::MonomorphizedProgram) -> HashSet<Type> {
        program
            .generated_drop_instances
            .values()
            .map(|generated| program.type_context.type_for(generated.receiver_ty))
            .collect()
    }

    pub fn build_monomorphized_with_instance_bodies(
        program: &crate::mono::MonomorphizedProgram,
        bodies: &MirInstanceBodies,
    ) -> MirProgram {
        let mut functions = std::collections::BTreeMap::new();
        let type_context = RefCell::new(program.type_context.clone());

        for record in program.instances.values() {
            if record.provided_by_object {
                continue;
            }
            let Some(body) = bodies.get(record.id) else {
                continue;
            };
            functions.insert(body.function.id.clone(), body.function.clone());
        }
        for nested in bodies.nested_functions() {
            functions.insert(nested.id.clone(), nested.clone());
        }

        let mut backend_contract = Self::backend_contract_for_program(
            program,
            &functions,
            &mut type_context.borrow_mut(),
            bodies,
        );
        Self::populate_backend_contract_function_bodies(
            &mut backend_contract,
            &functions,
            &type_context.borrow(),
        );
        Self::populate_backend_contract_runtime_requirements(&mut backend_contract, &functions);

        MirProgram {
            functions,
            type_context: type_context.into_inner(),
            backend_contract,
        }
    }

    fn callable_instances_by_def_id_for_program(
        program: &crate::mono::MonomorphizedProgram,
    ) -> HashMap<DefId, InstanceId> {
        let mut instances = HashMap::new();

        for record in program.instances.values() {
            if !record.substitution.is_empty() {
                continue;
            }
            let crate::mono::InstanceOrigin::Function(def_id) = record.origin else {
                continue;
            };
            instances.insert(def_id, record.id);
        }

        instances
    }

    fn method_instance_receiver_modes_for_program(
        instances: &std::collections::BTreeMap<InstanceId, crate::mono::InstanceRecord>,
        pre_mir_bodies: &crate::mono::PreMirInstanceBodies,
    ) -> HashMap<InstanceId, Option<crate::types::ReceiverMode>> {
        instances
            .values()
            .filter_map(|record| match record.origin {
                crate::mono::InstanceOrigin::ImplMethod { .. }
                | crate::mono::InstanceOrigin::TraitDefault { .. } => pre_mir_bodies
                    .get(record.id)
                    .or(record.declared.as_ref())
                    .map(|function| (record.id, function.self_receiver)),
                crate::mono::InstanceOrigin::Function(_) => None,
            })
            .collect()
    }
}
