mod error_reporting;
mod escape_validation;
mod initialization;
mod move_validation;
mod origin_tracking;
mod reference_escape;
mod statement_checks;

pub mod accesses;
pub mod borrows;
pub mod closures;
pub mod conflicts;
pub mod diagnostics;
pub mod liveness;
pub mod location;
pub mod paths;
pub mod provenance;

use std::collections::HashMap;

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::lexer::Span;
use crate::mir::{MirBackendContract, MirFunction, MirFunctionId, MirProgram};
use crate::type_services::display::TypeDisplayContext;

use move_validation::*;
use origin_tracking::*;
use reference_escape::*;

fn mir_diagnostic(message: String, span: Option<Span>) -> Diagnostic {
    match span {
        Some(span) => Diagnostic::new(message, span).with_code(DiagnosticCode::Borrow),
        None => Diagnostic::for_internal(message),
    }
}

pub struct BorrowChecker;

impl BorrowChecker {
    /// Checks every function of `program`; `type_names` names nominal types
    /// in diagnostics.
    pub fn run(program: &MirProgram, type_names: &TypeDisplayContext) -> Result<(), Diagnostics> {
        let mut diagnostics = Diagnostics::default();
        let return_summaries = Self::reference_return_summaries(program);

        for (_, func) in program.functions() {
            if let Err(func_diagnostics) = Self::check_function_with_contract(
                func,
                &program.type_context,
                &program.backend_contract,
                &return_summaries,
                type_names,
            ) {
                for diag in func_diagnostics.0 {
                    diagnostics.push(diag);
                }
            }
        }

        if diagnostics.0.is_empty() {
            Ok(())
        } else {
            Err(diagnostics)
        }
    }

    fn check_function_with_contract(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        type_names: &TypeDisplayContext,
    ) -> Result<(), Diagnostics> {
        let mut checker = BorrowChecker;
        checker.validate_ownership_core(
            func,
            type_context,
            backend_contract,
            return_summaries,
            type_names,
        )
    }

    pub fn validate_ownership_core(
        &mut self,
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        type_names: &TypeDisplayContext,
    ) -> Result<(), Diagnostics> {
        let mut diagnostics = Self::validate_moves_and_initialization(
            func,
            type_context,
            backend_contract,
            return_summaries,
            type_names,
        );
        Self::validate_reference_escapes(
            func,
            type_context,
            backend_contract,
            return_summaries,
            &mut diagnostics,
        );
        Self::validate_drop_obligations(func, backend_contract, &mut diagnostics);

        if diagnostics.0.is_empty() {
            Ok(())
        } else {
            Err(diagnostics)
        }
    }
}

#[cfg(test)]
mod tests;
