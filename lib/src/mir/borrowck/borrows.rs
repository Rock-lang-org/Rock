use std::collections::{HashMap, HashSet};

use crate::lexer::Span;
use crate::mir::borrowck::location::{Location, StatementIndex};
use crate::mir::BasicBlockId;
use crate::mir::{
    Constant, Local, MirBackendContract, MirCallable, MirCallableKey, MirFunction, MirFunctionId,
    MirIntrinsicId, Operand, Place, Projection, ReferenceOrigin, Rvalue, StatementData,
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
    collect_function_borrows_with_return_sources(func, type_context, backend_contract, return_summaries, None)
}

pub(super) fn collect_function_borrows_with_return_sources(
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    return_summaries: &HashMap<MirFunctionId, HashSet<ReferenceOrigin>>,
    return_sources: Option<&super::return_sources::NativeReturnSources>,
) -> Vec<BorrowData> {
    let mut borrows = Vec::new();

    for (block_idx, block) in func.basic_blocks.iter().enumerate() {
        for (stmt_idx, stmt) in block.statements.iter().enumerate() {
            borrows.extend(collect_statement_borrows(
                stmt,
                Location::new(BasicBlockId(block_idx), StatementIndex(stmt_idx)),
            ));
        }
        if let Some(term) = &block.terminator {
            borrows.extend(collect_terminator_borrows_with_context(
                term,
                Location::new(
                    BasicBlockId(block_idx),
                    StatementIndex(block.statements.len()),
                ),
                Some((func, type_context, backend_contract, return_summaries, return_sources)),
            ));
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
        Option<&super::return_sources::NativeReturnSources>,
    )>,
) -> Vec<BorrowData> {
    let term_span = context
        .as_ref()
        .and_then(|(func, _, _, _, _)| func.terminator_origin(term).source_span().cloned());
    let Terminator::Call {
        func,
        args,
        destination,
        ..
    } = term
    else {
        return Vec::new();
    };

    let certified_callable = context.as_ref().and_then(|(body, tc, contract, _, table)|
        table.and_then(|table| table.callable_result_loan_call(body, tc, contract, func, args, destination)));
    let (args, callable_summary) = certified_callable.as_ref()
        .map(|(inputs, summary)| (inputs.as_slice(), Some(summary)))
        .unwrap_or((args.as_slice(), None));
    let portable_summary = match func {
        Operand::Constant(Constant::ErasedCall(call)) => {
            context.as_ref().and_then(|(_, tc, contract, _, _)| {
                super::erased_return_summary(contract, tc, call)
            })
        }
        _ => None,
    };
    let callable = match func {
        Operand::Constant(Constant::Callable(callable)) => Some(callable),
        _ if portable_summary.is_some() || callable_summary.is_some() => None,
        _ => return Vec::new(),
    };
    let borrow_slice = match callable {
        Some(MirCallable::Resolved(MirCallableKey::Intrinsic(MirIntrinsicId::BorrowSlice))) => {
            Some(AccessKind::BorrowShared)
        }
        Some(MirCallable::Resolved(MirCallableKey::Intrinsic(MirIntrinsicId::BorrowSliceMut))) => {
            Some(AccessKind::BorrowMut)
        }
        _ => None,
    };
    if borrow_slice.is_none() {
        if let Some((mir_func, type_context, backend_contract, return_summaries, return_sources)) = context {
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

            if return_sources.and_then(|table|
                table.borrowed_view_call(mir_func, type_context, backend_contract, func, args, destination)).is_some() {
                let mut view = destination.clone();
                view.projection.push(Projection::Deref);
                return vec![BorrowData {
                    owner: destination.local, place: view, kind: AccessKind::ReturnedShared,
                    created_at, origin_span: term_span,
                }];
            }

            let is_opaque = matches!(
                callable,
                Some(MirCallable::Resolved(
                    MirCallableKey::Extern(_)
                        | MirCallableKey::Intrinsic(_)
                        | MirCallableKey::RuntimeHelper(_),
                ))
            );
            let summary = callable_summary.or(portable_summary.as_ref()).or_else(|| {
                callable.and_then(|callable| callable.function_id())
                    .and_then(|function_id| return_summaries.get(&function_id))
            });
            let argument_indices: Vec<(usize, bool)> = if is_opaque || summary.is_none() {
                (0..args.len()).map(|index| (index, false)).collect()
            } else {
                summary
                    .into_iter()
                    .flat_map(|summary| summary.iter())
                    .filter_map(|origin| match origin {
                        ReferenceOrigin::Param(local) => {
                            local.0.checked_sub(1).map(|index| (index, false))
                        }
                        ReferenceOrigin::ParamBorrowedContents(local) => {
                            local.0.checked_sub(1).map(|index| (index, true))
                        }
                        ReferenceOrigin::ParamContents(_) => None,
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
            let mut borrows = Vec::new();
            if portable_summary.is_some() || callable_summary.is_some() {
                // The validated concrete result type supplies the view's access
                // kind; its origin envelope separately retains incoming loans.
                let mut view = destination.clone();
                view.projection.push(Projection::Deref);
                borrows.push(BorrowData {
                    owner: destination.local, place: view,
                    kind: if *mutable { AccessKind::ReturnedMut } else { AccessKind::ReturnedShared },
                    created_at,
                    origin_span: term_span.clone(),
                });
            }
            borrows.extend(argument_indices
                .into_iter()
                .filter_map(|(index, borrowed_contents)| {
                    args.get(index).and_then(|arg| {
                        borrow_from_reference_argument(
                            arg,
                            destination.local,
                            kind,
                            created_at,
                            mir_func,
                            type_context,
                            backend_contract,
                            borrowed_contents,
                            term_span.clone(),
                        )
                    })
                })
            );
            return borrows;
        }
        return Vec::new();
    }

    let Some(Operand::Copy(source) | Operand::Move(source)) = args.first() else {
        return Vec::new();
    };

    let mut root = source.clone();
    root.projection.clear();
    vec![BorrowData {
        owner: destination.local,
        place: root,
        kind: borrow_slice.unwrap(),
        created_at,
        origin_span: term_span,
    }]
}

fn borrow_from_reference_argument(
    arg: &Operand,
    owner: Local,
    kind: AccessKind,
    created_at: Location,
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    borrowed_contents: bool,
    origin_span: Option<Span>,
) -> Option<BorrowData> {
    let (Operand::Copy(source) | Operand::Move(source)) = arg else {
        return None;
    };
    let type_view = TypeView::new(type_context);
    let mut source_ty = backend_contract.place_type_id(type_context, func, source)?;
    let mut deref_count = 0;
    while let Ty::Reference { inner, .. } = type_view.ty(source_ty) {
        deref_count += 1;
        source_ty = *inner;
    }
    if deref_count == 0 && borrowed_contents && matches!(type_view.ty(source_ty), Ty::Pointer(_)) {
        deref_count = 1;
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
    fn collect_terminator_borrows_dispatches_borrow_slice_by_typed_intrinsic() {
        let terminator = Terminator::Call {
            func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                MirCallableKey::Intrinsic(MirIntrinsicId::BorrowSlice),
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

        assert_eq!(borrows.len(), 1);
        assert_eq!(borrows[0].owner, Local(2));
        assert_eq!(borrows[0].place.local, Local(1));
        assert_eq!(borrows[0].kind, AccessKind::BorrowShared);
    }
}
