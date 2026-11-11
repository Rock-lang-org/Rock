//! Initialization tracking analysis.
//!
//! Tracks which locals are initialized, moved, or uninitialized.

use std::collections::{HashMap, HashSet};

use crate::ids::{Idx, MovePathId, PlacePathId};
use crate::lexer::Span;
use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
use crate::mir::dataflow::{Analysis, Lattice};
use crate::mir::{
    Local, MirFunction, Operand, Place, Rvalue, StatementData, StatementKind, Terminator,
};
use crate::type_context::{Ty, TypeContext, TypeView};

/// Information about where a value was moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveInfo {
    /// The span where the move occurred
    pub span: Option<Span>,
}

/// Initialization state of a local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitState {
    /// Local is uninitialized (just StorageLive)
    Uninit,
    /// Local is initialized and can be used
    Init,
    /// Local has been moved from, with info about where
    Moved(MoveInfo),
}

/// Map of locals to their initialization state.
/// Newtype wrapper to provide custom lattice join semantics.
#[derive(Debug, Clone)]
pub struct InitMap {
    reachable: bool,
    states: Vec<InitState>,
    local_roots: HashMap<Local, MovePathId>,
}

impl InitMap {
    pub fn new_for_move_paths(moves: &MovePathTable, initial: InitState) -> Self {
        let states = moves.iter_ids().map(|_| initial.clone()).collect();
        let local_roots = moves.local_roots();

        Self {
            reachable: true,
            states,
            local_roots,
        }
    }

    fn unreachable_for_move_paths(moves: &MovePathTable) -> Self {
        let states = moves.iter_ids().map(|_| InitState::Uninit).collect();
        let local_roots = moves.local_roots();

        Self {
            reachable: false,
            states,
            local_roots,
        }
    }

    pub fn get_local(&self, local: Local) -> Option<&InitState> {
        let move_path = self.local_roots.get(&local)?;
        self.states.get(move_path.index())
    }

    fn set_local(&mut self, local: Local, state: InitState) {
        if let Some(move_path) = self.local_roots.get(&local) {
            if let Some(slot) = self.states.get_mut(move_path.index()) {
                *slot = state;
            }
        }
    }

    pub fn set_moved(&mut self, place_path: PlacePathId, moves: &MovePathTable, info: MoveInfo) {
        let Some(move_path) = moves.move_path_for_place(place_path) else {
            return;
        };

        for descendant in moves.descendants_inclusive(move_path) {
            if let Some(slot) = self.states.get_mut(descendant.index()) {
                *slot = InitState::Moved(info.clone());
            }
        }
    }

    fn set_place_path(&mut self, place_path: PlacePathId, moves: &MovePathTable, state: InitState) {
        let Some(move_path) = moves.move_path_for_place(place_path) else {
            return;
        };

        for descendant in moves.descendants_inclusive(move_path) {
            if let Some(slot) = self.states.get_mut(descendant.index()) {
                *slot = state.clone();
            }
        }
    }

    pub fn check_place_path(
        &self,
        place_path: PlacePathId,
        moves: &MovePathTable,
    ) -> Result<(), InitError> {
        let Some(move_path) = moves.move_path_for_place(place_path) else {
            return Ok(());
        };

        for ancestor in moves.ancestors_inclusive(move_path) {
            match self.states.get(ancestor.index()) {
                Some(InitState::Init) => {}
                Some(InitState::Uninit) => {
                    return Err(InitError {
                        message: "use of uninitialized place".to_string(),
                        move_span: None,
                    });
                }
                Some(InitState::Moved(info)) => {
                    return Err(InitError {
                        message: "use of moved value".to_string(),
                        move_span: info.span.clone(),
                    });
                }
                None => return Ok(()),
            }
        }

        for descendant in moves.descendants(move_path) {
            if let Some(InitState::Moved(info)) = self.states.get(descendant.index()) {
                return Err(InitError {
                    message: "use of partially moved value".to_string(),
                    move_span: info.span.clone(),
                });
            }
        }

        Ok(())
    }

    pub fn check_discriminant_place_path(
        &self,
        place_path: PlacePathId,
        moves: &MovePathTable,
    ) -> Result<(), InitError> {
        let Some(move_path) = moves.move_path_for_place(place_path) else {
            return Ok(());
        };

        for ancestor in moves.ancestors_inclusive(move_path) {
            match self.states.get(ancestor.index()) {
                Some(InitState::Init) => {}
                Some(InitState::Uninit) => {
                    return Err(InitError {
                        message: "use of uninitialized place".to_string(),
                        move_span: None,
                    });
                }
                Some(InitState::Moved(info)) => {
                    return Err(InitError {
                        message: "use of moved value".to_string(),
                        move_span: info.span.clone(),
                    });
                }
                None => return Ok(()),
            }
        }

        Ok(())
    }

    pub fn check_drop_place_path(
        &self,
        place_path: PlacePathId,
        moves: &MovePathTable,
    ) -> Result<(), InitError> {
        let Some(move_path) = moves.move_path_for_place(place_path) else {
            return Ok(());
        };

        for ancestor in moves.ancestors_inclusive(move_path) {
            match self.states.get(ancestor.index()) {
                Some(InitState::Init) => {}
                Some(InitState::Uninit | InitState::Moved(_)) => return Ok(()),
                None => return Ok(()),
            }
        }

        for descendant in moves.descendants(move_path) {
            if let Some(InitState::Moved(info)) = self.states.get(descendant.index()) {
                return Err(InitError {
                    message: "drop of partially moved value".to_string(),
                    move_span: info.span.clone(),
                });
            }
        }

        Ok(())
    }
}

impl Lattice for InitMap {
    fn join(&mut self, other: &Self) -> bool {
        if !other.reachable {
            return false;
        }

        if !self.reachable {
            *self = other.clone();
            return true;
        }

        let mut changed = false;
        for (index, v) in other.states.iter().enumerate() {
            if let Some(existing) = self.states.get(index) {
                // At control flow merge, keep moved conservative and require
                // initialization on every incoming path.
                // - Init + Init = Init (definitely initialized)
                // - Init + Moved = Moved (may have been moved on one path)
                // - Init + Uninit = Uninit (not initialized on every path)
                // - Uninit + Uninit = Uninit (definitely uninitialized)
                // - Uninit + Moved = Moved (may have been moved)
                // - Moved + Moved = Moved (definitely moved, keep one of the move infos)
                let merged = match (existing, v) {
                    (InitState::Moved(info), _) => InitState::Moved(info.clone()),
                    (_, InitState::Moved(info)) => InitState::Moved(info.clone()),
                    (InitState::Init, InitState::Init) => InitState::Init,
                    (InitState::Uninit, _) | (_, InitState::Uninit) => InitState::Uninit,
                };
                if existing != &merged {
                    self.states[index] = merged;
                    changed = true;
                }
            } else {
                self.states.push(v.clone());
                changed = true;
            }
        }
        for (local, root) in &other.local_roots {
            self.local_roots.entry(*local).or_insert(*root);
        }
        changed
    }
}

/// Analysis that tracks initialization state of all locals.
pub struct InitializationAnalysis {
    /// Locals that are always initialized (function params, constants)
    always_init: HashSet<Local>,
    raw_pointer_locals: HashSet<Local>,
    copy_locals: HashSet<Local>,
    place_paths: PlacePathTable,
    move_paths: MovePathTable,
}

impl InitializationAnalysis {
    pub fn new(func: &MirFunction, type_context: &TypeContext) -> Self {
        let mut always_init = HashSet::new();
        let mut raw_pointer_locals = HashSet::new();
        let mut copy_locals = HashSet::new();
        let type_view = TypeView::new(type_context);

        // Arguments are initialized
        for i in 1..=func.arg_count {
            always_init.insert(Local(i));
        }

        // Closure capture locals are environment inputs and are initialized at entry,
        // but they are not user parameters and must not affect arg_count.
        for capture in &func.closure_captures {
            always_init.insert(capture.local);
        }

        for (i, decl) in func.local_decls.iter().enumerate() {
            if type_view.is_copy(decl.ty) {
                copy_locals.insert(Local(i));
            }
            if matches!(type_view.ty(decl.ty), Ty::Function { .. }) {
                always_init.insert(Local(i));
            }
            if matches!(type_view.ty(decl.ty), Ty::Pointer(_)) {
                raw_pointer_locals.insert(Local(i));
            }
        }

        let place_paths = PlacePathTable::for_function(func);
        let move_paths = MovePathTable::from_place_paths(&place_paths);

        // Return place (Local(0)) is not always init - it's written to

        Self {
            always_init,
            raw_pointer_locals,
            copy_locals,
            place_paths,
            move_paths,
        }
    }

    fn root_place(local: Local) -> Place {
        Place {
            local,
            projection: Vec::new(),
        }
    }

    fn set_place_state(&self, state: &mut InitMap, place: &Place, init_state: InitState) {
        if let Some(place_path) = self.place_paths.path_id(place) {
            state.set_place_path(place_path, &self.move_paths, init_state);
        } else {
            state.set_local(place.local, init_state);
        }
    }

    fn set_place_moved(&self, state: &mut InitMap, place: &Place, info: MoveInfo) {
        if self.raw_pointer_locals.contains(&place.local)
            && matches!(
                place.projection.first(),
                Some(crate::mir::Projection::Deref | crate::mir::Projection::Index(_))
            )
        {
            return;
        }

        if let Some(place_path) = self.place_paths.path_id(place) {
            state.set_moved(place_path, &self.move_paths, info);
        } else {
            state.set_local(place.local, InitState::Moved(info));
        }
    }

    fn move_operand_if_move(&self, state: &mut InitMap, operand: &Operand, info: &MoveInfo) {
        if let Operand::Move(place) = operand {
            self.set_place_moved(state, place, info.clone());
        }
    }

    fn move_operands<'a>(
        &self,
        state: &mut InitMap,
        operands: impl IntoIterator<Item = &'a Operand>,
        info: &MoveInfo,
    ) {
        for operand in operands {
            self.move_operand_if_move(state, operand, info);
        }
    }

    fn apply_rvalue_moves(&self, state: &mut InitMap, rvalue: &Rvalue, info: &MoveInfo) {
        match rvalue {
            Rvalue::Use(operand) | Rvalue::Cast(operand, _) | Rvalue::UnaryOp(_, operand) => {
                self.move_operand_if_move(state, operand, info);
            }
            Rvalue::BinaryOp(_, lhs, rhs) => {
                self.move_operand_if_move(state, lhs, info);
                self.move_operand_if_move(state, rhs, info);
            }
            Rvalue::Aggregate(_, operands) => self.move_operands(state, operands, info),
            Rvalue::Closure(closure) => {
                for capture in &closure.captures {
                    if capture.kind == crate::mir::MirClosureCaptureKind::ByValue
                        && !self.copy_locals.contains(&capture.local)
                    {
                        self.set_place_moved(state, &capture.place(), info.clone());
                    }
                }
            }
            Rvalue::Ref(_, _) | Rvalue::Discriminant(_) => {}
        }
    }

    pub fn check_operand_at_path(
        &self,
        operand: &Operand,
        state: &InitMap,
    ) -> Result<(), InitError> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => self
                .check_place_state_at_path(place, state)
                .map_err(|mut error| {
                    if matches!(error.message.as_str(), "use of moved value") {
                        error.message = match operand {
                            Operand::Copy(_) => "borrow of moved value".to_string(),
                            Operand::Move(_) => "use of moved value".to_string(),
                            Operand::Constant(_) => unreachable!(),
                        };
                    } else if matches!(error.message.as_str(), "use of uninitialized place") {
                        error.message = "use of uninitialized variable".to_string();
                    }
                    error
                }),
            Operand::Constant(_) => Ok(()),
        }
    }

    pub fn check_place_at_path(&self, place: &Place, state: &InitMap) -> Result<(), InitError> {
        self.check_place_state_at_path(place, state)
            .map_err(|mut error| {
                if matches!(error.message.as_str(), "use of moved value") {
                    error.message = "borrow of moved value".to_string();
                } else if matches!(error.message.as_str(), "use of uninitialized place") {
                    error.message = "borrow of uninitialized variable".to_string();
                }
                error
            })
    }

    pub fn check_discriminant_at_path(
        &self,
        place: &Place,
        state: &InitMap,
    ) -> Result<(), InitError> {
        if let Some(place_path) = self.place_paths.path_id(place) {
            state
                .check_discriminant_place_path(place_path, &self.move_paths)
                .map_err(|mut error| {
                    if matches!(error.message.as_str(), "use of moved value") {
                        error.message = "borrow of moved value".to_string();
                    } else if matches!(error.message.as_str(), "use of uninitialized place") {
                        error.message = "borrow of uninitialized variable".to_string();
                    }
                    error
                })
        } else {
            Self::check_place(place, state)
        }
    }

    fn check_place_state_at_path(&self, place: &Place, state: &InitMap) -> Result<(), InitError> {
        if let Some(place_path) = self.place_paths.path_id(place) {
            state.check_place_path(place_path, &self.move_paths)
        } else {
            Self::check_place(place, state)
        }
    }

    pub fn check_drop_at_path(&self, place: &Place, state: &InitMap) -> Result<(), InitError> {
        if let Some(place_path) = self.place_paths.path_id(place) {
            state.check_drop_place_path(place_path, &self.move_paths)
        } else {
            Ok(())
        }
    }
}

impl Analysis for InitializationAnalysis {
    type Domain = InitMap;

    fn initial_state(&self, _func: &MirFunction) -> Self::Domain {
        let mut state = InitMap::new_for_move_paths(&self.move_paths, InitState::Uninit);

        // Mark always-initialized locals
        for local in &self.always_init {
            self.set_place_state(&mut state, &Self::root_place(*local), InitState::Init);
        }

        state
    }

    fn bottom_state(&self, _func: &MirFunction) -> Self::Domain {
        InitMap::unreachable_for_move_paths(&self.move_paths)
    }

    fn apply_statement(&self, state: &mut Self::Domain, stmt: &StatementData) {
        match &stmt.kind {
            StatementKind::StorageLive(local) => {
                // Variable comes into scope but is uninitialized
                // (unless it's an always-init local like a param)
                if !self.always_init.contains(local) {
                    self.set_place_state(state, &Self::root_place(*local), InitState::Uninit);
                }
            }
            StatementKind::StorageDead(local) => {
                // Variable goes out of scope
                self.set_place_state(state, &Self::root_place(*local), InitState::Uninit);
            }
            StatementKind::Assign(dest, rvalue) => {
                let move_info = MoveInfo {
                    span: stmt.origin().source_span().cloned(),
                };
                // The loop owns the hidden array temporary and drops its unvisited
                // tail itself, so a loop-owned element move is not a partial move.
                if !stmt.loop_owned_array_move {
                    self.apply_rvalue_moves(state, rvalue, &move_info);
                }

                // Assignment writes happen after evaluating RHS operands.
                self.set_place_state(state, dest, InitState::Init);
            }
            StatementKind::Assert(assertion) => {
                let move_info = MoveInfo {
                    span: stmt.origin().source_span().cloned(),
                };
                self.move_operands(state, &assertion.operands, &move_info);
            }
        }
    }

    fn apply_terminator(&self, state: &mut Self::Domain, term: &Terminator) {
        match term {
            Terminator::Call {
                func,
                args,
                destination,
                span,
                ..
            } => {
                let move_info = MoveInfo { span: span.clone() };
                self.move_operand_if_move(state, func, &move_info);
                self.move_operands(state, args, &move_info);

                // Call writes happen after evaluating operands.
                self.set_place_state(state, destination, InitState::Init);
            }
            Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } => {
                // After drop, the place is uninitialized
                self.set_place_state(state, place, InitState::Uninit);
            }
            _ => {}
        }
    }
}

/// Error information for initialization/move errors
#[derive(Debug, Clone)]
pub struct InitError {
    pub message: String,
    pub move_span: Option<Span>,
}

impl InitializationAnalysis {
    /// Check if an operand is valid to use at the given state.
    pub fn check_operand(operand: &Operand, state: &InitMap) -> Result<(), InitError> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                match state.get_local(place.local) {
                    Some(InitState::Init) => Ok(()),
                    Some(InitState::Uninit) => Err(InitError {
                        message: "use of uninitialized variable".to_string(),
                        move_span: None,
                    }),
                    Some(InitState::Moved(info)) => Err(InitError {
                        message: match operand {
                            Operand::Copy(_) => "borrow of moved value".to_string(),
                            Operand::Move(_) => "use of moved value".to_string(),
                            Operand::Constant(_) => unreachable!(),
                        },
                        move_span: info.span.clone(),
                    }),
                    None => Ok(()), // Unknown local, assume OK
                }
            }
            Operand::Constant(_) => Ok(()),
        }
    }

    /// Check if a place is valid to create a reference to.
    pub fn check_place(place: &Place, state: &InitMap) -> Result<(), InitError> {
        match state.get_local(place.local) {
            Some(InitState::Init) => Ok(()),
            Some(InitState::Uninit) => Err(InitError {
                message: "borrow of uninitialized variable".to_string(),
                move_span: None,
            }),
            Some(InitState::Moved(info)) => Err(InitError {
                message: "borrow of moved value".to_string(),
                move_span: info.span.clone(),
            }),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
