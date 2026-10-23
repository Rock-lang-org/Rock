//! Simplify compiler-owned drop flags before ownership dataflow sees the CFG.

use crate::mir::dataflow::{run_fixpoint, Analysis, Lattice};
use crate::mir::{BasicBlockId, Constant, Local, MirFunction, Operand, Rvalue, StatementData, StatementKind, Terminator};

#[derive(Clone, PartialEq, Eq)]
struct FlagState {
    reachable: bool,
    values: Vec<Option<bool>>,
}

impl Lattice for FlagState {
    fn join(&mut self, other: &Self) -> bool {
        if !other.reachable { return false; }
        if !self.reachable {
            *self = other.clone();
            return true;
        }
        let mut changed = false;
        for (value, incoming) in self.values.iter_mut().zip(&other.values) {
            if value.is_some() && value != incoming {
                *value = None;
                changed = true;
            }
        }
        changed
    }
}

struct DropFlags {
    // Only the builder's private flag locals are eligible, never source booleans.
    indices: Vec<Option<usize>>,
    count: usize,
}

impl DropFlags {
    fn operand(&self, state: &FlagState, operand: &Operand) -> Option<bool> {
        match operand {
            Operand::Constant(Constant::Bool(value)) => Some(*value),
            Operand::Copy(place) | Operand::Move(place) if place.projection.is_empty() => {
                self.indices[place.local.0].and_then(|index| state.values[index])
            }
            _ => None,
        }
    }
}

impl Analysis for DropFlags {
    type Domain = FlagState;

    fn initial_state(&self, _func: &MirFunction) -> FlagState {
        FlagState { reachable: true, values: vec![None; self.count] }
    }

    fn bottom_state(&self, _func: &MirFunction) -> FlagState {
        FlagState { reachable: false, values: vec![None; self.count] }
    }

    fn apply_statement(&self, state: &mut FlagState, statement: &StatementData) {
        match &statement.kind {
            StatementKind::Assign(place, rvalue) => {
                if let Some(index) = self.indices[place.local.0] {
                    state.values[index] = match rvalue {
                        Rvalue::Use(operand) if place.projection.is_empty() => self.operand(state, operand),
                        _ => None,
                    };
                }
            }
            StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                if let Some(index) = self.indices[local.0] { state.values[index] = None; }
            }
            StatementKind::Assert(_) => {}
        }
    }

    fn apply_terminator(&self, state: &mut FlagState, terminator: &Terminator) {
        if let Terminator::Call { destination, .. } = terminator {
            if let Some(index) = self.indices[destination.local.0] { state.values[index] = None; }
        }
    }
}

pub(super) fn simplify(function: &mut MirFunction, flags: impl Iterator<Item = Local>) {
    let mut indices = vec![None; function.local_decls.len()];
    let mut count = 0;
    for local in flags {
        if indices[local.0].is_none() {
            indices[local.0] = Some(count);
            count += 1;
        }
    }
    if count == 0 { return; }
    let analysis = DropFlags { indices, count };
    let results = run_fixpoint(&analysis, function);
    let mut changed = false;
    for (index, block) in function.basic_blocks.iter_mut().enumerate() {
        let Some(terminator) = &block.terminator else { continue; };
        let (discr, targets, otherwise) = match terminator {
            Terminator::SwitchInt { discr, targets, otherwise }
            | Terminator::SwitchIntWithOrigin { discr, targets, otherwise, .. } => (discr, targets, otherwise),
            _ => continue,
        };
        let (Operand::Copy(place) | Operand::Move(place)) = discr else { continue; };
        if !place.projection.is_empty() || analysis.indices[place.local.0].is_none() { continue; }
        let state = &results.exit_sets[index];
        if !state.reachable { continue; }
        if let Some(value) = analysis.operand(state, discr) {
            let target = targets.iter().find(|(test, _)| *test == i64::from(value))
                .map(|(_, target)| *target).unwrap_or(*otherwise);
            block.terminator = Some(Terminator::goto(target, terminator.origin()));
            changed = true;
        }
    }
    drop(results);
    if changed { retain_reachable_blocks(function); }
}

fn visit_targets(terminator: &mut Terminator, mut visit: impl FnMut(&mut BasicBlockId)) {
    match terminator {
        Terminator::Goto(target) | Terminator::GotoWithOrigin { target, .. }
        | Terminator::Call { target, .. } | Terminator::Drop { target, .. }
        | Terminator::DropWithOrigin { target, .. } => visit(target),
        Terminator::SwitchInt { targets, otherwise, .. }
        | Terminator::SwitchIntWithOrigin { targets, otherwise, .. } => {
            for (_, target) in targets { visit(target); }
            visit(otherwise);
        }
        Terminator::Return | Terminator::ReturnWithOrigin { .. } => {}
    }
}

fn retain_reachable_blocks(function: &mut MirFunction) {
    if function.basic_blocks.is_empty() { return; }
    let mut reachable = vec![false; function.basic_blocks.len()];
    let mut worklist = vec![BasicBlockId(0)];
    while let Some(block) = worklist.pop() {
        if reachable[block.0] { continue; }
        reachable[block.0] = true;
        if let Some(terminator) = &mut function.basic_blocks[block.0].terminator {
            visit_targets(terminator, |target| worklist.push(*target));
        }
    }
    let mut remap = vec![BasicBlockId(usize::MAX); reachable.len()];
    let mut next = 0;
    for (index, retained) in reachable.iter().enumerate() {
        if *retained {
            remap[index] = BasicBlockId(next);
            next += 1;
        }
    }
    let mut index = 0;
    function.basic_blocks.retain_mut(|block| {
        let keep = reachable[index];
        index += 1;
        if keep {
            if let Some(terminator) = &mut block.terminator {
                visit_targets(terminator, |target| *target = remap[target.0]);
            }
        }
        keep
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::mir::{BasicBlock, LocalDecl, LocalSource, MirFunctionId, MirOrigin, Mutability, Place};
    use crate::type_context::TypeContext;
    use crate::types::Type;

    fn place(local: usize) -> Place {
        Place { local: Local(local), projection: vec![] }
    }

    fn assign(local: usize, value: bool) -> StatementData {
        StatementData::assign(place(local), Rvalue::Use(Operand::Constant(Constant::Bool(value))), None)
    }

    fn block(statements: Vec<StatementData>, terminator: Terminator) -> BasicBlock {
        BasicBlock { statements, terminator: Some(terminator) }
    }

    fn guard(local: usize, yes: usize, no: usize) -> Terminator {
        Terminator::SwitchInt {
            discr: Operand::Copy(place(local)),
            targets: vec![(1, BasicBlockId(yes))],
            otherwise: BasicBlockId(no),
        }
    }

    fn function(blocks: Vec<BasicBlock>) -> MirFunction {
        let mut types = TypeContext::new();
        let ty = types.intern_type(&Type::Bool);
        MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
            name: "cleanup".into(),
            basic_blocks: blocks,
            local_decls: (0..3).map(|_| LocalDecl {
                ty, mutability: Mutability::Mut, name: None, span: None,
                source: LocalSource::Temporary,
            }).collect(),
            closure_captures: vec![],
            arg_count: 0,
            ret_type: ty,
            ownership: Default::default(),
        }
    }

    #[test]
    fn proven_cleanup_guard_preserves_origin_and_remaps_drop_target() {
        let origin = MirOrigin::Source(crate::lexer::Span::test());
        let mut func = function(vec![
            block(vec![assign(1, true)], Terminator::switch_int(
                Operand::Copy(place(1)), vec![(1, BasicBlockId(2))], BasicBlockId(1), origin.clone(),
            )),
            block(vec![], Terminator::Return),
            block(vec![], Terminator::Drop { place: place(2), target: BasicBlockId(3) }),
            block(vec![], Terminator::Return),
        ]);
        simplify(&mut func, [Local(1)].into_iter());
        assert_eq!(func.basic_blocks.len(), 3);
        let term = func.basic_blocks[0].terminator.as_ref().unwrap();
        assert_eq!(term.origin(), origin);
        assert!(matches!(term, Terminator::GotoWithOrigin { target: BasicBlockId(1), .. }));
        assert!(matches!(func.basic_blocks[1].terminator, Some(Terminator::Drop { target: BasicBlockId(2), .. })));
    }

    #[test]
    fn cleared_move_flag_prunes_drop_but_source_boolean_is_not_folded() {
        let mut func = function(vec![
            block(vec![assign(1, true), assign(1, false), assign(2, true)], guard(1, 1, 2)),
            block(vec![], Terminator::Drop { place: place(0), target: BasicBlockId(2) }),
            block(vec![], guard(2, 3, 4)),
            block(vec![], Terminator::Return),
            block(vec![], Terminator::Return),
        ]);
        simplify(&mut func, [Local(1)].into_iter());
        assert_eq!(func.basic_blocks.len(), 4);
        assert!(matches!(func.basic_blocks[1].terminator, Some(Terminator::SwitchInt { .. })));
        assert!(!func.basic_blocks.iter().any(|bb| matches!(bb.terminator, Some(Terminator::Drop { .. }))));
    }

    #[test]
    fn disagreeing_branch_and_loop_flags_remain_conditional() {
        let mut branch = function(vec![
            block(vec![], guard(2, 1, 2)),
            block(vec![assign(1, true)], Terminator::Goto(BasicBlockId(3))),
            block(vec![assign(1, false)], Terminator::Goto(BasicBlockId(3))),
            block(vec![], guard(1, 4, 5)),
            block(vec![], Terminator::Return),
            block(vec![], Terminator::Return),
        ]);
        simplify(&mut branch, [Local(1)].into_iter());
        assert!(matches!(branch.basic_blocks[3].terminator, Some(Terminator::SwitchInt { .. })));
        let mut loop_body = function(vec![
            block(vec![assign(1, true)], Terminator::Goto(BasicBlockId(1))),
            block(vec![], guard(1, 2, 3)),
            block(vec![assign(1, false)], Terminator::Goto(BasicBlockId(1))),
            block(vec![], Terminator::Return),
        ]);
        simplify(&mut loop_body, [Local(1)].into_iter());
        assert!(matches!(loop_body.basic_blocks[1].terminator, Some(Terminator::SwitchInt { .. })));
    }

    #[test]
    fn agreeing_branches_fold_and_dead_predecessors_do_not_poison_flags() {
        let mut func = function(vec![
            block(vec![], guard(2, 1, 2)),
            block(vec![assign(1, true)], Terminator::Goto(BasicBlockId(4))),
            block(vec![assign(1, true)], Terminator::Goto(BasicBlockId(4))),
            block(vec![assign(1, false)], Terminator::Goto(BasicBlockId(4))),
            block(vec![], guard(1, 5, 6)),
            block(vec![], Terminator::Return),
            block(vec![], Terminator::Return),
        ]);
        simplify(&mut func, [Local(1)].into_iter());
        assert_eq!(func.basic_blocks.len(), 5);
        assert!(matches!(func.basic_blocks[3].terminator, Some(Terminator::GotoWithOrigin { target: BasicBlockId(4), .. })));
    }
}
