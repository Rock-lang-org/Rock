mod backend_contract;
mod blocks;
mod callables;
mod closure_state;
mod closures;
mod contract_layouts;
mod contract_types;
mod drop_flags;
mod drop_state;
mod emit;
mod entry;
mod expr;
mod function;
mod locals;
mod match_patterns;
mod move_tracking;
mod places;
mod projection_provider;
mod reference_origins;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use closure_state::ClosureState;
use drop_state::DropState;

use crate::hir::AcceptedHir;
use crate::ids::{DefId, InstanceId};
use crate::source_map::SemanticSourceMap;
use crate::type_context::TypeContext;
use crate::types::Type;

use super::{
    BasicBlock, BasicBlockId, Local, LocalDecl, MirFunctionId, MirOwnershipMetadata, Place,
};

type HirProgram = crate::hir::HirProgramFor<AcceptedHir>;
type HirFunction = crate::hir::HirFunctionFor<AcceptedHir>;
type HirImpl = crate::hir::HirImplFor<AcceptedHir>;
type HirBlock = crate::hir::HirBlockFor<AcceptedHir>;
type HirStmt = crate::hir::HirStmtFor<AcceptedHir>;
type HirExpr = crate::hir::HirExprFor<AcceptedHir>;
type HirExprKind = crate::hir::HirExprKindFor<AcceptedHir>;

pub struct MirBuilder<'a> {
    program: &'a HirProgram,
    type_context: &'a RefCell<TypeContext>,
    blocks: Vec<BasicBlock>,
    locals: Vec<LocalDecl>,
    current_block: Option<BasicBlockId>,
    var_map: HashMap<String, Local>,
    loop_stack: Vec<LoopTargets>,
    scope_locals: Vec<Vec<Local>>,
    drops: DropState,
    closures: ClosureState,
    current_function_id: Option<MirFunctionId>,
    current_function_name: Option<String>,
    callable_instances_by_def_id: HashMap<DefId, InstanceId>,
    method_def_ids: HashSet<DefId>,
    method_instance_receiver_modes: HashMap<InstanceId, Option<crate::types::ReceiverMode>>,
    direct_drop_types: HashSet<Type>,
    ownership: MirOwnershipMetadata,
    source_map: SemanticSourceMap,
    current_source_owner: Option<DefId>,
}

#[derive(Debug, Clone, Copy)]
struct LoopTargets {
    break_target: BasicBlockId,
    continue_target: BasicBlockId,
    cleanup_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchScrutineeMode {
    ByValue,
    Borrowed,
}

impl<'a> MirBuilder<'a> {
    fn lower_expr(&mut self, expr: &HirExpr, dest: Place) {
        self.lower_expr_with_context(expr, dest, false)
    }
}

#[cfg(test)]
mod tests;
