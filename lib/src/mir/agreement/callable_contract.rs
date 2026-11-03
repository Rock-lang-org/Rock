use crate::ids::TypeId;
use crate::mir::{
    Constant, MirCallable, MirCallableDecl, MirCallableKey, MirFunction, MirParamAbi, MirProgram,
    Operand,
};
use crate::type_context::{Ty, TypeView};

use super::*;

pub(super) fn validate_callable_type_ids(
    _program: &MirProgram,
    _callable: &MirCallable,
    _report: &mut MirAgreementReport,
) {
}

pub(super) fn validate_call_argument_metadata(
    program: &MirProgram,
    callable_contract: &CallableContract<'_>,
    function: &MirFunction,
    func: &Operand,
    args: &[Operand],
    report: &mut MirAgreementReport,
) {
    if let Some(params) = callable_contract.params_for_operand(func) {
        for (arg, param) in args.iter().zip(params) {
            if is_function_type_id(program, param.semantic_ty) && callable_contract.is_missing(arg)
            {
                report.missing_callable_metadata += 1;
            }
        }
        return;
    }

    let Some(params) = function_operand_params(program, function, func) else {
        return;
    };
    for (arg, param) in args.iter().zip(params) {
        if is_function_type_id(program, *param) && callable_contract.is_missing(arg) {
            report.missing_callable_metadata += 1;
        }
    }
}

pub(super) fn function_operand_params<'a>(
    program: &'a MirProgram,
    function: &MirFunction,
    operand: &Operand,
) -> Option<&'a [TypeId]> {
    let local = match operand {
        Operand::Copy(place) | Operand::Move(place) if place.projection.is_empty() => place.local,
        _ => return None,
    };
    let local_ty = function.local_decls.get(local.0)?.ty;
    match program.type_context.try_ty(local_ty)? {
        Ty::Function { params, .. } => Some(params.as_slice()),
        _ => None,
    }
}

pub(super) fn is_function_type_id(program: &MirProgram, id: TypeId) -> bool {
    matches!(program.type_context.try_ty(id), Some(Ty::Function { .. }))
}

pub(super) struct CallableContract<'a> {
    callables: &'a std::collections::BTreeMap<MirCallableKey, MirCallableDecl>,
}

impl<'a> CallableContract<'a> {
    pub(super) fn new(program: &'a MirProgram) -> Self {
        Self {
            callables: &program.backend_contract.callables,
        }
    }

    pub(super) fn params_for_operand(&self, operand: &Operand) -> Option<&[MirParamAbi]> {
        match operand {
            Operand::Constant(Constant::Callable(callable)) => self.params_for_callable(callable),
            _ => None,
        }
    }

    fn params_for_callable(&self, callable: &MirCallable) -> Option<&[MirParamAbi]> {
        let key = Self::callable_key(callable);
        self.callables
            .get(&key)
            .map(|declaration| declaration.signature.params.as_slice())
    }

    pub(super) fn is_missing(&self, operand: &Operand) -> bool {
        match operand {
            Operand::Constant(Constant::Callable(callable)) => self.callable_is_missing(callable),
            _ => false,
        }
    }

    fn callable_is_missing(&self, callable: &MirCallable) -> bool {
        !self.callables.contains_key(Self::callable_key(callable))
    }

    fn callable_key(callable: &MirCallable) -> &MirCallableKey {
        let MirCallable::Resolved(key) = callable;
        key
    }
}

pub(super) fn is_unit_callable_operand(
    type_view: TypeView<'_>,
    function: &crate::mir::MirFunction,
    operand: &Operand,
) -> bool {
    match operand {
        Operand::Constant(Constant::Unit) => true,
        Operand::Copy(place) | Operand::Move(place) => {
            place.projection.is_empty()
                && function
                    .local_decls
                    .get(place.local.0)
                    .is_some_and(|local| matches!(type_view.try_ty(local.ty), Some(Ty::Unit)))
        }
        _ => false,
    }
}
