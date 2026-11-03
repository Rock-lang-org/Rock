use crate::diagnostic::Diagnostics;
use crate::lexer::Span;
use crate::mir::borrowck::accesses::AccessKind;
use crate::mir::borrowck::closures::CaptureKind;
use crate::mir::borrowck::liveness::LoanLocationIndex;
use crate::mir::dataflow::analyses::{
    InitMap, InitializationAnalysis, LoanAnalysis, LoanKind, LoanState, LoanTable,
};
use crate::mir::{
    MirFunction, Mutability, Operand, Rvalue, StatementData, StatementKind, Terminator,
};

use super::*;

impl BorrowChecker {
    pub(super) fn check_created_reborrows(
        location: crate::mir::borrowck::location::Location,
        locations: &LoanLocationIndex,
        table: &LoanTable,
        active: &LoanState,
        diagnostics: &mut Diagnostics,
        span: Option<Span>,
    ) {
        for id in locations.loan_ids_at(location) {
            let Some(loan) = table.get(id) else {
                continue;
            };
            if let Some(through) = &loan.reborrow_source {
                if let Err(conflict) = LoanAnalysis::check_reborrow(
                    &loan.diagnostic_place,
                    loan.kind,
                    through.local,
                    table,
                    active,
                ) {
                    diagnostics.push(crate::mir::borrowck::diagnostics::borrow_conflict(
                        &loan.diagnostic_place,
                        span.clone(),
                        table
                            .get(conflict)
                            .and_then(|loan| loan.origin_span.clone()),
                    ));
                }
            }
        }
    }

    pub(super) fn check_statement(
        stmt: &StatementData,
        init_analysis: &InitializationAnalysis,
        state: &InitMap,
        func: &MirFunction,
        move_validation: &mut MoveValidationContext<'_>,
        diagnostics: &mut Diagnostics,
    ) {
        let stmt_span = stmt.span.clone();

        match &stmt.kind {
            StatementKind::Assign(_dest, rvalue) => match rvalue {
                Rvalue::Use(operand) => {
                    match operand {
                        Operand::Move(place) => move_validation.validate_move(
                            place,
                            func,
                            stmt_span.clone(),
                            diagnostics,
                        ),
                        Operand::Copy(place) => move_validation.validate_copy(
                            place,
                            func,
                            stmt_span.clone(),
                            diagnostics,
                        ),
                        Operand::Constant(_) => {}
                    }
                    if let Err(e) = init_analysis.check_operand_at_path(operand, state) {
                        diagnostics.push(Self::make_error(e, func, operand, stmt_span));
                    }
                }
                Rvalue::Ref(_mutability, place) => {
                    if let Err(e) = init_analysis.check_place_at_path(place, state) {
                        diagnostics.push(Self::make_error(
                            e,
                            func,
                            &Operand::Copy(place.clone()),
                            stmt_span,
                        ));
                    }
                }
                Rvalue::Cast(op, _) => {
                    if let Err(e) = init_analysis.check_operand_at_path(op, state) {
                        diagnostics.push(Self::make_error(e, func, op, stmt_span));
                    }
                }
                Rvalue::BinaryOp(_, a, b) => {
                    if let Err(e) = init_analysis.check_operand_at_path(a, state) {
                        diagnostics.push(Self::make_error(e, func, a, stmt_span.clone()));
                    }
                    if let Err(e) = init_analysis.check_operand_at_path(b, state) {
                        diagnostics.push(Self::make_error(e, func, b, stmt_span));
                    }
                }
                Rvalue::UnaryOp(_, a) => {
                    if let Err(e) = init_analysis.check_operand_at_path(a, state) {
                        diagnostics.push(Self::make_error(e, func, a, stmt_span));
                    }
                }
                Rvalue::Discriminant(place) => {
                    // Structural enum cleanup reads discriminants behind drop flags.
                    // Initialization state alone cannot prove those drop-flag branches
                    // unreachable after a move, so only skip explicitly marked cleanup reads.
                    if !stmt.cleanup {
                        if let Err(e) = init_analysis.check_discriminant_at_path(place, state) {
                            diagnostics.push(Self::make_error(
                                e,
                                func,
                                &Operand::Copy(place.clone()),
                                stmt_span,
                            ));
                        }
                    }
                }
                Rvalue::Aggregate(_, operands) => {
                    for op in operands {
                        if let Err(e) = init_analysis.check_operand_at_path(op, state) {
                            diagnostics.push(Self::make_error(e, func, op, stmt_span.clone()));
                        }
                    }
                }
                Rvalue::Closure(closure) => {
                    for capture in &closure.captures {
                        if matches!(CaptureKind::from(capture.kind), CaptureKind::Move) {
                            continue;
                        }
                        let capture_operand = Operand::Copy(capture.place());
                        if let Err(e) = init_analysis.check_operand_at_path(&capture_operand, state)
                        {
                            diagnostics.push(Self::make_error(
                                e,
                                func,
                                &capture_operand,
                                stmt_span.clone(),
                            ));
                        }
                    }
                }
            },
            StatementKind::Assert(assertion) => {
                for operand in &assertion.operands {
                    if let Err(e) = init_analysis.check_operand_at_path(operand, state) {
                        diagnostics.push(Self::make_error(e, func, operand, stmt_span.clone()));
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn check_statement_loans(
        stmt: &StatementData,
        type_context: &crate::type_context::TypeContext,
        table: &LoanTable,
        active_loans: &LoanState,
        func: &MirFunction,
        diagnostics: &mut Diagnostics,
    ) {
        let assign_rvalue = if let StatementKind::Assign(dest, rvalue) = &stmt.kind {
            if !matches!(rvalue, Rvalue::Ref(_, _)) {
                Self::check_place_loan_access(
                    dest,
                    LoanKind::Mut,
                    table,
                    active_loans,
                    func,
                    diagnostics,
                    stmt.span.clone(),
                );
            }

            Some(rvalue)
        } else {
            None
        };

        for event in accesses::classify_statement(stmt, type_context) {
            match event.kind {
                AccessKind::BorrowShared => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Shared,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        stmt.span.clone(),
                    );
                }
                AccessKind::BorrowMut => {
                    if !event
                        .place
                        .projection
                        .iter()
                        .any(|projection| matches!(projection, crate::mir::Projection::Deref))
                        && event.place.local.0 < func.local_decls.len()
                        && func.local_decls[event.place.local.0]
                            .source
                            .reports_immutable_mut_borrow()
                        && func.local_decls[event.place.local.0].mutability == Mutability::Not
                    {
                        let name = func.local_decls[event.place.local.0]
                            .name
                            .clone()
                            .unwrap_or_else(|| "unnamed binding".to_string());
                        diagnostics.push(mir_diagnostic(
                            format!(
                                "Cannot take a mutable reference to immutable binding '{}' for mutable receiver",
                                name
                            ),
                            stmt.span.clone(),
                        ));
                    }

                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Mut,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        stmt.span.clone(),
                    );
                }
                AccessKind::Read => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Shared,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        stmt.span.clone(),
                    );
                }
                AccessKind::Move => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Mut,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        stmt.span.clone(),
                    );
                }
                AccessKind::CaptureShared => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Shared,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        stmt.span.clone(),
                    );
                }
                AccessKind::CaptureMut => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Mut,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        stmt.span.clone(),
                    );
                }
                AccessKind::RawPointerCast => {
                    if let Some(Rvalue::Cast(op, _)) = assign_rvalue {
                        match op {
                            Operand::Copy(place) => Self::check_place_loan_access(
                                place,
                                LoanKind::Shared,
                                table,
                                active_loans,
                                func,
                                diagnostics,
                                stmt.span.clone(),
                            ),
                            Operand::Move(place) => Self::check_place_loan_access(
                                place,
                                LoanKind::Mut,
                                table,
                                active_loans,
                                func,
                                diagnostics,
                                stmt.span.clone(),
                            ),
                            Operand::Constant(_) => {}
                        }
                    }
                }
                AccessKind::CaptureMove | AccessKind::Write | AccessKind::Drop => {}
            }
        }
    }

    pub(super) fn check_terminator(
        term: &Terminator,
        init_analysis: &InitializationAnalysis,
        state: &InitMap,
        table: &LoanTable,
        active_loans: &LoanState,
        func: &MirFunction,
        diagnostics: &mut Diagnostics,
    ) {
        let term_span = term.origin().source_span().cloned();
        for event in accesses::classify_terminator(term) {
            match event.kind {
                AccessKind::Read => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Shared,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        term_span.clone(),
                    );
                }
                AccessKind::Move | AccessKind::Drop => {
                    Self::check_place_loan_access(
                        &event.place,
                        LoanKind::Mut,
                        table,
                        active_loans,
                        func,
                        diagnostics,
                        term_span.clone(),
                    );
                }
                AccessKind::Write
                | AccessKind::BorrowShared
                | AccessKind::BorrowMut
                | AccessKind::CaptureShared
                | AccessKind::CaptureMut
                | AccessKind::CaptureMove
                | AccessKind::RawPointerCast => {}
            }
        }

        match term {
            Terminator::SwitchInt { discr, .. } | Terminator::SwitchIntWithOrigin { discr, .. } => {
                if let Err(e) = init_analysis.check_operand_at_path(discr, state) {
                    diagnostics.push(Self::make_error(e, func, discr, term_span.clone()));
                }
            }
            Terminator::Call {
                func: op_func,
                args,
                ..
            } => {
                if let Err(e) = init_analysis.check_operand_at_path(op_func, state) {
                    diagnostics.push(Self::make_error(e, func, op_func, term_span.clone()));
                }
                for arg in args {
                    if let Err(e) = init_analysis.check_operand_at_path(arg, state) {
                        diagnostics.push(Self::make_error(e, func, arg, term_span.clone()));
                    }
                }
            }
            Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } => {
                if let Err(e) = init_analysis.check_drop_at_path(place, state) {
                    diagnostics.push(Self::make_error(
                        e,
                        func,
                        &Operand::Copy(place.clone()),
                        term_span,
                    ));
                }
            }
            _ => {}
        }
    }

    fn check_place_loan_access(
        place: &crate::mir::Place,
        kind: LoanKind,
        table: &LoanTable,
        active_loans: &LoanState,
        func: &MirFunction,
        diagnostics: &mut Diagnostics,
        stmt_span: Option<Span>,
    ) {
        Self::check_place_loan_access_through(
            place,
            kind,
            place,
            table,
            active_loans,
            func,
            diagnostics,
            stmt_span,
        );
    }

    fn check_place_loan_access_through(
        place: &crate::mir::Place,
        kind: LoanKind,
        through: &crate::mir::Place,
        table: &LoanTable,
        active_loans: &LoanState,
        _func: &MirFunction,
        diagnostics: &mut Diagnostics,
        stmt_span: Option<Span>,
    ) {
        if let Err(conflicting_loan_id) =
            LoanAnalysis::check_aliasing_through(place, kind, through, table, active_loans)
        {
            let use_span = stmt_span;
            let borrow_span = table
                .get(conflicting_loan_id)
                .and_then(|loan| loan.origin_span.clone());
            diagnostics.push(crate::mir::borrowck::diagnostics::borrow_conflict(
                place,
                use_span,
                borrow_span,
            ));
        }
    }
}
