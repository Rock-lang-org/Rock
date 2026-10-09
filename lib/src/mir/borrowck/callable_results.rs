//! Check callable result obligations against actual native bodies and allocations.
//! Binding certificates may supply result origins, never effects or layout permission.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::{
    mir_diagnostic, type_id_tracks_origin, BorrowChecker, MissingSummaryMode, OriginPurpose,
    OriginSet, ReferenceEscapeAnalysis,
};
use crate::diagnostic::Diagnostics;
use crate::mir::dataflow::run_fixpoint;
use crate::mir::{
    Local, LocalSource, MirFunction, MirFunctionId, MirProgram, Place, ReferenceOrigin, Rvalue,
    StatementKind, Terminator,
};
use crate::type_context::{Ty, TypeView};
use crate::types::{CallableResultBorrowing, CallableResultSource};

/// Translate measured body origins, not abstract capture metadata, to a portable
/// envelope. Local invocation storage and missing summaries remain unknown.
#[cfg(test)]
fn body_sources(
    program: &MirProgram,
    body: &MirFunction,
    lifetime: &HashMap<MirFunctionId, OriginSet>,
    values: &HashMap<MirFunctionId, OriginSet>,
) -> Option<CallableResultBorrowing> {
    body_sources_with_calls(program, body, lifetime, values, None)
}

pub(super) fn body_sources_with_calls(
    program: &MirProgram,
    body: &MirFunction,
    lifetime: &HashMap<MirFunctionId, OriginSet>,
    values: &HashMap<MirFunctionId, OriginSet>,
    calls: Option<&super::return_sources::NativeReturnSources>,
) -> Option<CallableResultBorrowing> {
    let tc = &program.type_context;
    if !BorrowChecker::body_available_for_purpose(
        program,
        &body.id,
        body,
        OriginPurpose::ValueLoans,
    ) || !tc.type_id_tree_is_valid(body.ret_type)
    {
        return None;
    }
    let declared = body
        .local_decls
        .iter()
        .enumerate()
        .filter_map(|(index, local)| {
            (local.source == LocalSource::ClosureCapture).then_some(Local(index))
        })
        .collect::<HashSet<_>>();
    let captures = body
        .closure_captures
        .iter()
        .map(|capture| capture.local)
        .collect::<HashSet<_>>();
    if declared != captures || captures.len() != body.closure_captures.len() {
        return None;
    }
    if body
        .local_decls
        .iter()
        .skip(1)
        .take(body.arg_count)
        .any(|local| local.source != LocalSource::Argument || !tc.type_id_tree_is_valid(local.ty))
        || captures
            .iter()
            .any(|local| !tc.type_id_tree_is_valid(body.local_decls[local.0].ty))
    {
        return None;
    }
    let analysis = ReferenceEscapeAnalysis::new(
        body,
        tc,
        &program.backend_contract,
        lifetime,
        MissingSummaryMode::Defer,
    )
    .with_return_sources(calls);
    let results = run_fixpoint(&analysis, body);
    if !body.basic_blocks.iter().enumerate().any(|(index, block)| {
        results.reachable[index]
            && matches!(
                block.terminator,
                Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
            )
    }) {
        return None;
    }
    let tracked =
        type_id_tracks_origin(TypeView::new(tc), &program.backend_contract, body.ret_type);
    // Scalar/owned shape cannot substitute for measurement. Analyze those
    // actual bodies with dependency tracking on every value, preserving opaque
    // calls, carried inputs, casts and invocation-local storage. No empty or
    // missing summary becomes independence/effect authority.
    let measured = |summaries, purpose| {
        let mut analysis = ReferenceEscapeAnalysis::new(
            body,
            tc,
            &program.backend_contract,
            summaries,
            MissingSummaryMode::Conservative,
        )
        .with_return_sources(calls);
        analysis.purpose = purpose;
        analysis.track_value_dependencies = true;
        analysis.measured_return_origins()
    };
    let (measured_lifetime, measured_values);
    let (lifetime, values) = if tracked {
        (lifetime.get(&body.id)?, values.get(&body.id)?)
    } else {
        measured_lifetime = measured(lifetime, OriginPurpose::Lifetime)?;
        measured_values = measured(values, OriginPurpose::ValueLoans)?;
        (&measured_lifetime, &measured_values)
    };
    if lifetime.iter().chain(values).any(|origin| match origin {
        ReferenceOrigin::Local(local) => !captures.contains(local),
        ReferenceOrigin::Temporary(_) => true,
        _ => false,
    }) {
        // Invocation-local storage is not external storage and cannot be
        // weakened into UnknownExternal when a certificate is instantiated.
        return None;
    }
    let sources = lifetime
        .iter()
        .chain(values)
        .map(|origin| {
            use CallableResultSource as S;
            let parameter = |local: Local| {
                let index = local.0.checked_sub(1)?;
                (index < body.arg_count
                    && body.local_decls.get(local.0)?.source == LocalSource::Argument)
                    .then(|| u32::try_from(index).ok())
                    .flatten()
            };
            match *origin {
                ReferenceOrigin::Param(local) => {
                    parameter(local)
                        .map(S::ArgumentValue)
                        .unwrap_or(if captures.contains(&local) {
                            S::EnvironmentValue
                        } else {
                            S::Unknown
                        })
                }
                ReferenceOrigin::ParamContents(local) => parameter(local)
                    .map(S::ArgumentContents)
                    .unwrap_or(if captures.contains(&local) {
                        S::EnvironmentContents
                    } else {
                        S::Unknown
                    }),
                ReferenceOrigin::ParamBorrowedContents(local) => parameter(local)
                    .map(S::ArgumentBorrowedContents)
                    .unwrap_or(if captures.contains(&local) {
                        S::EnvironmentBorrowedContents
                    } else {
                        S::Unknown
                    }),
                ReferenceOrigin::Local(local) if captures.contains(&local) => S::EnvironmentStorage,
                ReferenceOrigin::Static => S::Static,
                ReferenceOrigin::Local(_)
                | ReferenceOrigin::Temporary(_)
                | ReferenceOrigin::UnknownExternal => S::Unknown,
            }
        })
        .collect::<BTreeSet<_>>();
    let result = CallableResultBorrowing::Sources(Box::new(sources.into_iter().collect()));
    result
        .sources_are_valid(body.arg_count, |index| {
            matches!(
                tc.try_ty(body.local_decls[index as usize + 1].ty),
                Some(Ty::Reference { .. } | Ty::Pointer(_))
            )
        })
        .then_some(result)
}

fn allocation_body<'a>(
    program: &'a MirProgram,
    owner: &MirFunction,
    destination: &Place,
    closure: &crate::mir::MirClosure,
) -> Option<&'a MirFunction> {
    if closure.id.owner != owner.id {
        return None;
    }
    let ty = program
        .backend_contract
        .place_type_id(&program.type_context, owner, destination)?;
    let Ty::Function { params, ret, .. } = program.type_context.try_ty(ty)? else {
        return None;
    };
    let id = MirFunctionId::Closure(Box::new(closure.id.clone()));
    let body = program.functions.get(&id)?;
    if body.id != id
        || *ret != body.ret_type
        || params.len() != body.arg_count
        || !params.iter().enumerate().all(|(index, ty)| {
            body.local_decls
                .get(index + 1)
                .is_some_and(|local| local.ty == *ty)
        })
        || body.closure_captures.len() != closure.captures.len()
    {
        return None;
    }
    for (supplied, stored) in closure.captures.iter().zip(&body.closure_captures) {
        if supplied.kind != stored.kind {
            return None;
        }
        let source = owner.local_decls.get(supplied.local.0)?;
        let slot = body.local_decls.get(stored.local.0)?;
        if slot.source != LocalSource::ClosureCapture {
            return None;
        }
        let agrees = match supplied.kind {
            crate::mir::MirClosureCaptureKind::ByValue => slot.ty == source.ty,
            crate::mir::MirClosureCaptureKind::ByRef => {
                program.type_context.try_ty(slot.ty)
                    == Some(&Ty::Reference {
                        mutable: false,
                        inner: source.ty,
                    })
            }
            crate::mir::MirClosureCaptureKind::ByMutRef => {
                program.type_context.try_ty(slot.ty)
                    == Some(&Ty::Reference {
                        mutable: true,
                        inner: source.ty,
                    })
            }
        };
        if !agrees {
            return None;
        }
    }
    Some(body)
}

pub(super) fn validate_declarations(
    program: &MirProgram,
    lifetime: &HashMap<MirFunctionId, OriginSet>,
    values: &HashMap<MirFunctionId, OriginSet>,
    calls: Option<&super::return_sources::NativeReturnSources>,
    diagnostics: &mut Diagnostics,
) {
    bindings::validate(program, lifetime, values, calls, diagnostics);
    for (_, owner) in program.functions() {
        for block in &owner.basic_blocks {
            for statement in &block.statements {
                let StatementKind::Assign(destination, Rvalue::Closure(closure)) = &statement.kind
                else {
                    continue;
                };
                let Some(ty) = program.backend_contract.place_type_id(
                    &program.type_context,
                    owner,
                    destination,
                ) else {
                    continue;
                };
                let Some(Ty::Function {
                    result_borrowing, ..
                }) = program.type_context.try_ty(ty)
                else {
                    continue;
                };
                if *result_borrowing == CallableResultBorrowing::Unknown {
                    continue;
                }
                let actual = allocation_body(program, owner, destination, closure)
                    .and_then(|body| {
                        body_sources_with_calls(program, body, lifetime, values, calls)
                    })
                    .unwrap_or(CallableResultBorrowing::Unknown);
                if !result_borrowing.accepts(&actual) {
                    diagnostics.push(mir_diagnostic(
                        "Callable result borrowing contract is not satisfied".into(),
                        statement.span.clone(),
                    ));
                }
            }
        }
    }
}

mod bindings;

pub(super) use bindings::CertifiedCalls;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod transport_tests;
