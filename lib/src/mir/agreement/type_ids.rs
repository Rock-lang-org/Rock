use crate::ids::TypeId;
use crate::mir::{Constant, MirFunction, MirProgram, Operand, Place, Rvalue};
use crate::type_context::Ty;
use crate::types::Type;

use super::*;

pub(super) fn validate_function_type_ids(
    program: &MirProgram,
    function: &MirFunction,
    report: &mut MirAgreementReport,
) {
    validate_type_id(program, function.ret_type, report);
    for local in &function.local_decls {
        validate_type_id(program, local.ty, report);
    }
}

pub(super) fn validate_type_id(program: &MirProgram, id: TypeId, report: &mut MirAgreementReport) {
    let Some(ty) = program.type_context.try_ty(id) else {
        report.invalid_type_ids += 1;
        return;
    };

    if program.type_context.kind(id) != &crate::type_services::kind::Kind::Type {
        report.invalid_type_ids += 1;
        return;
    }

    if matches!(ty, Ty::Projection { .. }) {
        let raw = program.type_context.type_for(id);
        let normalized = program
            .backend_contract
            .normalize_type(&program.type_context, &raw);
        if matches!(normalized, Type::Projection { .. }) {
            report.missing_projection_facts += 1;
        }
    }

    match ty {
        Ty::Slice(inner) | Ty::Reference { inner, .. } | Ty::Pointer(inner) => {
            validate_type_id(program, *inner, report);
        }
        Ty::Array { inner, len } => {
            // Codegen requires a concrete length; a surviving const parameter or
            // inference variable means monomorphization missed a binding.
            if !len.is_concrete() {
                report.invalid_type_ids += 1;
            }
            validate_type_id(program, *inner, report);
        }
        Ty::Tuple(elems) => {
            for elem in elems {
                validate_type_id(program, *elem, report);
            }
        }
        Ty::Function {
            params,
            ret,
            captures,
            ..
        } => {
            for param in params {
                validate_type_id(program, *param, report);
            }
            validate_type_id(program, *ret, report);
            for capture in captures {
                validate_type_id(program, capture.ty, report);
            }
        }
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => {
            for arg in args {
                if program.type_context.kind(*arg) == &crate::type_services::kind::Kind::Type {
                    validate_type_id(program, *arg, report);
                }
            }
        }
        Ty::Projection { ty, trait_args, .. } => {
            validate_type_id(program, *ty, report);
            for arg in trait_args {
                validate_type_id(program, *arg, report);
            }
        }
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
        | Ty::Never
        | Ty::ConstValue(_) => {}
        Ty::TypeVar(_)
        | Ty::Generic(_)
        | Ty::Constructor { .. }
        | Ty::Apply { .. }
        | Ty::Lambda { .. }
        | Ty::BoundVar { .. }
        | Ty::Error => report.invalid_type_ids += 1,
    }
}

pub(super) fn validate_rvalue_type_ids(
    program: &MirProgram,
    callable_contract: &CallableContract<'_>,
    function: &MirFunction,
    destination: &Place,
    rvalue: &Rvalue,
    report: &mut MirAgreementReport,
) {
    match rvalue {
        Rvalue::Use(operand) => {
            validate_runtime_operand(program, operand, true, report);
            if callable_contract.is_missing(operand) {
                report.missing_callable_metadata += 1;
            }
            if matches!(operand, Operand::Constant(Constant::Callable(_))) {
                let destination_ty = program.backend_contract.place_type_id(
                    &program.type_context,
                    function,
                    destination,
                );
                if !destination_ty.is_some_and(|ty| is_function_type_id(program, ty)) {
                    report.invalid_constant_operands += 1;
                }
            }
        }
        Rvalue::Ref(_, _) | Rvalue::Closure(_) | Rvalue::Discriminant(_) => {}
        Rvalue::Cast(operand, target_ty) => {
            validate_runtime_operand(program, operand, false, report);
            validate_type_id(program, *target_ty, report);
        }
        Rvalue::BinaryOp(_, left, right) => {
            validate_runtime_operand(program, left, false, report);
            validate_runtime_operand(program, right, false, report);
        }
        Rvalue::UnaryOp(_, operand) => validate_runtime_operand(program, operand, false, report),
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                validate_runtime_operand(program, operand, false, report);
            }
        }
    }
}

pub(super) fn validate_operand_type_ids(
    program: &MirProgram,
    operand: &Operand,
    report: &mut MirAgreementReport,
) {
    match operand {
        Operand::Constant(Constant::Callable(callable)) => {
            validate_callable_type_ids(program, callable, report);
        }
        Operand::Constant(Constant::TypeId(id)) => validate_type_id(program, *id, report),
        Operand::Copy(_) | Operand::Move(_) | Operand::Constant(_) => {}
    }
}
