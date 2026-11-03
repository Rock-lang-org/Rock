mod backend_contract;
mod callable_contract;
mod layout_ids;
mod operands;
mod type_ids;

use std::collections::HashSet;

use crate::mir::{
    Constant, MirAssertKind, MirBackendContractError, MirProgram, Operand, Rvalue, StatementKind,
    Terminator,
};
use crate::type_context::{Ty, TypeView};

use backend_contract::*;
use callable_contract::*;
use layout_ids::*;
use operands::*;
use type_ids::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MirAgreementReport {
    pub non_unit_placeholder_units: usize,
    pub malformed_runtime_requirements: usize,
    pub missing_runtime_requirements: usize,
    pub stale_runtime_requirements: usize,
    pub invalid_type_ids: usize,
    pub invalid_constant_operands: usize,
    pub missing_callable_metadata: usize,
    pub invalid_layout_ids: usize,
    pub missing_projection_facts: usize,
    pub stale_projection_facts: usize,
    pub invalid_backend_contract: usize,
    pub backend_contract_errors: Vec<MirBackendContractError>,
}

impl MirAgreementReport {
    pub fn is_clean(&self) -> bool {
        self.non_unit_placeholder_units == 0
            && self.malformed_runtime_requirements == 0
            && self.missing_runtime_requirements == 0
            && self.stale_runtime_requirements == 0
            && self.invalid_type_ids == 0
            && self.invalid_constant_operands == 0
            && self.missing_callable_metadata == 0
            && self.invalid_layout_ids == 0
            && self.missing_projection_facts == 0
            && self.stale_projection_facts == 0
            && self.invalid_backend_contract == 0
    }
}

pub fn check_mir_runtime_agreement(program: &MirProgram) -> MirAgreementReport {
    let mut report = MirAgreementReport::default();
    let type_view = TypeView::new(&program.type_context);
    let callable_contract = CallableContract::new(program);
    let backend_contract = BackendContract::new(program);
    let mut seen_nominal_layout_ids = HashSet::new();

    validate_backend_contract_type_ids(program, &mut report);
    validate_backend_contract_projection_traits(program, &backend_contract, &mut report);
    validate_mir_backend_contract(program, &mut report);
    validate_runtime_requirements(program, &mut report);
    validate_backend_contract_nominal_layout_ids(
        program,
        &backend_contract,
        &mut seen_nominal_layout_ids,
        &mut report,
    );

    for (_, function) in program.functions() {
        validate_function_type_ids(program, function, &mut report);
        validate_function_nominal_layout_ids(
            program,
            &backend_contract,
            function,
            &mut seen_nominal_layout_ids,
            &mut report,
        );

        for block in &function.basic_blocks {
            for statement in &block.statements {
                match &statement.kind {
                    StatementKind::Assign(place, rvalue) => {
                        validate_place_layout_ids(&backend_contract, place, &mut report);
                        validate_rvalue_type_ids(
                            program,
                            &callable_contract,
                            function,
                            place,
                            rvalue,
                            &mut report,
                        );
                        validate_rvalue_layout_ids(&backend_contract, rvalue, &mut report);
                        let Some(local_decl) = function.local_decls.get(place.local.0) else {
                            continue;
                        };
                        if program.type_context.contains_type_id(local_decl.ty)
                            && place.projection.is_empty()
                            && !matches!(type_view.ty(local_decl.ty), Ty::Unit)
                            && matches!(rvalue, Rvalue::Use(Operand::Constant(Constant::Unit)))
                        {
                            report.non_unit_placeholder_units += 1;
                        }
                    }
                    StatementKind::Assert(assertion) => {
                        for operand in &assertion.operands {
                            validate_runtime_operand(program, operand, false, &mut report);
                        }
                        match assertion.kind {
                            MirAssertKind::BoundsCheck if assertion.operands.len() != 2 => {
                                report.malformed_runtime_requirements += 1;
                            }
                            MirAssertKind::BoundsCheck => {}
                        }
                    }
                    _ => {}
                }
            }

            match &block.terminator {
                Some(Terminator::Call { func, args, .. }) => {
                    validate_runtime_operand(program, func, true, &mut report);
                    if callable_contract.is_missing(func) {
                        report.missing_callable_metadata += 1;
                    }
                    validate_call_argument_metadata(
                        program,
                        &callable_contract,
                        function,
                        func,
                        args,
                        &mut report,
                    );
                    validate_call_constant_operands(
                        program,
                        &callable_contract,
                        function,
                        func,
                        args,
                        &mut report,
                    );
                    if is_unit_callable_operand(type_view, function, func) {
                        report.non_unit_placeholder_units += 1;
                    }
                }
                Some(Terminator::SwitchInt { discr, .. })
                | Some(Terminator::SwitchIntWithOrigin { discr, .. }) => {
                    validate_runtime_operand(program, discr, false, &mut report);
                }
                Some(Terminator::Drop { place, .. })
                | Some(Terminator::DropWithOrigin { place, .. }) => {
                    validate_place_layout_ids(&backend_contract, place, &mut report);
                }
                _ => {}
            }
        }
    }

    report
}

fn validate_runtime_requirements(program: &MirProgram, report: &mut MirAgreementReport) {
    let observed = crate::mir::backend_contract::runtime_requirements_for_functions(
        program.functions.values(),
    );

    report.missing_runtime_requirements = observed
        .difference(&program.backend_contract.runtime_requirements)
        .count();
    report.stale_runtime_requirements = program
        .backend_contract
        .runtime_requirements
        .difference(&observed)
        .count();
}

#[cfg(test)]
mod tests;
