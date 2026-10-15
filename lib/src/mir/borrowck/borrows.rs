use std::collections::{HashMap, HashSet};

use crate::lexer::Span;
use crate::mir::borrowck::location::{Location, StatementIndex};
use crate::mir::BasicBlockId;
use crate::mir::{
    Constant, Local, MirBackendContract, MirCallable, MirCallableKey, MirFunction, MirFunctionId,
    Operand, Place, Projection, ReferenceOrigin, Rvalue, StatementData,
    StatementKind, Terminator,
};
use crate::type_context::{Ty, TypeContext, TypeView};

use super::accesses::AccessKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorrowData {
    pub owner: Local,
    pub place: Place,
    pub kind: AccessKind,
    pub created_at: Location,
    pub origin_span: Option<Span>,
}

pub fn collect_statement_borrows(stmt: &StatementData, created_at: Location) -> Vec<BorrowData> {
    match &stmt.kind {
        StatementKind::Assign(dest, Rvalue::Ref(mutability, place)) => vec![BorrowData {
            owner: dest.local,
            place: place.clone(),
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
    let needs_raw_roots = func.local_decls.iter().any(|decl|
        matches!(TypeView::new(type_context).ty(decl.ty), Ty::Pointer(_))
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
                if matches!(place.projection.first(), Some(Projection::Deref))
                    && origins.local_type_is_pointer(place.local)
                {
                    let source = Place { local: place.local, projection: Vec::new() };
                    let source_origins = origins.origins_for_place(&state, &source);
                    if let Some(template) = statement_borrows.first().cloned() {
                        let replacements = source_origins.into_iter().filter_map(|origin| {
                            let (local, deref) = match origin {
                                ReferenceOrigin::Local(local) | ReferenceOrigin::Temporary(local) => (local, false),
                                ReferenceOrigin::Param(local) => (local, true),
                                _ => return None,
                            };
                            let mut borrow = template.clone();
                            borrow.place = Place { local, projection: if deref { vec![Projection::Deref] } else { Vec::new() } };
                            Some(borrow)
                        }).collect::<Vec<_>>();
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
            let mut call_borrows = collect_terminator_borrows_with_context(
                term,
                Location::new(
                    BasicBlockId(block_idx),
                    StatementIndex(block.statements.len()),
                ),
                Some((func, type_context, backend_contract, return_summaries)),
            );
            if let Terminator::Call { func: callee, args, destination, span, .. } = term {
                if call_borrows.is_empty() && args.iter().any(|arg| match arg {
                    Operand::Copy(place) | Operand::Move(place) => origins.local_type_is_pointer(place.local),
                    _ => false,
                }) {
                    if let Some(decl) = func.local_decls.get(destination.local.0) {
                        if let Ty::Reference { mutable, .. } = TypeView::new(type_context).ty(decl.ty) {
                            for origin in origins.call_return_origins(&state, callee, args, destination) {
                                let (local, deref) = match origin {
                                    ReferenceOrigin::Local(local) | ReferenceOrigin::Temporary(local) => (local, false),
                                    ReferenceOrigin::Param(local) => (local, true),
                                    _ => continue,
                                };
                                call_borrows.push(BorrowData {
                                    owner: destination.local,
                                    place: Place { local, projection: if deref { vec![Projection::Deref] } else { Vec::new() } },
                                    kind: if *mutable { AccessKind::BorrowMut } else { AccessKind::BorrowShared },
                                    created_at: Location::new(BasicBlockId(block_idx), StatementIndex(block.statements.len())),
                                    origin_span: span.clone(),
                                });
                            }
                        }
                    }
                }
            }
            borrows.extend(call_borrows);
        }
    }

    borrows
}

fn collect_terminator_borrows_with_context(
    term: &Terminator,
    created_at: Location,
    context: Option<(
        &MirFunction,
        &TypeContext,
        &MirBackendContract,
        &HashMap<MirFunctionId, HashSet<ReferenceOrigin>>,
    )>,
) -> Vec<BorrowData> {
    let term_span = context
        .as_ref()
        .and_then(|(func, _, _, _)| func.terminator_origin(term).source_span().cloned());
    let Terminator::Call {
        func,
        args,
        destination,
        ..
    } = term
    else {
        return Vec::new();
    };

    let Operand::Constant(Constant::Callable(callable)) = func else {
        return Vec::new();
    };
    {
        if let Some((mir_func, type_context, backend_contract, return_summaries)) = context {
            let Some(destination_decl) = mir_func.local_decls.get(destination.local.0) else {
                return Vec::new();
            };
            let type_view = TypeView::new(type_context);
            let Ty::Reference { mutable, .. } = type_view.ty(destination_decl.ty) else {
                return Vec::new();
            };
            if !super::type_id_contains_reference(type_view, backend_contract, destination_decl.ty)
            {
                return Vec::new();
            }

            let is_opaque = matches!(
                callable,
                MirCallable::Resolved(
                    MirCallableKey::Extern(_)
                        | MirCallableKey::Intrinsic(_)
                        | MirCallableKey::RuntimeHelper(_),
                )
            );
            let summary = callable
                .function_id()
                .and_then(|function_id| return_summaries.get(&function_id));
            let argument_indices: Vec<usize> = if is_opaque || summary.is_none() {
                (0..args.len()).collect()
            } else {
                summary
                    .into_iter()
                    .flat_map(|summary| summary.iter())
                    .filter_map(|origin| match origin {
                        ReferenceOrigin::Param(local) => local.0.checked_sub(1),
                        ReferenceOrigin::Local(_)
                        | ReferenceOrigin::Temporary(_)
                        | ReferenceOrigin::Static
                        | ReferenceOrigin::UnknownExternal => None,
                    })
                    .collect()
            };
            let kind = if *mutable {
                AccessKind::BorrowMut
            } else {
                AccessKind::BorrowShared
            };
            return argument_indices
                .into_iter()
                .filter_map(|index| {
                    args.get(index).and_then(|arg| {
                        borrow_from_reference_argument(
                            arg,
                            destination.local,
                            kind,
                            created_at,
                            mir_func,
                            type_view,
                            term_span.clone(),
                        )
                    })
                })
                .collect();
        }
        return Vec::new();
    }

}

fn borrow_from_reference_argument(
    arg: &Operand,
    owner: Local,
    kind: AccessKind,
    created_at: Location,
    func: &MirFunction,
    type_view: TypeView<'_>,
    origin_span: Option<Span>,
) -> Option<BorrowData> {
    let (Operand::Copy(source) | Operand::Move(source)) = arg else {
        return None;
    };
    let source_decl = func.local_decls.get(source.local.0)?;
    let mut source_ty = source_decl.ty;
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
