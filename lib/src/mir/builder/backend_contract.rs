use crate::ids::TypeId;
use crate::type_context::TypeContext;

use crate::mir::{MirClosureId, MirFunction, MirFunctionId, MirInstanceBodies};

use super::*;

impl<'a> MirBuilder<'a> {
    pub(super) fn populate_backend_contract_function_bodies(
        contract: &mut crate::mir::MirBackendContract,
        functions: &std::collections::BTreeMap<MirFunctionId, MirFunction>,
        type_context: &TypeContext,
    ) {
        for (function_id, function) in functions {
            if contract.function_bodies.contains_key(function_id) {
                continue;
            }

            let key = match function_id {
                MirFunctionId::Function(id) => crate::mir::MirCallableKey::Function(*id),
                MirFunctionId::Extern(id) => crate::mir::MirCallableKey::Extern(*id),
                MirFunctionId::Instance(id) => crate::mir::MirCallableKey::Instance(*id),
                MirFunctionId::Closure(_) => {
                    crate::mir::MirCallableKey::Closure(function_id.clone())
                }
            };
            if contract.callables.contains_key(&key) {
                continue;
            }

            let params = function
                .local_decls
                .iter()
                .skip(1)
                .take(function.arg_count)
                .map(|local| local.ty)
                .collect::<Vec<_>>();
            let signature = crate::mir::MirCallableSignature::from_type_ids(
                &params,
                function.ret_type,
                crate::mir::MirPassMode::Direct,
            );

            contract.callables.insert(
                key.clone(),
                crate::mir::MirCallableDecl {
                    key: key.clone(),
                    source_def_id: match function_id {
                        MirFunctionId::Function(id) | MirFunctionId::Extern(id) => Some(*id),
                        MirFunctionId::Instance(_) | MirFunctionId::Closure(_) => None,
                    },
                    kind: crate::mir::MirCallableKind::LocalBody {
                        function_id: function_id.clone(),
                    },
                    llvm_symbol: Self::mir_contract_local_body_symbol(function_id, function),
                    linkage: crate::mir::MirLinkage::Internal,
                    signature,
                },
            );
            contract.function_bodies.insert(function_id.clone(), key);
        }

        Self::populate_backend_contract_intrinsic_callables(contract, functions, type_context);
    }

    pub(super) fn populate_backend_contract_runtime_requirements(
        contract: &mut crate::mir::MirBackendContract,
        functions: &std::collections::BTreeMap<MirFunctionId, MirFunction>,
    ) {
        contract.runtime_requirements =
            crate::mir::backend_contract::runtime_requirements_for_functions(functions.values());
    }

    fn mir_contract_local_body_symbol(
        function_id: &MirFunctionId,
        function: &MirFunction,
    ) -> String {
        match function_id {
            MirFunctionId::Closure(closure_id) => {
                Self::mir_contract_closure_symbol(&function.name, closure_id)
            }
            _ => function.name.clone(),
        }
    }

    fn mir_contract_closure_symbol(parent_symbol: &str, closure_id: &MirClosureId) -> String {
        format!(
            "__mir_closure_{}_{}_{}",
            Self::sanitize_mir_contract_symbol(parent_symbol),
            Self::mir_function_id_symbol_key(&closure_id.owner),
            closure_id.local_index
        )
    }

    fn mir_function_id_symbol_key(id: &MirFunctionId) -> String {
        match id {
            MirFunctionId::Function(id) => format!("fn_{}_{}", id.crate_id.0, id.local.0),
            MirFunctionId::Extern(id) => format!("extern_{}_{}", id.crate_id.0, id.local.0),
            MirFunctionId::Instance(id) => format!("instance_{}", id.0),
            MirFunctionId::Closure(id) => format!(
                "closure_{}_{}",
                Self::mir_function_id_symbol_key(&id.owner),
                id.local_index
            ),
        }
    }

    fn sanitize_mir_contract_symbol(symbol: &str) -> String {
        symbol
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
            .collect()
    }

    pub(super) fn backend_contract_for_program(
        program: &crate::mono::MonomorphizedProgram,
        functions: &std::collections::BTreeMap<MirFunctionId, MirFunction>,
        type_context: &mut TypeContext,
        bodies: &MirInstanceBodies,
    ) -> crate::mir::MirBackendContract {
        let mut contract = crate::mir::MirBackendContract::default();
        Self::populate_backend_contract_externs(&mut contract, &program.program, type_context);
        Self::populate_backend_contract_nominal_layouts(
            &mut contract,
            &program.program,
            type_context,
        );

        contract.projection_traits.extend(
            program
                .program
                .traits_by_id()
                .map(|(trait_id, _, _)| trait_id),
        );
        contract.projection_traits.extend(
            program
                .program
                .impls_in_order()
                .filter_map(|(_, imp)| imp.trait_id),
        );

        let exportable_drop_glue_symbols = program
            .instances
            .values()
            .filter(|record| {
                Self::instance_is_drop_method(program, record)
                    && record.substitution.is_empty()
                    && bodies.contains_key(record.id)
                    && !record.provided_by_object
            })
            .map(|record| record.symbols.backend_symbol.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        for record in program.instances.values() {
            let (params, ret, is_method): (Vec<TypeId>, TypeId, bool) =
                if let Some(body) = bodies.get(record.id) {
                    (
                        body.function
                            .local_decls
                            .iter()
                            .skip(1)
                            .take(body.function.arg_count)
                            .map(|local| local.ty)
                            .collect(),
                        body.function.ret_type,
                        body.is_method,
                    )
                } else if let Some(function) = record.declared.as_ref() {
                    (
                        function
                            .params
                            .iter()
                            .map(|param| type_context.intern_canonical_type(&param.ty))
                            .collect(),
                        type_context.intern_canonical_type(&function.ret_type),
                        function.is_method,
                    )
                } else {
                    panic!(
                        "instance {:?} has neither a MIR body nor a declared signature",
                        record.id,
                    );
                };

            let key = crate::mir::MirCallableKey::Instance(record.id);
            let function_id = MirFunctionId::Instance(record.id);
            let kind = if record.provided_by_object {
                crate::mir::MirCallableKind::ObjectProvided
            } else {
                assert!(
                    contract
                        .function_bodies
                        .insert(function_id.clone(), key.clone())
                        .is_none(),
                    "duplicate instance body authority for {:?}",
                    record.id,
                );
                crate::mir::MirCallableKind::LocalBody { function_id }
            };
            let signature = crate::mir::MirCallableSignature::from_instance_type_ids(
                &params,
                ret,
                is_method,
                type_context,
            );
            let linkage = if record.is_specialization
                && !record.provided_by_object
                && !exportable_drop_glue_symbols.contains(record.symbols.backend_symbol.as_str())
            {
                crate::mir::MirLinkage::Internal
            } else {
                crate::mir::MirLinkage::External
            };
            assert!(
                contract
                    .callables
                    .insert(
                        key.clone(),
                        crate::mir::MirCallableDecl {
                            key,
                            source_def_id: Some(Self::instance_origin_callable_def_id(
                                &record.origin
                            )),
                            kind,
                            llvm_symbol: record.symbols.backend_symbol.clone(),
                            linkage,
                            signature,
                        },
                    )
                    .is_none(),
                "duplicate instance callable authority for {:?}",
                record.id,
            );
            contract
                .artifact_exports
                .push(crate::mir::MirArtifactExport {
                    origin_def_id: Some(Self::instance_origin_callable_def_id(&record.origin)),
                    source_name: record.symbols.source_name.clone(),
                    backend_symbol: record.symbols.backend_symbol.clone(),
                    substitution_empty: record.substitution.is_empty(),
                    has_body: bodies.contains_key(record.id),
                    provided_by_object: record.provided_by_object,
                    is_specialization: record.is_specialization,
                    is_drop_glue: Self::instance_is_drop_method(program, record),
                });
        }

        Self::populate_backend_contract_projection_outputs(
            &mut contract,
            &program.program,
            functions,
            type_context,
        );
        Self::populate_backend_contract_drop_glue(&mut contract, &program.generated_drop_instances);

        contract
    }

    pub(super) fn mir_generic_param_ids(
        params: &[crate::types::GenericParamDecl],
    ) -> Vec<crate::types::GenericParamId> {
        params.iter().map(|param| param.id).collect()
    }
}
