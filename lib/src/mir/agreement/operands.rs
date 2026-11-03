use crate::ids::TypeId;
use crate::mir::{Constant, MirCallable, MirCallableKey, MirFunction, MirProgram, Operand};
use crate::type_context::Ty;

use super::*;

pub(super) fn validate_runtime_operand(
    program: &MirProgram,
    operand: &Operand,
    allow_callable: bool,
    report: &mut MirAgreementReport,
) {
    validate_operand_type_ids(program, operand, report);
    if matches!(operand, Operand::Constant(Constant::TypeId(_)))
        || (!allow_callable && matches!(operand, Operand::Constant(Constant::Callable(_))))
    {
        report.invalid_constant_operands += 1;
    }
}

pub(super) fn validate_call_constant_operands(
    program: &MirProgram,
    callable_contract: &CallableContract<'_>,
    function: &MirFunction,
    func: &Operand,
    args: &[Operand],
    report: &mut MirAgreementReport,
) {
    let metadata_intrinsic = match func {
        Operand::Constant(Constant::Callable(MirCallable::Resolved(
            MirCallableKey::Intrinsic(intrinsic),
        ))) if intrinsic.is_type_only() => Some(intrinsic),
        _ => None,
    };
    let contract_params = callable_contract.params_for_operand(func);
    let indirect_params = function_operand_params(program, function, func);

    let metadata_args_valid = match metadata_intrinsic {
        Some(crate::mir::MirIntrinsicId::SizeOf) => {
            args.len() == 1 && matches!(args[0], Operand::Constant(Constant::TypeId(_)))
        }
        Some(crate::mir::MirIntrinsicId::ArrayLen) => {
            array_len_args_are_valid(program, function, args)
        }
        Some(_) => unreachable!("is_type_only returned true for an unhandled MIR intrinsic"),
        None => true,
    };
    if !metadata_args_valid {
        report.invalid_constant_operands += 1;
    }

    for (index, arg) in args.iter().enumerate() {
        validate_operand_type_ids(program, arg, report);
        if metadata_intrinsic.is_none() && matches!(arg, Operand::Constant(Constant::TypeId(_))) {
            report.invalid_constant_operands += 1;
        }
        if matches!(arg, Operand::Constant(Constant::Callable(_))) {
            let expected_ty = contract_params
                .and_then(|params| params.get(index).map(|param| param.semantic_ty))
                .or_else(|| indirect_params.and_then(|params| params.get(index).copied()));
            if !expected_ty.is_some_and(|ty| is_function_type_id(program, ty)) {
                report.invalid_constant_operands += 1;
            }
        }
    }
}

fn array_len_args_are_valid(
    program: &MirProgram,
    function: &MirFunction,
    args: &[Operand],
) -> bool {
    let [arg] = args else {
        return false;
    };
    match arg {
        Operand::Constant(Constant::TypeId(id)) => {
            matches!(program.type_context.try_ty(*id), Some(Ty::Array { .. }))
        }
        Operand::Copy(place) | Operand::Move(place) => program
            .backend_contract
            .place_type_id(&program.type_context, function, place)
            .is_some_and(|id| array_len_runtime_type_is_supported(program, id)),
        Operand::Constant(_) => false,
    }
}

fn array_len_runtime_type_is_supported(program: &MirProgram, id: TypeId) -> bool {
    match program.type_context.try_ty(id) {
        Some(Ty::Array { .. } | Ty::Slice(_)) => true,
        Some(Ty::Reference { inner, .. }) => matches!(
            program.type_context.try_ty(*inner),
            Some(Ty::Array { .. } | Ty::Slice(_) | Ty::Str)
        ),
        _ => false,
    }
}
