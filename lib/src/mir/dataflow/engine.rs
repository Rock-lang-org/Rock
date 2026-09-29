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

/// Run a forward dataflow analysis to fixpoint.
pub fn run_fixpoint<A: Analysis>(analysis: &A, func: &MirFunction) -> Results<A::Domain> {
    let num_blocks = func.basic_blocks.len();

    // Initialize entry/exit sets
    let mut entry_sets: Vec<A::Domain> = (0..num_blocks)
        .map(|_| analysis.bottom_state(func))
        .collect();

    // First block gets the initial state
    if num_blocks > 0 {
        entry_sets[0] = analysis.initial_state(func);
    }

    let mut exit_sets: Vec<A::Domain> = (0..num_blocks)
        .map(|_| analysis.bottom_state(func))
        .collect();

    // Worklist of blocks to process
    let mut worklist = VecDeque::new();
    let mut reachable = vec![false; num_blocks];
    let mut processed = vec![false; num_blocks];
    if num_blocks > 0 {
        reachable[0] = true;
        worklist.push_back(0);
    }

    // Successor map
    let successors = compute_successors(func);

    while let Some(block_idx) = worklist.pop_front() {
        let block_id = BasicBlockId(block_idx);

        // Compute exit state from entry state
        let mut state = entry_sets[block_idx].clone();
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
                let entry = &mut entry_sets[succ_idx];
                let entry_changed = entry.join(&exit_sets[block_idx]);

                if (!reachable[succ_idx] || entry_changed) && !worklist.contains(&succ_idx) {
                    worklist.push_back(succ_idx);
                }
                reachable[succ_idx] = true;
            }
        }
    }

    Results {
        reachable,
        entry_sets,
        exit_sets,
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
                Terminator::Return | Terminator::ReturnWithOrigin { .. } => {}
            }
        }
    }

    successors
}
