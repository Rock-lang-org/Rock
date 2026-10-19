use std::collections::{HashMap, HashSet};

use crate::lexer::Span;
use crate::mir::borrowck::location::{Location, StatementIndex};
use crate::mir::BasicBlockId;
use crate::mir::{
    Local, MirBackendContract, MirFunction, MirFunctionId,
    Operand, Place, Projection, ReferenceOrigin, Rvalue, StatementData,
    StatementKind, Terminator,
};
use crate::type_context::{Ty, TypeContext, TypeView};

use super::accesses::AccessKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorrowData {
    pub owner: Local,
    pub place: Place,
    /// Pointer access or aggregate input carrying authority for recovered storage.
    pub reborrow_source: Option<Place>,
    pub kind: AccessKind,
    pub created_at: Location,
    pub origin_span: Option<Span>,
}

pub fn collect_statement_borrows(stmt: &StatementData, created_at: Location) -> Vec<BorrowData> {
    match &stmt.kind {
        StatementKind::Assign(dest, Rvalue::Ref(mutability, place)) => vec![BorrowData {
            owner: dest.local,
            place: place.clone(),
            reborrow_source: None,
            kind: AccessKind::for_borrow(*mutability),
            created_at,
            origin_span: stmt.origin().source_span().cloned(),
        }],
        StatementKind::Assign(dest, Rvalue::Closure(closure)) => closure
            .captures
            .iter()
            .filter_map(|capture| {
                let kind = match capture.kind {
                    crate::mir::MirClosureCaptureKind::ByRef => AccessKind::BorrowShared,
                    crate::mir::MirClosureCaptureKind::ByMutRef => AccessKind::BorrowMut,
                    crate::mir::MirClosureCaptureKind::ByValue => return None,
                };

                Some(BorrowData {
                    owner: dest.local,
                    place: capture.place(),
                    reborrow_source: None,
                    kind,
                    created_at,
                    origin_span: stmt.origin().source_span().cloned(),
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub fn collect_terminator_borrows(term: &Terminator, created_at: Location) -> Vec<BorrowData> {
    collect_terminator_borrows_with_context(term, created_at, None)
}

pub fn collect_function_borrows_with_contract(
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    return_summaries: &HashMap<MirFunctionId, HashSet<ReferenceOrigin>>,
) -> Vec<BorrowData> {
    let mut borrows = Vec::new();
    use crate::mir::dataflow::{run_fixpoint, Analysis};
    let origins = super::ReferenceEscapeAnalysis::new(
        func, type_context, backend_contract, return_summaries, super::MissingSummaryMode::Conservative,
    );
    let needs_raw_roots = func.local_decls.iter().enumerate().any(|(index, _)|
        origins.local_type_tracks_origin(Local(index))
    );
    let entries = needs_raw_roots.then(|| run_fixpoint(&origins, func));

    for (block_idx, block) in func.basic_blocks.iter().enumerate() {
        let mut state = entries.as_ref().map(|entries| entries.entry_sets[block_idx].clone())
            .unwrap_or_default();
        for (stmt_idx, stmt) in block.statements.iter().enumerate() {
            let mut statement_borrows = collect_statement_borrows(
                stmt,
                Location::new(BasicBlockId(block_idx), StatementIndex(stmt_idx)),
            );
            if let StatementKind::Assign(_, Rvalue::Ref(_, place)) = &stmt.kind {
                if let Some(source) = origins.raw_pointer_source(place) {
                    if let Some(template) = statement_borrows.first().cloned() {
                        let replacements = recover_raw_borrows(&origins, &state, &source, &template);
                        if !replacements.is_empty() {
                            statement_borrows = replacements;
                        }
                    }
                }
            }
            borrows.extend(statement_borrows);
            if needs_raw_roots {
                origins.apply_statement(&mut state, stmt);
            }
        }
        if let Some(term) = &block.terminator {
            let call_borrows = collect_terminator_borrows_with_context(
                term,
                Location::new(
                    BasicBlockId(block_idx),
                    StatementIndex(block.statements.len()),
                ),
                Some((&origins, &state)),
            );
            borrows.extend(call_borrows);
        }
    }

    borrows
}

fn recover_raw_borrows(
    origins: &super::ReferenceEscapeAnalysis<'_>,
    state: &super::ReferenceOriginState,
    source: &Place,
    template: &BorrowData,
) -> Vec<BorrowData> {
    let mut roots = origins.origins_for_place(state, source).into_iter().filter_map(|origin| {
        let (local, deref) = match origin {
            ReferenceOrigin::Local(local) | ReferenceOrigin::Temporary(local) => (local, false),
            ReferenceOrigin::Param(local) => (local,
                origins.local_type_is_pointer(local) || origins.local_type_is_reference(local)),
            _ => return None,
        };
        Some((local, deref))
    }).collect::<Vec<_>>();
    roots.sort_by_key(|(local, deref)| (local.0, *deref));
    roots.dedup();
    roots.into_iter().map(|(local, deref)| {
        let mut borrow = template.clone();
        borrow.place = Place { local, projection: if deref { vec![Projection::Deref] } else { Vec::new() } };
        let mut through = source.clone();
        if origins.place_type_is_pointer(source) {
            through.projection.push(Projection::Deref);
        }
        borrow.reborrow_source = Some(through);
        borrow
    }).collect()
}

fn collect_terminator_borrows_with_context(
    term: &Terminator,
    created_at: Location,
    context: Option<(&super::ReferenceEscapeAnalysis<'_>, &super::ReferenceOriginState)>,
) -> Vec<BorrowData> {
    let term_span = context
        .as_ref()
        .and_then(|(origins, _)| origins.func.terminator_origin(term).source_span().cloned());
    let Terminator::Call {
        func,
        args,
        destination,
        ..
    } = term
    else {
        return Vec::new();
    };

    let Some((origins, state)) = context else { return Vec::new(); };
    let mir_func = origins.func;
    let type_context = origins.type_context;
    let Some(destination_decl) = mir_func.local_decls.get(destination.local.0) else {
        return Vec::new();
    };
    let type_view = TypeView::new(type_context);
    if !super::type_id_contains_reference(type_view, origins.backend_contract, destination_decl.ty) {
        return Vec::new();
    }
    let kind = if super::type_id_contains_mutable_reference(type_view, origins.backend_contract, destination_decl.ty) {
        AccessKind::BorrowMut
    } else {
        AccessKind::BorrowShared
    };
    origins.call_return_argument_indices(func, args)
        .into_iter()
        .flat_map(|index| {
            let Some(arg) = args.get(index) else { return Vec::new(); };
            if let Operand::Copy(source) | Operand::Move(source) = arg {
                let source_ty = origins.backend_contract.place_type_id(type_context, mir_func, source);
                // Aggregate arguments carry a conservative origin/loan envelope.
                // Recover it at the call just as for a direct raw pointer; do not
                // invent a dereference projection on the aggregate itself.
                if source_ty.is_some_and(|ty| !matches!(type_view.ty(ty), Ty::Reference { .. })
                    && super::type_id_contains_pointer(type_view, origins.backend_contract, ty)) {
                    return recover_raw_borrows(origins, state, source, &BorrowData {
                        owner: destination.local,
                        place: source.clone(),
                        reborrow_source: None,
                        kind,
                        created_at,
                        origin_span: term_span.clone(),
                    });
                }
            }
            borrow_from_reference_argument(
                arg, destination.local, kind, created_at,
                mir_func, type_context, origins.backend_contract, term_span.clone(),
            ).into_iter().collect()
        })
        .collect()
}

fn borrow_from_reference_argument(
    arg: &Operand,
    owner: Local,
    kind: AccessKind,
    created_at: Location,
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    origin_span: Option<Span>,
) -> Option<BorrowData> {
    let (Operand::Copy(source) | Operand::Move(source)) = arg else {
        return None;
    };
    let type_view = TypeView::new(type_context);
    let mut source_ty = backend_contract.place_type_id(type_context, func, source)?;
    // A shared input stays shared even if a different returned field is mutable.
    let kind = if matches!(type_view.ty(source_ty), Ty::Reference { mutable: false, .. }) {
        AccessKind::BorrowShared
    } else {
        kind
    };
    let mut deref_count = 0;
    while let Ty::Reference { inner, .. } = type_view.ty(source_ty) {
        deref_count += 1;
        source_ty = *inner;
    }
    if deref_count == 0 {
        return None;
    }

    let mut place = source.clone();
    place
        .projection
        .extend(std::iter::repeat(Projection::Deref).take(deref_count));
    Some(BorrowData {
        owner,
        place,
        reborrow_source: None,
        kind,
        created_at,
        origin_span,
    })
}

pub fn collect_function_borrows(func: &MirFunction) -> Vec<BorrowData> {
    let mut borrows = Vec::new();

    for (block_idx, block) in func.basic_blocks.iter().enumerate() {
        for (stmt_idx, stmt) in block.statements.iter().enumerate() {
            borrows.extend(collect_statement_borrows(
                stmt,
                Location::new(BasicBlockId(block_idx), StatementIndex(stmt_idx)),
            ));
        }
        if let Some(term) = &block.terminator {
            borrows.extend(collect_terminator_borrows(
                term,
                Location::new(
                    BasicBlockId(block_idx),
                    StatementIndex(block.statements.len()),
                ),
            ));
        }
    }

    borrows
}

#[cfg(test)]
mod tests {
    use super::{collect_statement_borrows, collect_terminator_borrows, AccessKind};
    use crate::mir::borrowck::location::{Location, StatementIndex};
    use crate::mir::{
        BasicBlockId, Constant, Local, MirCallable, MirCallableKey, MirClosure,
        MirClosureCaptureKind, MirClosureId, MirFunctionId, MirIntrinsicId, Mutability, Operand,
        Place, Rvalue, StatementData, Terminator,
    };

    #[test]
    fn test_collect_borrow_from_ref_assignment() {
        let stmt = StatementData::assign(
            Place {
                local: Local(0),
                projection: vec![],
            },
            Rvalue::Ref(
                Mutability::Not,
                Place {
                    local: Local(1),
                    projection: vec![],
                },
            ),
            None,
        );

        let borrows =
            collect_statement_borrows(&stmt, Location::new(BasicBlockId(0), StatementIndex(0)));
        assert_eq!(borrows.len(), 1);
        assert_eq!(borrows[0].owner, Local(0));
        assert_eq!(borrows[0].place.local, Local(1));
        assert_eq!(borrows[0].kind, AccessKind::BorrowShared);
        assert_eq!(
            borrows[0].created_at,
            Location::new(BasicBlockId(0), StatementIndex(0))
        );
    }

    #[test]
    fn test_collect_borrow_from_shared_closure_capture() {
        let stmt = StatementData::assign(
            Place {
                local: Local(2),
                projection: vec![],
            },
            Rvalue::Closure(MirClosure {
                id: MirClosureId {
                    owner: MirFunctionId::Function(crate::ids::DefId::new(
                        crate::ids::CrateId(0),
                        crate::ids::LocalDefId(0),
                    )),
                    local_index: 0,
                },
                display_name: "lambda_0".to_string(),
                captures: vec![crate::mir::MirClosureCapture {
                    name: "x".to_string(),
                    local: Local(1),
                    kind: MirClosureCaptureKind::ByRef,
                    span: None,
                }],
            }),
            None,
        );

        let borrows =
            collect_statement_borrows(&stmt, Location::new(BasicBlockId(0), StatementIndex(0)));
        assert_eq!(borrows.len(), 1);
        assert_eq!(borrows[0].owner, Local(2));
        assert_eq!(borrows[0].place.local, Local(1));
        assert_eq!(borrows[0].kind, AccessKind::BorrowShared);
    }

    #[test]
    fn raw_wide_pointer_construction_does_not_create_a_loan() {
        let terminator = Terminator::Call {
            func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                MirCallableKey::Intrinsic(MirIntrinsicId::PtrFromParts),
            ))),
            args: vec![Operand::Copy(Place {
                local: Local(1),
                projection: vec![],
            })],
            destination: Place {
                local: Local(2),
                projection: vec![],
            },
            target: BasicBlockId(1),
            span: Some(crate::lexer::Span::test()),
        };

        let borrows = collect_terminator_borrows(
            &terminator,
            Location::new(BasicBlockId(0), StatementIndex(0)),
        );

        assert!(borrows.is_empty(), "raw pointer construction must not establish a loan");
    }
}
