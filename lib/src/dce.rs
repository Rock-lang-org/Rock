//! Dead code elimination.
//!
//! The compiler DCE authority is instance reachability over monomorphized
//! callable records.

use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{CrateId, DefId, InstanceId};
use crate::mir::{
    Constant, MirCallable, MirCallableKey, MirFunction, MirFunctionId, MirInstanceBodies, Operand,
    Rvalue, StatementKind, Terminator,
};
use crate::mono::{InstanceOrigin, MonomorphizedProgram};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstanceDceReport {
    pub removed_instances: usize,
    pub retained_instances: usize,
    pub missing_instance_edges: Vec<InstanceId>,
}

pub fn prune_unreachable_instances(
    program: &mut MonomorphizedProgram,
    bodies: &mut MirInstanceBodies,
) -> InstanceDceReport {
    let original_len = program.instances.len();
    let indexes = InstanceReachabilityIndexes::new(program);
    let mut reachable = BTreeSet::new();
    let mut worklist = instance_roots(program, bodies);
    let mut missing_instance_edges = BTreeSet::new();

    if worklist.is_empty() {
        return InstanceDceReport {
            removed_instances: 0,
            retained_instances: original_len,
            missing_instance_edges: Vec::new(),
        };
    }

    while let Some(instance_id) = worklist.pop() {
        if !reachable.insert(instance_id) {
            continue;
        }

        if !indexes.known_instances.contains(&instance_id) {
            missing_instance_edges.insert(instance_id);
            continue;
        }
        let Some(body) = bodies.get(instance_id) else {
            continue;
        };

        let mut edges = Vec::new();
        collect_instance_edges_mir_function(
            &body.function,
            &indexes,
            &mut edges,
            &mut missing_instance_edges,
        );
        for nested in bodies
            .nested_functions()
            .iter()
            .filter(|nested| nested_function_owner_instance(nested) == Some(instance_id))
        {
            collect_instance_edges_mir_function(
                nested,
                &indexes,
                &mut edges,
                &mut missing_instance_edges,
            );
        }
        worklist.extend(edges);
    }

    program.instances.retain(|id, _| reachable.contains(id));
    bodies.retain(|id, _| reachable.contains(&id));
    bodies.retain_nested_functions(|nested| {
        nested_function_owner_instance(nested).is_some_and(|owner| reachable.contains(&owner))
    });

    InstanceDceReport {
        removed_instances: original_len.saturating_sub(program.instances.len()),
        retained_instances: program.instances.len(),
        missing_instance_edges: missing_instance_edges.into_iter().collect(),
    }
}

fn nested_function_owner_instance(function: &MirFunction) -> Option<InstanceId> {
    match &function.id {
        MirFunctionId::Closure(closure) => mir_function_root_owner_instance(&closure.owner),
        MirFunctionId::Instance(id) => Some(*id),
        MirFunctionId::Function(_) | MirFunctionId::Extern(_)
        | MirFunctionId::ArrayTailDrop(_) => None,
    }
}

fn mir_function_root_owner_instance(id: &MirFunctionId) -> Option<InstanceId> {
    match id {
        MirFunctionId::Instance(id) => Some(*id),
        MirFunctionId::Closure(closure) => mir_function_root_owner_instance(&closure.owner),
        MirFunctionId::Function(_) | MirFunctionId::Extern(_)
        | MirFunctionId::ArrayTailDrop(_) => None,
    }
}

fn collect_instance_edges_mir_function(
    function: &MirFunction,
    indexes: &InstanceReachabilityIndexes,
    edges: &mut Vec<InstanceId>,
    missing_instance_edges: &mut BTreeSet<InstanceId>,
) {
    for block in &function.basic_blocks {
        for statement in &block.statements {
            collect_instance_edges_mir_statement(
                &statement.kind,
                indexes,
                edges,
                missing_instance_edges,
            );
        }
        if let Some(terminator) = &block.terminator {
            collect_instance_edges_mir_terminator(
                terminator,
                indexes,
                edges,
                missing_instance_edges,
            );
        }
    }
}

fn collect_instance_edges_mir_statement(
    statement: &StatementKind,
    indexes: &InstanceReachabilityIndexes,
    edges: &mut Vec<InstanceId>,
    missing_instance_edges: &mut BTreeSet<InstanceId>,
) {
    match statement {
        StatementKind::Assign(_, rvalue) => {
            collect_instance_edges_mir_rvalue(rvalue, indexes, edges, missing_instance_edges);
        }
        StatementKind::Assert(assertion) => {
            for operand in &assertion.operands {
                collect_instance_edges_mir_operand(operand, indexes, edges, missing_instance_edges);
            }
        }
        StatementKind::StorageLive(_) | StatementKind::StorageDead(_) => {}
    }
}

fn collect_instance_edges_mir_terminator(
    terminator: &Terminator,
    indexes: &InstanceReachabilityIndexes,
    edges: &mut Vec<InstanceId>,
    missing_instance_edges: &mut BTreeSet<InstanceId>,
) {
    match terminator {
        Terminator::Goto(_)
        | Terminator::GotoWithOrigin { .. }
        | Terminator::Return
        | Terminator::ReturnWithOrigin { .. }
        | Terminator::Drop { .. }
        | Terminator::DropWithOrigin { .. } => {}
        Terminator::SwitchInt { discr, .. } | Terminator::SwitchIntWithOrigin { discr, .. } => {
            collect_instance_edges_mir_operand(discr, indexes, edges, missing_instance_edges);
        }
        Terminator::Call { func, args, .. } => {
            collect_instance_edges_mir_operand(func, indexes, edges, missing_instance_edges);
            for arg in args {
                collect_instance_edges_mir_operand(arg, indexes, edges, missing_instance_edges);
            }
        }
    }
}

fn collect_instance_edges_mir_rvalue(
    rvalue: &Rvalue,
    indexes: &InstanceReachabilityIndexes,
    edges: &mut Vec<InstanceId>,
    missing_instance_edges: &mut BTreeSet<InstanceId>,
) {
    match rvalue {
        Rvalue::Use(operand) | Rvalue::Cast(operand, _) | Rvalue::UnaryOp(_, operand) => {
            collect_instance_edges_mir_operand(operand, indexes, edges, missing_instance_edges);
        }
        Rvalue::BinaryOp(_, lhs, rhs) => {
            collect_instance_edges_mir_operand(lhs, indexes, edges, missing_instance_edges);
            collect_instance_edges_mir_operand(rhs, indexes, edges, missing_instance_edges);
        }
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                collect_instance_edges_mir_operand(operand, indexes, edges, missing_instance_edges);
            }
        }
        Rvalue::Ref(_, _) | Rvalue::Closure(_) | Rvalue::Discriminant(_) => {}
    }
}

fn collect_instance_edges_mir_operand(
    operand: &Operand,
    indexes: &InstanceReachabilityIndexes,
    edges: &mut Vec<InstanceId>,
    missing_instance_edges: &mut BTreeSet<InstanceId>,
) {
    if let Operand::Constant(Constant::Callable(callable)) = operand {
        collect_instance_edges_mir_callable(callable, indexes, edges, missing_instance_edges);
    }
}

fn collect_instance_edges_mir_callable(
    callable: &MirCallable,
    indexes: &InstanceReachabilityIndexes,
    edges: &mut Vec<InstanceId>,
    missing_instance_edges: &mut BTreeSet<InstanceId>,
) {
    match callable {
        MirCallable::Resolved(MirCallableKey::Instance(id)) => {
            record_instance_edge(*id, indexes, edges, missing_instance_edges)
        }
        MirCallable::Resolved(MirCallableKey::Function(id)) => {
            if let Some(instance_id) = indexes.function_instance(*id) {
                record_instance_edge(instance_id, indexes, edges, missing_instance_edges);
            }
        }
        MirCallable::Resolved(
            MirCallableKey::Extern(_)
            | MirCallableKey::Closure(_)
            | MirCallableKey::Intrinsic(_)
            | MirCallableKey::RuntimeHelper(_),
        ) => {}
    }
}

fn record_instance_edge(
    id: InstanceId,
    indexes: &InstanceReachabilityIndexes,
    edges: &mut Vec<InstanceId>,
    missing_instance_edges: &mut BTreeSet<InstanceId>,
) {
    if indexes.known_instances.contains(&id) {
        edges.push(id);
    } else {
        missing_instance_edges.insert(id);
    }
}

fn instance_roots(program: &MonomorphizedProgram, bodies: &MirInstanceBodies) -> Vec<InstanceId> {
    let main_def = current_crate_main_function_id(&program.program);

    let mut roots = program
        .instances
        .iter()
        .filter_map(|(id, record)| {
            let is_main_origin = main_def
                .is_some_and(|main_def| record.origin == InstanceOrigin::Function(main_def));
            is_main_origin.then_some(*id)
        })
        .collect::<Vec<_>>();

    if !roots.is_empty() {
        roots.extend(bodies.runtime_instance_roots());
    }
    roots
}

fn current_crate_main_function_id(
    program: &crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
) -> Option<DefId> {
    program
        .functions_by_id()
        .find_map(|(id, name, _)| (id.crate_id == CrateId(0) && name == "main").then_some(id))
}

struct InstanceReachabilityIndexes {
    zero_substitution_functions: BTreeMap<DefId, InstanceId>,
    known_instances: BTreeSet<InstanceId>,
}

impl InstanceReachabilityIndexes {
    fn new(program: &MonomorphizedProgram) -> Self {
        let mut zero_substitution_functions = BTreeMap::new();
        let mut known_instances = BTreeSet::new();

        for (id, record) in &program.instances {
            known_instances.insert(*id);
            if record.substitution.is_empty() {
                if let InstanceOrigin::Function(def_id) = record.origin {
                    zero_substitution_functions.insert(def_id, *id);
                }
            }
        }

        Self {
            zero_substitution_functions,
            known_instances,
        }
    }

    fn function_instance(&self, def_id: DefId) -> Option<InstanceId> {
        self.zero_substitution_functions.get(&def_id).copied()
    }
}

#[cfg(test)]
mod tests;
