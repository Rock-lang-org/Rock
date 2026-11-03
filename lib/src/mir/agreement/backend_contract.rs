use std::collections::HashSet;

use crate::ids::{DefId, TypeId};
use crate::mir::{MirNominalLayout, MirProgram};
use crate::type_context::Ty;

use super::*;

pub(super) fn validate_mir_backend_contract(program: &MirProgram, report: &mut MirAgreementReport) {
    let errors = crate::mir::validate_backend_contract_against_mir(
        &program.backend_contract,
        program.functions.values(),
    );
    report.invalid_backend_contract += errors.len();
    report.backend_contract_errors.extend(errors);
}

pub(super) fn validate_backend_contract_type_ids(
    program: &MirProgram,
    report: &mut MirAgreementReport,
) {
    for callable in program.backend_contract.callables.values() {
        for param in &callable.signature.params {
            validate_type_id(program, param.semantic_ty, report);
        }
        validate_type_id(program, callable.signature.ret.semantic_ty, report);
        validate_type_id(program, callable.signature.ret.abi_ty, report);
    }

    for layout in program.backend_contract.nominal_layouts.values() {
        match layout {
            MirNominalLayout::Struct {
                fields,
                generic_params,
                ..
            } => {
                for (_, field_ty) in fields {
                    validate_layout_template_type_id(program, *field_ty, generic_params, report);
                }
            }
            MirNominalLayout::Enum {
                variants,
                generic_params,
                ..
            } => {
                for variant in variants {
                    match &variant.fields {
                        crate::mir::MirVariantLayoutFields::Unit => {}
                        crate::mir::MirVariantLayoutFields::Positional(fields) => {
                            for field_ty in fields {
                                validate_layout_template_type_id(
                                    program,
                                    *field_ty,
                                    generic_params,
                                    report,
                                );
                            }
                        }
                        crate::mir::MirVariantLayoutFields::Named(fields) => {
                            for (_, field_ty) in fields {
                                validate_layout_template_type_id(
                                    program,
                                    *field_ty,
                                    generic_params,
                                    report,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    for (key, output) in &program.backend_contract.projection_outputs {
        validate_type_id(program, key.base, report);
        for trait_arg in &key.trait_args {
            validate_type_id(program, *trait_arg, report);
        }
        validate_type_id(program, *output, report);
    }

    for ty in program.backend_contract.drop_glue.keys() {
        validate_type_id(program, *ty, report);
    }
}

fn validate_layout_template_type_id(
    program: &MirProgram,
    id: TypeId,
    generic_params: &[crate::types::GenericParamId],
    report: &mut MirAgreementReport,
) {
    let Some(ty) = program.type_context.try_ty(id) else {
        report.invalid_type_ids += 1;
        return;
    };

    match ty {
        Ty::Slice(inner) | Ty::Reference { inner, .. } | Ty::Pointer(inner) => {
            validate_layout_template_type_id(program, *inner, generic_params, report);
        }
        Ty::Array { inner, .. } => {
            validate_layout_template_type_id(program, *inner, generic_params, report);
        }
        Ty::Tuple(elems) => {
            for elem in elems {
                validate_layout_template_type_id(program, *elem, generic_params, report);
            }
        }
        Ty::Function {
            params,
            ret,
            captures,
            ..
        } => {
            for param in params {
                validate_layout_template_type_id(program, *param, generic_params, report);
            }
            validate_layout_template_type_id(program, *ret, generic_params, report);
            for capture in captures {
                validate_layout_template_type_id(program, capture.ty, generic_params, report);
            }
        }
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => {
            for arg in args {
                if program.type_context.kind(*arg) == &crate::type_services::kind::Kind::Type {
                    validate_layout_template_type_id(program, *arg, generic_params, report);
                }
            }
        }
        Ty::Projection { ty, trait_args, .. } => {
            validate_layout_template_type_id(program, *ty, generic_params, report);
            for arg in trait_args {
                validate_layout_template_type_id(program, *arg, generic_params, report);
            }
        }
        Ty::Apply { constructor, args } => {
            validate_layout_template_type_id(program, *constructor, generic_params, report);
            for arg in args {
                validate_layout_template_type_id(program, *arg, generic_params, report);
            }
        }
        Ty::Generic(param) if generic_params.contains(param) => {}
        Ty::TypeVar(_)
        | Ty::Generic(_)
        | Ty::Constructor { .. }
        | Ty::Lambda { .. }
        | Ty::BoundVar { .. }
        | Ty::Error => report.invalid_type_ids += 1,
        Ty::I8
        | Ty::I16
        | Ty::I32
        | Ty::I64
        | Ty::U8
        | Ty::U16
        | Ty::U32
        | Ty::U64
        | Ty::F32
        | Ty::F64
        | Ty::Bool
        | Ty::Str
        | Ty::Char
        | Ty::Unit
        | Ty::Never => {}
    }
}

pub(super) fn validate_backend_contract_nominal_layout_ids(
    program: &MirProgram,
    backend_contract: &BackendContract,
    seen_nominals: &mut HashSet<DefId>,
    report: &mut MirAgreementReport,
) {
    for callable in program.backend_contract.callables.values() {
        for param in &callable.signature.params {
            validate_type_nominal_layout_id(
                program,
                backend_contract,
                param.semantic_ty,
                seen_nominals,
                report,
            );
        }
        validate_type_nominal_layout_id(
            program,
            backend_contract,
            callable.signature.ret.semantic_ty,
            seen_nominals,
            report,
        );
        validate_type_nominal_layout_id(
            program,
            backend_contract,
            callable.signature.ret.abi_ty,
            seen_nominals,
            report,
        );
    }

    for (key, layout) in &program.backend_contract.nominal_layouts {
        match layout {
            MirNominalLayout::Struct { id, fields, .. } => {
                if key != id {
                    report.invalid_layout_ids += 1;
                }
                for (_, field_ty) in fields {
                    validate_type_nominal_layout_id(
                        program,
                        backend_contract,
                        *field_ty,
                        seen_nominals,
                        report,
                    );
                }
            }
            MirNominalLayout::Enum { id, variants, .. } => {
                if key != id {
                    report.invalid_layout_ids += 1;
                }
                for variant in variants {
                    match &variant.fields {
                        crate::mir::MirVariantLayoutFields::Unit => {}
                        crate::mir::MirVariantLayoutFields::Positional(fields) => {
                            for field_ty in fields {
                                validate_type_nominal_layout_id(
                                    program,
                                    backend_contract,
                                    *field_ty,
                                    seen_nominals,
                                    report,
                                );
                            }
                        }
                        crate::mir::MirVariantLayoutFields::Named(fields) => {
                            for (_, field_ty) in fields {
                                validate_type_nominal_layout_id(
                                    program,
                                    backend_contract,
                                    *field_ty,
                                    seen_nominals,
                                    report,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    for (key, output) in &program.backend_contract.projection_outputs {
        validate_type_nominal_layout_id(program, backend_contract, key.base, seen_nominals, report);
        for trait_arg in &key.trait_args {
            validate_type_nominal_layout_id(
                program,
                backend_contract,
                *trait_arg,
                seen_nominals,
                report,
            );
        }
        validate_type_nominal_layout_id(program, backend_contract, *output, seen_nominals, report);
    }

    for ty in program.backend_contract.drop_glue.keys() {
        validate_type_nominal_layout_id(program, backend_contract, *ty, seen_nominals, report);
    }
}

pub(super) fn validate_backend_contract_projection_traits(
    program: &MirProgram,
    backend_contract: &BackendContract,
    report: &mut MirAgreementReport,
) {
    for key in program.backend_contract.projection_outputs.keys() {
        if !backend_contract.projection_trait_known(key.trait_id) {
            report.stale_projection_facts += 1;
        }
    }
}

pub(super) struct BackendContract {
    pub(super) structs: HashSet<DefId>,
    pub(super) enums: HashSet<DefId>,
    pub(super) projection_traits: HashSet<DefId>,
}

impl BackendContract {
    pub(super) fn new(program: &MirProgram) -> Self {
        let mut structs = HashSet::new();
        let mut enums = HashSet::new();
        for (id, layout) in &program.backend_contract.nominal_layouts {
            match layout {
                MirNominalLayout::Struct { .. } => {
                    structs.insert(*id);
                }
                MirNominalLayout::Enum { .. } => {
                    enums.insert(*id);
                }
            }
        }
        let projection_traits: HashSet<DefId> = program
            .backend_contract
            .projection_traits
            .iter()
            .copied()
            .collect();

        Self {
            structs,
            enums,
            projection_traits,
        }
    }

    pub(super) fn nominal_layout_known(&self, id: DefId) -> bool {
        self.structs.contains(&id) || self.enums.contains(&id)
    }

    pub(super) fn enum_variant_known(
        &self,
        enum_id: DefId,
        _variant_id: crate::ids::VariantId,
    ) -> bool {
        self.enums.contains(&enum_id)
    }

    fn projection_trait_known(&self, trait_id: DefId) -> bool {
        self.projection_traits.contains(&trait_id)
    }
}
