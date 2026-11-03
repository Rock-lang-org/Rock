use std::collections::HashMap;

use crate::diagnostic::Diagnostics;
use crate::lexer::Span;
use crate::mir::dataflow::{run_fixpoint, Analysis};
use crate::mir::{
    Local, MirBackendContract, MirFunction, MirFunctionId, MirProgram, Place, ReferenceOrigin,
    StatementKind, Terminator,
};
use crate::type_context::TypeView;

use super::*;

impl BorrowChecker {
    pub(super) fn reference_return_summaries(
        program: &MirProgram,
    ) -> HashMap<MirFunctionId, OriginSet> {
        let mut summaries = HashMap::new();

        loop {
            let previous = summaries.clone();
            let mut next = HashMap::new();
            for (id, func) in program.functions() {
                if !type_id_tracks_origin(
                    TypeView::new(&program.type_context),
                    &program.backend_contract,
                    func.ret_type,
                ) {
                    continue;
                }

                let origins = Self::reference_return_origins(
                    func,
                    &program.type_context,
                    &program.backend_contract,
                    &previous,
                );
                if !origins.is_empty() {
                    next.insert(id.clone(), origins);
                }
            }

            if next == previous {
                return next;
            }
            summaries = next;
        }
    }

    pub(super) fn reference_return_origins(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
    ) -> OriginSet {
        let analysis = ReferenceEscapeAnalysis::new(
            func,
            type_context,
            backend_contract,
            return_summaries,
            MissingSummaryMode::Defer,
        );
        let results = run_fixpoint(&analysis, func);
        let return_place = Place {
            local: Local(0),
            projection: Vec::new(),
        };
        let mut origins = OriginSet::new();

        for (block_idx, block) in func.basic_blocks.iter().enumerate() {
            if !matches!(
                block.terminator,
                Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
            ) {
                continue;
            }
            for (place, block_origins) in &results.exit_sets[block_idx].origins {
                if ReferenceEscapeAnalysis::place_is_prefix(&return_place, place) {
                    origins.extend(block_origins.iter().copied());
                }
            }
        }

        origins
    }

    pub(super) fn validate_reference_escapes(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        diagnostics: &mut Diagnostics,
    ) {
        if !type_id_contains_reference(TypeView::new(type_context), backend_contract, func.ret_type)
            && !func.local_decls.iter().any(|decl| {
                type_id_tracks_origin(TypeView::new(type_context), backend_contract, decl.ty)
            })
        {
            return;
        }

        let analysis = ReferenceEscapeAnalysis::new(
            func,
            type_context,
            backend_contract,
            return_summaries,
            MissingSummaryMode::Conservative,
        );
        let results = run_fixpoint(&analysis, func);

        Self::validate_temporary_reference_storage(func, &analysis, &results, diagnostics);

        if !type_id_contains_reference(TypeView::new(type_context), backend_contract, func.ret_type)
        {
            return;
        }

        let return_place = Place {
            local: Local(0),
            projection: Vec::new(),
        };
        for (block_idx, block) in func.basic_blocks.iter().enumerate() {
            if !matches!(
                block.terminator,
                Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
            ) {
                continue;
            }
            for (place, origins) in &results.exit_sets[block_idx].origins {
                if !ReferenceEscapeAnalysis::place_is_prefix(&return_place, place) {
                    continue;
                }

                for origin in origins {
                    match origin {
                        ReferenceOrigin::Temporary(local) => {
                            diagnostics.push(mir_diagnostic(
                                "Cannot return reference to temporary value".to_string(),
                                Self::origin_span(func, *local),
                            ));
                        }
                        ReferenceOrigin::Local(local) => {
                            diagnostics.push(mir_diagnostic(
                                "Cannot return reference to local value because it does not live long enough"
                                    .to_string(),
                                Self::origin_span(func, *local),
                            ));
                        }
                        ReferenceOrigin::Param(_)
                        | ReferenceOrigin::Static
                        | ReferenceOrigin::UnknownExternal => {}
                    }
                }
            }
        }
    }

    fn validate_temporary_reference_storage(
        func: &MirFunction,
        analysis: &ReferenceEscapeAnalysis<'_>,
        results: &crate::mir::dataflow::Results<ReferenceOriginState>,
        diagnostics: &mut Diagnostics,
    ) {
        for (block_idx, block) in func.basic_blocks.iter().enumerate() {
            if !results.reachable[block_idx] {
                continue;
            }
            let mut state = results.entry_sets[block_idx].clone();
            for stmt in &block.statements {
                if let StatementKind::Assign(dest, rvalue) = &stmt.kind {
                    Self::report_temporary_reference_origins(
                        func,
                        diagnostics,
                        analysis.temporary_origins_for_assignment(&state, dest, rvalue),
                    );
                }
                analysis.apply_statement(&mut state, stmt);
            }

            if let Some(Terminator::Call {
                func: callable,
                args,
                destination,
                ..
            }) = &block.terminator
            {
                Self::report_temporary_reference_origins(
                    func,
                    diagnostics,
                    analysis.temporary_origins_for_call(&state, callable, args, destination),
                );
            }
        }
    }

    fn report_temporary_reference_origins(
        func: &MirFunction,
        diagnostics: &mut Diagnostics,
        temporaries: impl IntoIterator<Item = Local>,
    ) {
        for temporary in temporaries {
            diagnostics.push(mir_diagnostic(
                "Cannot store reference to temporary value because it does not live long enough"
                    .to_string(),
                Self::origin_span(func, temporary),
            ));
        }
    }

    pub(super) fn origin_span(func: &MirFunction, local: Local) -> Option<Span> {
        func.local_decls
            .get(local.0)
            .and_then(|decl| decl.span.clone())
    }
}
