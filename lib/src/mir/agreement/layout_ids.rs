use std::collections::HashSet;

use crate::ids::{DefId, TypeId};
use crate::mir::{AggregateKind, MirFunction, MirProgram, Place, Projection, Rvalue};
use crate::types::Type;

use super::*;

pub(super) fn validate_function_nominal_layout_ids(
    program: &MirProgram,
    backend_contract: &BackendContract,
    function: &MirFunction,
    seen_nominals: &mut HashSet<DefId>,
    report: &mut MirAgreementReport,
) {
    validate_type_nominal_layout_id(
        program,
        backend_contract,
        function.ret_type,
        seen_nominals,
        report,
    );
    for local in &function.local_decls {
        validate_type_nominal_layout_id(program, backend_contract, local.ty, seen_nominals, report);
    }
}

pub(super) fn validate_type_nominal_layout_id(
    program: &MirProgram,
    backend_contract: &BackendContract,
    id: TypeId,
    seen_nominals: &mut HashSet<DefId>,
    report: &mut MirAgreementReport,
) {
    if !program.type_context.type_id_tree_is_valid(id) {
        return;
    }
    let raw = program.type_context.type_for(id);
    let normalized = program
        .backend_contract
        .normalize_type(&program.type_context, &raw);
    validate_type_nominal_layout(
        &program.type_context,
        backend_contract,
        &normalized,
        seen_nominals,
        report,
    );
}

fn validate_type_nominal_layout(
    type_context: &crate::type_context::TypeContext,
    backend_contract: &BackendContract,
    ty: &Type,
    seen_nominals: &mut HashSet<DefId>,
    report: &mut MirAgreementReport,
) {
    match ty {
        Type::Struct { id, args } => {
            if seen_nominals.insert(*id) && !backend_contract.structs.contains(id) {
                report.invalid_layout_ids += 1;
            }
            for arg in args {
                if type_context.id_for_type(arg).map_or(true, |id| {
                    type_context.kind(id) == &crate::type_services::kind::Kind::Type
                }) {
                    validate_type_nominal_layout(
                        type_context,
                        backend_contract,
                        arg,
                        seen_nominals,
                        report,
                    );
                }
            }
        }
        Type::Enum { id, args } => {
            if seen_nominals.insert(*id) && !backend_contract.enums.contains(id) {
                report.invalid_layout_ids += 1;
            }
            for arg in args {
                if type_context.id_for_type(arg).map_or(true, |id| {
                    type_context.kind(id) == &crate::type_services::kind::Kind::Type
                }) {
                    validate_type_nominal_layout(
                        type_context,
                        backend_contract,
                        arg,
                        seen_nominals,
                        report,
                    );
                }
            }
        }
        Type::Reference { inner, .. } | Type::Pointer(inner) | Type::Slice(inner) => {
            validate_type_nominal_layout(
                type_context,
                backend_contract,
                inner,
                seen_nominals,
                report,
            );
        }
        Type::Array(inner, _) => {
            validate_type_nominal_layout(
                type_context,
                backend_contract,
                inner,
                seen_nominals,
                report,
            );
        }
        Type::Tuple(elems) => {
            for elem in elems {
                validate_type_nominal_layout(
                    type_context,
                    backend_contract,
                    elem,
                    seen_nominals,
                    report,
                );
            }
        }
        Type::Function {
            params,
            ret,
            captures,
            ..
        } => {
            for param in params {
                validate_type_nominal_layout(
                    type_context,
                    backend_contract,
                    param,
                    seen_nominals,
                    report,
                );
            }
            validate_type_nominal_layout(
                type_context,
                backend_contract,
                ret,
                seen_nominals,
                report,
            );
            for capture in captures {
                validate_type_nominal_layout(
                    type_context,
                    backend_contract,
                    &capture.ty,
                    seen_nominals,
                    report,
                );
            }
        }
        Type::Projection { ty, trait_args, .. } => {
            validate_type_nominal_layout(type_context, backend_contract, ty, seen_nominals, report);
            for arg in trait_args {
                validate_type_nominal_layout(
                    type_context,
                    backend_contract,
                    arg,
                    seen_nominals,
                    report,
                );
            }
        }
        Type::Apply { constructor, args } => {
            validate_type_nominal_layout(
                type_context,
                backend_contract,
                constructor,
                seen_nominals,
                report,
            );
            for arg in args {
                validate_type_nominal_layout(
                    type_context,
                    backend_contract,
                    arg,
                    seen_nominals,
                    report,
                );
            }
        }
        Type::Lambda { body, .. } => {
            validate_type_nominal_layout(
                type_context,
                backend_contract,
                body,
                seen_nominals,
                report,
            );
        }
        Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::F32
        | Type::F64
        | Type::Bool
        | Type::Str
        | Type::Char
        | Type::Unit
        | Type::Never => {}
        Type::TypeVar(_)
        | Type::Generic(_)
        | Type::Constructor { .. }
        | Type::BoundVar { .. }
        | Type::Error => {}
    }
}

pub(super) fn validate_rvalue_layout_ids(
    backend_contract: &BackendContract,
    rvalue: &Rvalue,
    report: &mut MirAgreementReport,
) {
    match rvalue {
        Rvalue::Ref(_, place) | Rvalue::Discriminant(place) => {
            validate_place_layout_ids(backend_contract, place, report);
        }
        Rvalue::Aggregate(kind, _) => validate_aggregate_layout_id(backend_contract, kind, report),
        Rvalue::Use(_)
        | Rvalue::Cast(_, _)
        | Rvalue::Closure(_)
        | Rvalue::BinaryOp(_, _, _)
        | Rvalue::UnaryOp(_, _) => {}
    }
}

pub(super) fn validate_place_layout_ids(
    backend_contract: &BackendContract,
    place: &Place,
    report: &mut MirAgreementReport,
) {
    for projection in &place.projection {
        match projection {
            Projection::Field {
                identity: Some(identity),
                ..
            } if !backend_contract.nominal_layout_known(identity.owner) => {
                report.invalid_layout_ids += 1;
            }
            Projection::Field { .. } | Projection::Deref | Projection::Index(_) => {}
            Projection::Downcast(_) => {}
        }
    }
}

fn validate_aggregate_layout_id(
    backend_contract: &BackendContract,
    kind: &AggregateKind,
    report: &mut MirAgreementReport,
) {
    match kind {
        AggregateKind::Struct { id, .. } if !backend_contract.structs.contains(id) => {
            report.invalid_layout_ids += 1;
        }
        AggregateKind::EnumVariant {
            enum_id,
            variant_id,
            ..
        } if !backend_contract.enum_variant_known(*enum_id, *variant_id) => {
            report.invalid_layout_ids += 1;
        }
        AggregateKind::Tuple | AggregateKind::Array | AggregateKind::Struct { .. } => {}
        AggregateKind::EnumVariant { .. } => {}
    }
}
