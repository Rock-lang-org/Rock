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
    place_paths: PlacePathTable,
    move_paths: MovePathTable,
}

impl InitializationAnalysis {
    pub fn new(func: &MirFunction, type_context: &TypeContext) -> Self {
        let mut always_init = HashSet::new();
        let mut raw_pointer_locals = HashSet::new();
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
            Rvalue::Object(operand, _)
            | Rvalue::Use(operand)
            | Rvalue::Cast(operand, _)
            | Rvalue::UnaryOp(_, operand) => {
                self.move_operand_if_move(state, operand, info);
            }
            Rvalue::BinaryOp(_, lhs, rhs) => {
                self.move_operand_if_move(state, lhs, info);
                self.move_operand_if_move(state, rhs, info);
            }
            Rvalue::Aggregate(_, operands) => self.move_operands(state, operands, info),
            Rvalue::Closure(closure) => {
                for capture in &closure.captures {
                    if capture.kind == crate::mir::MirClosureCaptureKind::ByValue {
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
                self.apply_rvalue_moves(state, rvalue, &move_info);

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
mod tests {
    use super::*;

    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::mir::{
        AggregateKind, BasicBlock, BasicBlockId, Constant, LocalDecl, MirAssert, MirAssertKind,
        MirClosureCapture, MirClosureCaptureKind, MirClosureId, MirFunctionId, Mutability,
        Projection,
    };
    use crate::types::Type;

    fn test_type_id(
        type_context: &mut crate::type_context::TypeContext,
        ty: Type,
    ) -> crate::ids::TypeId {
        type_context.intern_type(&ty)
    }

    fn field_place(local: Local, index: usize) -> Place {
        Place {
            local,
            projection: vec![Projection::Field {
                index,
                identity: None,
            }],
        }
    }

    fn test_function_with_block(
        type_context: &mut crate::type_context::TypeContext,
        block: BasicBlock,
    ) -> MirFunction {
        let i64_ty = test_type_id(type_context, Type::I64);

        MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1))),
            name: "test".to_string(),
            basic_blocks: vec![block],
            local_decls: vec![
                LocalDecl {
                    ty: i64_ty,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_ty,
                    mutability: Mutability::Mut,
                    name: Some("value".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_ty,
                    mutability: Mutability::Mut,
                    name: Some("dest".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: i64_ty,
            ownership: Default::default(),
        }
    }

    #[test]
    fn initialization_fixpoint_successor_entry_is_not_poisoned_by_unvisited_state() {
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_ty = test_type_id(&mut type_context, Type::I64);
        let local = Place {
            local: Local(1),
            projection: Vec::new(),
        };
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1))),
            name: "test".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![StatementData::assign(
                        local.clone(),
                        Rvalue::Use(Operand::Constant(Constant::Int(1))),
                        None,
                    )],
                    terminator: Some(Terminator::Goto(BasicBlockId(1))),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: i64_ty,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_ty,
                    mutability: Mutability::Mut,
                    name: Some("value".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: i64_ty,
            ownership: Default::default(),
        };

        let analysis = InitializationAnalysis::new(&function, &type_context);
        let results = crate::mir::dataflow::run_fixpoint(&analysis, &function);

        assert!(analysis
            .check_operand_at_path(&Operand::Copy(local), &results.entry_sets[1])
            .is_ok());
    }

    #[test]
    fn initialization_binary_op_move_marks_field_moved_and_preserves_sibling() {
        let mut type_context = crate::type_context::TypeContext::new();
        let field0 = field_place(Local(1), 0);
        let field1 = field_place(Local(1), 1);
        let stmt = StatementData::assign(
            Place {
                local: Local(2),
                projection: Vec::new(),
            },
            Rvalue::BinaryOp(
                crate::mir::MirBinOp::Add,
                Operand::Move(field0.clone()),
                Operand::Copy(field1.clone()),
            ),
            None,
        );
        let function = test_function_with_block(
            &mut type_context,
            BasicBlock {
                statements: vec![stmt.clone()],
                terminator: None,
            },
        );
        let analysis = InitializationAnalysis::new(&function, &type_context);
        let mut state = analysis.initial_state(&function);

        analysis.apply_statement(&mut state, &stmt);

        let field0_error = analysis
            .check_operand_at_path(&Operand::Move(field0), &state)
            .unwrap_err();
        assert!(field0_error.message.contains("use of moved value"));
        assert!(analysis.check_place_at_path(&field1, &state).is_ok());
    }

    #[test]
    fn initialization_move_through_raw_pointer_index_preserves_pointer_local() {
        let mut type_context = crate::type_context::TypeContext::new();
        let i64_ty = test_type_id(&mut type_context, Type::I64);
        let ptr_ty = test_type_id(&mut type_context, Type::Pointer(Box::new(Type::I64)));
        let pointer = Place {
            local: Local(1),
            projection: Vec::new(),
        };
        let pointee = Place {
            local: Local(1),
            projection: vec![Projection::Index(Local(2))],
        };
        let stmt = StatementData::assign(
            Place {
                local: Local(3),
                projection: Vec::new(),
            },
            Rvalue::Use(Operand::Move(pointee)),
            None,
        );
        let function = MirFunction {
            id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1))),
            name: "test".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![stmt.clone()],
                terminator: None,
            }],
            local_decls: vec![
                LocalDecl {
                    ty: i64_ty,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: ptr_ty,
                    mutability: Mutability::Mut,
                    name: Some("ptr".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_ty,
                    mutability: Mutability::Mut,
                    name: Some("index".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_ty,
                    mutability: Mutability::Mut,
                    name: Some("dest".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: i64_ty,
            ownership: Default::default(),
        };
        let analysis = InitializationAnalysis::new(&function, &type_context);
        let mut state = analysis.initial_state(&function);

        analysis.apply_statement(&mut state, &stmt);

        assert!(analysis
            .check_operand_at_path(&Operand::Copy(pointer), &state)
            .is_ok());
    }

    #[test]
    fn initialization_assert_move_marks_field_moved() {
        let mut type_context = crate::type_context::TypeContext::new();
        let field0 = field_place(Local(1), 0);
        let stmt = StatementData::assert(
            MirAssert {
                kind: MirAssertKind::BoundsCheck,
                operands: vec![Operand::Move(field0.clone())],
            },
            None,
        );
        let function = test_function_with_block(
            &mut type_context,
            BasicBlock {
                statements: vec![stmt.clone()],
                terminator: None,
            },
        );
        let analysis = InitializationAnalysis::new(&function, &type_context);
        let mut state = analysis.initial_state(&function);

        analysis.apply_statement(&mut state, &stmt);

        let field0_error = analysis
            .check_operand_at_path(&Operand::Move(field0), &state)
            .unwrap_err();
        assert!(field0_error.message.contains("use of moved value"));
    }

    #[test]
    fn initialization_call_move_argument_marks_field_moved() {
        let mut type_context = crate::type_context::TypeContext::new();
        let field0 = field_place(Local(1), 0);
        let terminator = Terminator::Call {
            func: Operand::Constant(Constant::Unit),
            args: vec![Operand::Move(field0.clone())],
            destination: Place {
                local: Local(2),
                projection: Vec::new(),
            },
            target: BasicBlockId(0),
            span: Some(crate::lexer::Span::test()),
        };
        let function = test_function_with_block(
            &mut type_context,
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(terminator.clone()),
            },
        );
        let analysis = InitializationAnalysis::new(&function, &type_context);
        let mut state = analysis.initial_state(&function);

        analysis.apply_terminator(&mut state, &terminator);

        let field0_error = analysis
            .check_operand_at_path(&Operand::Move(field0), &state)
            .unwrap_err();
        assert!(field0_error.message.contains("use of moved value"));
    }

    #[test]
    fn initialization_call_destination_remains_init_when_also_moved_arg() {
        let mut type_context = crate::type_context::TypeContext::new();
        let destination = Place {
            local: Local(2),
            projection: Vec::new(),
        };
        let terminator = Terminator::Call {
            func: Operand::Constant(Constant::Unit),
            args: vec![Operand::Move(destination.clone())],
            destination: destination.clone(),
            target: BasicBlockId(0),
            span: Some(crate::lexer::Span::test()),
        };
        let function = test_function_with_block(
            &mut type_context,
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(terminator.clone()),
            },
        );
        let analysis = InitializationAnalysis::new(&function, &type_context);
        let mut state = analysis.initial_state(&function);

        analysis.apply_terminator(&mut state, &terminator);

        assert!(analysis
            .check_operand_at_path(&Operand::Copy(destination), &state)
            .is_ok());
    }

    #[test]
    fn initialization_assignment_destination_remains_init_when_rhs_moves_part() {
        let mut type_context = crate::type_context::TypeContext::new();
        let destination = Place {
            local: Local(1),
            projection: Vec::new(),
        };
        let field0 = field_place(Local(1), 0);
        let field1 = field_place(Local(1), 1);
        let stmt = StatementData::assign(
            destination.clone(),
            Rvalue::Aggregate(
                AggregateKind::Tuple,
                vec![Operand::Move(field0), Operand::Copy(field1)],
            ),
            None,
        );
        let function = test_function_with_block(
            &mut type_context,
            BasicBlock {
                statements: vec![stmt.clone()],
                terminator: None,
            },
        );
        let analysis = InitializationAnalysis::new(&function, &type_context);
        let mut state = analysis.initial_state(&function);

        analysis.apply_statement(&mut state, &stmt);

        assert!(analysis.check_place_at_path(&destination, &state).is_ok());
    }

    #[test]
    fn initialization_treats_closure_capture_locals_as_entry_initialized() {
        let owner = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(1)));
        let capture_local = Local(1);
        let mut type_context = crate::type_context::TypeContext::new();
        let function = MirFunction {
            id: MirFunctionId::Closure(Box::new(MirClosureId {
                owner,
                local_index: 0,
            })),
            name: "lambda_0".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: Vec::new(),
                terminator: None,
            }],
            local_decls: vec![
                LocalDecl {
                    ty: test_type_id(&mut type_context, Type::I64),
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: test_type_id(&mut type_context, Type::I64),
                    mutability: Mutability::Not,
                    name: Some("captured".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: vec![MirClosureCapture {
                name: "captured".to_string(),
                local: capture_local,
                kind: MirClosureCaptureKind::ByRef,
                span: None,
            }],
            arg_count: 0,
            ret_type: test_type_id(&mut type_context, Type::I64),
            ownership: Default::default(),
        };

        let analysis = InitializationAnalysis::new(&function, &type_context);
        let state = analysis.initial_state(&function);

        assert_eq!(state.get_local(capture_local), Some(&InitState::Init));
    }

    #[test]
    fn initialization_tracks_move_path_for_field_move() {
        use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
        use crate::mir::{Local, Place, Projection};

        let mut places = PlacePathTable::new();
        let root_place = places.intern(Place {
            local: Local(1),
            projection: Vec::new(),
        });
        let field_place = places.intern(Place {
            local: Local(1),
            projection: vec![Projection::Field {
                index: 0,
                identity: None,
            }],
        });
        let moves = MovePathTable::from_place_paths(&places);
        let mut state = InitMap::new_for_move_paths(&moves, InitState::Init);

        state.set_moved(
            field_place,
            &moves,
            MoveInfo {
                span: Some(crate::lexer::Span::test()),
            },
        );

        assert!(state.check_place_path(field_place, &moves).is_err());
        assert!(state.check_place_path(root_place, &moves).is_err());
    }

    #[test]
    fn initialization_drop_allows_fully_moved_root() {
        use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
        use crate::mir::{Local, Place, Projection};

        let mut places = PlacePathTable::new();
        let root_place = places.intern(Place {
            local: Local(1),
            projection: Vec::new(),
        });
        places.intern(Place {
            local: Local(1),
            projection: vec![Projection::Field {
                index: 0,
                identity: None,
            }],
        });
        places.intern(Place {
            local: Local(1),
            projection: vec![Projection::Field {
                index: 1,
                identity: None,
            }],
        });
        let moves = MovePathTable::from_place_paths(&places);
        let mut state = InitMap::new_for_move_paths(&moves, InitState::Init);

        state.set_moved(
            root_place,
            &moves,
            MoveInfo {
                span: Some(crate::lexer::Span::test()),
            },
        );

        assert!(state.check_drop_place_path(root_place, &moves).is_ok());
    }

    #[test]
    fn initialization_drop_rejects_partially_moved_root() {
        use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
        use crate::mir::{Local, Place, Projection};

        let mut places = PlacePathTable::new();
        let root_place = places.intern(Place {
            local: Local(1),
            projection: Vec::new(),
        });
        let field_place = places.intern(Place {
            local: Local(1),
            projection: vec![Projection::Field {
                index: 0,
                identity: None,
            }],
        });
        places.intern(Place {
            local: Local(1),
            projection: vec![Projection::Field {
                index: 1,
                identity: None,
            }],
        });
        let moves = MovePathTable::from_place_paths(&places);
        let mut state = InitMap::new_for_move_paths(&moves, InitState::Init);

        state.set_moved(
            field_place,
            &moves,
            MoveInfo {
                span: Some(crate::lexer::Span::test()),
            },
        );

        let error = state.check_drop_place_path(root_place, &moves).unwrap_err();

        assert_eq!(error.message, "drop of partially moved value");
        assert!(error.move_span.is_some());
    }

    #[test]
    fn init_map_join_preserves_moved_over_init() {
        use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
        use crate::mir::{Local, Place, Projection};

        let mut places = PlacePathTable::new();
        let root_place = places.intern(Place {
            local: Local(1),
            projection: Vec::new(),
        });
        let field_place = places.intern(Place {
            local: Local(1),
            projection: vec![Projection::Field {
                index: 0,
                identity: None,
            }],
        });
        let moves = MovePathTable::from_place_paths(&places);
        let mut moved_state = InitMap::new_for_move_paths(&moves, InitState::Init);
        let init_state = InitMap::new_for_move_paths(&moves, InitState::Init);

        moved_state.set_moved(
            field_place,
            &moves,
            MoveInfo {
                span: Some(crate::lexer::Span::test()),
            },
        );

        moved_state.join(&init_state);

        let field_error = moved_state
            .check_place_path(field_place, &moves)
            .unwrap_err();
        let root_error = moved_state
            .check_place_path(root_place, &moves)
            .unwrap_err();

        assert_eq!(field_error.message, "use of moved value");
        assert_eq!(root_error.message, "use of partially moved value");
    }

    #[test]
    fn init_map_join_requires_init_on_all_paths() {
        use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
        use crate::mir::{Local, Place, Projection};

        let mut places = PlacePathTable::new();
        let root_place = places.intern(Place {
            local: Local(1),
            projection: Vec::new(),
        });
        let field_place = places.intern(Place {
            local: Local(1),
            projection: vec![Projection::Field {
                index: 0,
                identity: None,
            }],
        });
        let moves = MovePathTable::from_place_paths(&places);
        let mut init_state = InitMap::new_for_move_paths(&moves, InitState::Uninit);
        let uninit_state = InitMap::new_for_move_paths(&moves, InitState::Uninit);

        init_state.set_place_path(root_place, &moves, InitState::Init);
        init_state.join(&uninit_state);

        let error = init_state
            .check_place_path(field_place, &moves)
            .unwrap_err();

        assert_eq!(error.message, "use of uninitialized place");
    }

    #[test]
    fn initialization_errors_do_not_render_mir_local_ids() {
        use crate::mir::borrowck::paths::{MovePathTable, PlacePathTable};
        use crate::mir::{Local, Operand, Place};

        let mut places = PlacePathTable::new();
        places.intern(Place {
            local: Local(9),
            projection: Vec::new(),
        });
        let moves = MovePathTable::from_place_paths(&places);
        let state = InitMap::new_for_move_paths(&moves, InitState::Uninit);
        let error = InitializationAnalysis::check_operand(
            &Operand::Move(Place {
                local: Local(9),
                projection: Vec::new(),
            }),
            &state,
        )
        .unwrap_err();

        assert_eq!(error.message, "use of uninitialized variable");
        assert!(!error.message.contains("Local("));
    }
}
