//! Dataflow analysis engine with fixpoint iteration.

use std::collections::VecDeque;

use super::lattice::Lattice;
use crate::mir::{BasicBlockId, MirFunction, StatementData, Terminator};

/// A forward dataflow analysis over MIR.
pub trait Analysis {
    /// The lattice domain for this analysis.
    type Domain: Lattice;

    /// Create the initial state (entry state for first block).
    fn initial_state(&self, func: &MirFunction) -> Self::Domain;

    /// Create the bottom state used for blocks not reached yet.
    fn bottom_state(&self, func: &MirFunction) -> Self::Domain {
        self.initial_state(func)
    }

    /// Apply a statement's effects to the state.
    fn apply_statement(&self, state: &mut Self::Domain, stmt: &StatementData);

    /// Apply a terminator's effects to the state.
    fn apply_terminator(&self, state: &mut Self::Domain, term: &Terminator);

    /// Apply the effects of a basic block to the state.
    fn apply_block(&self, state: &mut Self::Domain, func: &MirFunction, block_id: BasicBlockId) {
        let block = &func.basic_blocks[block_id.0];

        for stmt in &block.statements {
            self.apply_statement(state, stmt);
        }

        if let Some(term) = &block.terminator {
            self.apply_terminator(state, term);
        }
    }
}

/// Results of a dataflow analysis.
pub struct Results<D> {
    /// Blocks reachable from the function entry.
    pub reachable: Vec<bool>,
    /// State at entry of each basic block.
    pub entry_sets: Vec<D>,
    /// State at exit of each basic block.
    pub exit_sets: Vec<D>,
}

/// Shared reachable-edge propagation for native and descriptor-backed bodies.
/// An absent predecessor is not an uninitialized path: the first reachable
/// edge installs its state; subsequent edges use the domain's join.
pub(crate) struct ForwardWorklist<D> {
    entries: Vec<Option<D>>,
    pending: VecDeque<usize>,
}

impl<D: Lattice> ForwardWorklist<D> {
    pub(crate) fn new(blocks: usize, initial: D) -> Self {
        let mut entries = vec![None; blocks];
        let mut pending = VecDeque::new();
        if let Some(entry) = entries.first_mut() {
            *entry = Some(initial);
            pending.push_back(0);
        }
        Self { entries, pending }
    }

    pub(crate) fn pop(&mut self) -> Option<(usize, D)> {
        let block = self.pending.pop_front()?;
        self.entries[block].clone().map(|state| (block, state))
    }

    /// `None` denotes an invalid CFG edge, never a silently forgotten path.
    pub(crate) fn propagate(&mut self, block: usize, state: &D) -> Option<bool> {
        let entry = self.entries.get_mut(block)?;
        let changed = match entry {
            Some(entry) => entry.join(state),
            None => {
                *entry = Some(state.clone());
                true
            }
        };
        if changed && !self.pending.contains(&block) {
            self.pending.push_back(block);
        }
        Some(changed)
    }
}

/// Run a forward dataflow analysis to fixpoint.
pub fn run_fixpoint<A: Analysis>(analysis: &A, func: &MirFunction) -> Results<A::Domain> {
    let num_blocks = func.basic_blocks.len();

    let mut worklist = ForwardWorklist::new(num_blocks, analysis.initial_state(func));
    let mut exit_sets: Vec<A::Domain> = (0..num_blocks)
        .map(|_| analysis.bottom_state(func))
        .collect();

    let mut processed = vec![false; num_blocks];

    // Successor map
    let successors = compute_successors(func);

    while let Some((block_idx, mut state)) = worklist.pop() {
        let block_id = BasicBlockId(block_idx);

        // Compute exit state from entry state
        analysis.apply_block(&mut state, func, block_id);

        // Check if exit state changed
        let changed = {
            let exit = &mut exit_sets[block_idx];
            exit.join(&state)
        };

        // If changed, enqueue successors
        if changed || !processed[block_idx] {
            processed[block_idx] = true;
            for &succ_idx in &successors[block_idx] {
                // MIR admission establishes valid block IDs before analysis.
                assert!(worklist
                    .propagate(succ_idx, &exit_sets[block_idx])
                    .is_some());
            }
        }
    }

    let reachable = worklist.entries.iter().map(Option::is_some).collect();
    let entry_sets = worklist
        .entries
        .into_iter()
        .map(|entry| entry.unwrap_or_else(|| analysis.bottom_state(func)))
        .collect();
    Results {
        reachable,
        entry_sets,
        exit_sets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::dataflow::analyses::init::{InitState, MoveInfo};

    #[test]
    fn first_reachable_edge_is_not_joined_with_an_absent_path() {
        let mut flow = ForwardWorklist::new(3, InitState::Uninit);
        assert_eq!(flow.pop(), Some((0, InitState::Uninit)));
        assert_eq!(flow.propagate(2, &InitState::Init), Some(true));
        assert_eq!(flow.pop(), Some((2, InitState::Init)));
        assert!(flow.entries[1].is_none());
        assert_eq!(flow.propagate(3, &InitState::Init), None);
    }

    #[test]
    fn diamond_and_backedge_joins_revisit_changed_states_once() {
        let mut flow = ForwardWorklist::new(4, InitState::Init);
        flow.pop();
        flow.propagate(1, &InitState::Init);
        flow.propagate(2, &InitState::Uninit);
        let (left, initialized) = flow.pop().unwrap();
        assert_eq!(left, 1);
        flow.propagate(3, &initialized);
        let (right, uninitialized) = flow.pop().unwrap();
        assert_eq!(right, 2);
        flow.propagate(3, &uninitialized);
        assert_eq!(flow.pop(), Some((3, InitState::Uninit)));
        assert!(flow.pop().is_none());
        let moved = InitState::Moved(MoveInfo { span: None });
        flow.propagate(1, &moved);
        assert_eq!(flow.pop(), Some((1, moved.clone())));
        flow.propagate(3, &moved);
        assert_eq!(flow.pop(), Some((3, moved.clone())));
        assert_eq!(flow.propagate(1, &moved), Some(false));
        assert!(flow.pop().is_none());
    }
}

/// Compute successor blocks for each basic block.
fn compute_successors(func: &MirFunction) -> Vec<Vec<usize>> {
    let mut successors = vec![Vec::new(); func.basic_blocks.len()];

    for (i, block) in func.basic_blocks.iter().enumerate() {
        if let Some(term) = &block.terminator {
            match term {
                Terminator::Goto(target) | Terminator::GotoWithOrigin { target, .. } => {
                    successors[i].push(target.0);
                }
                Terminator::SwitchInt {
                    targets, otherwise, ..
                }
                | Terminator::SwitchIntWithOrigin {
                    targets, otherwise, ..
                } => {
                    for (_, target) in targets {
                        successors[i].push(target.0);
                    }
                    successors[i].push(otherwise.0);
                }
                Terminator::Call { target, .. } => {
                    successors[i].push(target.0);
                }
                Terminator::Drop { target, .. } | Terminator::DropWithOrigin { target, .. } => {
                    successors[i].push(target.0);
                }
                Terminator::Return
                | Terminator::ReturnWithOrigin { .. }
                | Terminator::Unreachable { .. } => {}
            }
        }
    }

    successors
}
