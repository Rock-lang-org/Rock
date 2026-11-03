use crate::mir::{
    BasicBlock, Local, MirBackendContract, MirFunction, Operand, Place, Projection, Rvalue,
    StatementData, StatementKind, Terminator,
};

use crate::ids::TypeId;
use crate::mir::dataflow::{Lattice, LocalSet};
use crate::type_context::{Ty, TypeContext, TypeView};

use super::*;

pub(super) fn compute_successors(func: &MirFunction) -> Vec<Vec<usize>> {
    let mut successors = vec![Vec::new(); func.basic_blocks.len()];

    for (block_idx, block) in func.basic_blocks.iter().enumerate() {
        if let Some(term) = &block.terminator {
            match term {
                crate::mir::Terminator::Goto(target)
                | crate::mir::Terminator::GotoWithOrigin { target, .. } => {
                    successors[block_idx].push(target.0);
                }
                crate::mir::Terminator::SwitchInt {
                    targets, otherwise, ..
                }
                | crate::mir::Terminator::SwitchIntWithOrigin {
                    targets, otherwise, ..
                } => {
                    for (_, target) in targets {
                        successors[block_idx].push(target.0);
                    }
                    successors[block_idx].push(otherwise.0);
                }
                crate::mir::Terminator::Call { target, .. } => {
                    successors[block_idx].push(target.0);
                }
                crate::mir::Terminator::Drop { target, .. }
                | crate::mir::Terminator::DropWithOrigin { target, .. } => {
                    successors[block_idx].push(target.0);
                }
                crate::mir::Terminator::Return
                | crate::mir::Terminator::ReturnWithOrigin { .. } => {}
            }
        }
    }

    successors
}

pub(super) fn reference_locals(
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
) -> LocalSet {
    let type_view = TypeView::new(type_context);
    let mut locals = LocalSet::with_capacity(func.local_decls.len());
    for (idx, decl) in func.local_decls.iter().enumerate() {
        if tracks_reference_liveness(type_view, backend_contract, decl.ty) {
            locals.insert(Local(idx));
        }
    }

    locals
}

fn tracks_reference_liveness(
    view: TypeView<'_>,
    backend_contract: &MirBackendContract,
    ty: TypeId,
) -> bool {
    type_id_contains_reference_with_contract(view, backend_contract, ty)
        || matches!(view.ty(ty), Ty::Function { .. } | Ty::Pointer(_))
}

pub(super) fn tracks_local_reference_liveness(
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    local: Local,
) -> bool {
    let type_view = TypeView::new(type_context);
    func.local_decls
        .get(local.0)
        .is_some_and(|decl| tracks_reference_liveness(type_view, backend_contract, decl.ty))
}

pub(super) fn compute_reference_liveness_for(
    func: &MirFunction,
    reference_locals: &LocalSet,
) -> ReferenceLiveness {
    let successors = compute_successors(func);
    let mut entry_sets = vec![LocalSet::new(); func.basic_blocks.len()];
    let mut exit_sets = vec![LocalSet::new(); func.basic_blocks.len()];

    loop {
        let mut changed = false;

        for block_idx in (0..func.basic_blocks.len()).rev() {
            let mut live_out = LocalSet::new();
            for &succ in &successors[block_idx] {
                live_out.join(&entry_sets[succ]);
            }

            let live_in = transfer_block(
                func,
                &func.basic_blocks[block_idx],
                live_out.clone(),
                reference_locals,
            );

            if live_in != entry_sets[block_idx] {
                entry_sets[block_idx] = live_in;
                changed = true;
            }

            if live_out != exit_sets[block_idx] {
                exit_sets[block_idx] = live_out;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    let mut before_statement_sets = Vec::with_capacity(func.basic_blocks.len());
    let mut after_statement_sets = Vec::with_capacity(func.basic_blocks.len());
    for (block_idx, block) in func.basic_blocks.iter().enumerate() {
        let mut live_after = exit_sets[block_idx].clone();
        let mut block_before = vec![LocalSet::new(); block.statements.len()];
        let mut block_after = vec![LocalSet::new(); block.statements.len()];

        if let Some(term) = &block.terminator {
            apply_terminator_liveness(term, &mut live_after, reference_locals);
        }

        for stmt_idx in (0..block.statements.len()).rev() {
            if let StatementKind::Assign(dest, Rvalue::Ref(_, _)) = &block.statements[stmt_idx].kind
            {
                if reference_locals.contains(dest.local)
                    && local_preserves_reference_liveness(func, dest.local)
                {
                    live_after.insert(dest.local);
                }
            }

            block_after[stmt_idx] = live_after.clone();
            apply_statement_liveness(
                &block.statements[stmt_idx],
                &mut live_after,
                reference_locals,
            );
            block_before[stmt_idx] = live_after.clone();
        }

        before_statement_sets.push(block_before);
        after_statement_sets.push(block_after);
    }

    ReferenceLiveness {
        entry_sets,
        exit_sets,
        before_statement_sets,
        after_statement_sets,
    }
}

fn transfer_block(
    func: &MirFunction,
    block: &BasicBlock,
    mut live: LocalSet,
    reference_locals: &LocalSet,
) -> LocalSet {
    if let Some(term) = &block.terminator {
        apply_terminator_liveness(term, &mut live, reference_locals);
    }

    for stmt in block.statements.iter().rev() {
        if let StatementKind::Assign(dest, Rvalue::Ref(_, _)) = &stmt.kind {
            if reference_locals.contains(dest.local)
                && local_preserves_reference_liveness(func, dest.local)
            {
                live.insert(dest.local);
            }
        }

        apply_statement_liveness(stmt, &mut live, reference_locals);
    }

    live
}

fn local_preserves_reference_liveness(func: &MirFunction, local: Local) -> bool {
    func.local_decls
        .get(local.0)
        .is_some_and(|decl| decl.source.preserves_reference_liveness())
}

fn apply_statement_liveness(
    stmt: &StatementData,
    live: &mut LocalSet,
    reference_locals: &LocalSet,
) {
    match &stmt.kind {
        StatementKind::Assign(dest, rvalue) => {
            if dest.projection.is_empty() && reference_locals.contains(dest.local) {
                live.remove(dest.local);
            }

            for local in rvalue_reference_uses(rvalue, reference_locals) {
                live.insert(local);
            }

            if !dest.projection.is_empty() {
                let mut locals = Vec::new();
                place_reference_uses_vec(dest, reference_locals, &mut locals);
                insert_locals(live, locals);
            }

            if let Rvalue::Closure(closure) = rvalue {
                for capture in &closure.captures {
                    if reference_locals.contains(capture.local) {
                        live.insert(capture.local);
                    }
                }
            }
        }
        StatementKind::Assert(assertion) => {
            let mut locals = Vec::new();
            for operand in &assertion.operands {
                operand_reference_uses(operand, reference_locals, &mut locals);
            }
            insert_locals(live, locals);
        }
        StatementKind::StorageLive(local) => {
            live.remove(*local);
        }
        StatementKind::StorageDead(local) => {
            live.remove(*local);
        }
    }
}

fn apply_terminator_liveness(term: &Terminator, live: &mut LocalSet, reference_locals: &LocalSet) {
    match term {
        Terminator::SwitchInt { discr, .. } | Terminator::SwitchIntWithOrigin { discr, .. } => {
            let mut locals = Vec::new();
            operand_reference_uses(discr, reference_locals, &mut locals);
            insert_locals(live, locals);
        }
        Terminator::Call {
            func,
            args,
            destination,
            ..
        } => {
            if reference_locals.contains(destination.local) {
                live.remove(destination.local);
            }

            let mut locals = Vec::new();
            operand_reference_uses(func, reference_locals, &mut locals);
            for arg in args {
                operand_reference_uses(arg, reference_locals, &mut locals);
            }
            insert_locals(live, locals);
        }
        Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } => {
            let mut locals = Vec::new();
            place_reference_uses_vec(place, reference_locals, &mut locals);
            insert_locals(live, locals);
        }
        Terminator::Goto(_)
        | Terminator::GotoWithOrigin { .. }
        | Terminator::Return
        | Terminator::ReturnWithOrigin { .. } => {}
    }
}

fn rvalue_reference_uses(rvalue: &Rvalue, reference_locals: &LocalSet) -> Vec<Local> {
    let mut locals = Vec::new();

    match rvalue {
        Rvalue::Use(op) => operand_reference_uses(op, reference_locals, &mut locals),
        Rvalue::Ref(_, place) => place_reference_uses_vec(place, reference_locals, &mut locals),
        Rvalue::Cast(op, _) => operand_reference_uses(op, reference_locals, &mut locals),
        Rvalue::BinaryOp(_, a, b) => {
            operand_reference_uses(a, reference_locals, &mut locals);
            operand_reference_uses(b, reference_locals, &mut locals);
        }
        Rvalue::UnaryOp(_, a) => operand_reference_uses(a, reference_locals, &mut locals),
        Rvalue::Discriminant(place) => {
            place_reference_uses_vec(place, reference_locals, &mut locals)
        }
        Rvalue::Aggregate(_, ops) => {
            for op in ops {
                operand_reference_uses(op, reference_locals, &mut locals);
            }
        }
        Rvalue::Closure(closure) => {
            for capture in &closure.captures {
                if reference_locals.contains(capture.local) {
                    locals.push(capture.local);
                }
            }
        }
    }

    locals
}

fn operand_reference_uses(op: &Operand, reference_locals: &LocalSet, out: &mut Vec<Local>) {
    match op {
        Operand::Copy(place) | Operand::Move(place) => {
            place_reference_uses_vec(place, reference_locals, out)
        }
        Operand::Constant(_) => {}
    }
}

fn place_reference_uses_vec(place: &Place, reference_locals: &LocalSet, out: &mut Vec<Local>) {
    if reference_locals.contains(place.local) {
        out.push(place.local);
    }

    for projection in &place.projection {
        if let Projection::Index(local) = projection {
            if reference_locals.contains(*local) {
                out.push(*local);
            }
        }
    }
}

fn insert_locals(live: &mut LocalSet, locals: Vec<Local>) {
    for local in locals {
        live.insert(local);
    }
}
