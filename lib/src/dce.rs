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
        MirFunctionId::Function(_) | MirFunctionId::Extern(_) => None,
    }
}

fn mir_function_root_owner_instance(id: &MirFunctionId) -> Option<InstanceId> {
    match id {
        MirFunctionId::Instance(id) => Some(*id),
        MirFunctionId::Closure(closure) => mir_function_root_owner_instance(&closure.owner),
        MirFunctionId::Function(_) | MirFunctionId::Extern(_) => None,
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
        | Terminator::Unreachable { .. }
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
        Rvalue::Object(operand, _)
        | Rvalue::Use(operand)
        | Rvalue::Cast(operand, _)
        | Rvalue::UnaryOp(_, operand) => {
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
            | MirCallableKey::ObjectAdapter(_)
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
        roots.extend(
            program
                .owned_object_calls
                .values()
                .flat_map(|plan| [&plan.into_parts, &plan.release])
                .filter_map(|key| match key {
                    MirCallableKey::Instance(id) => Some(*id),
                    _ => None,
                }),
        );
        roots.extend(
            program
                .erased
                .dictionaries
                .values()
                .flat_map(|dictionary| dictionary.members.iter())
                .filter_map(|member| match &member.target {
                    MirCallableKey::Instance(id) => Some(*id),
                    MirCallableKey::ObjectAdapter(
                        crate::mir::MirObjectAdapterKey::ConsumingMethod { target, .. },
                    ) => Some(*target),
                    _ => None,
                }),
        );
        // Metadata is a code edge even when no source-level direct call exists.
        roots.extend(program.vtables.values().flat_map(|table| {
            table
                .methods
                .iter()
                .chain(table.drop.iter())
                .filter_map(|key| match key {
                    MirCallableKey::Instance(id) => Some(*id),
                    MirCallableKey::ObjectAdapter(
                        crate::mir::MirObjectAdapterKey::ConsumingMethod { target, .. },
                    ) => Some(*target),
                    _ => None,
                })
        }));
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

#[allow(dead_code)]
struct InstanceReachabilityIndexes {
    zero_substitution_functions: BTreeMap<DefId, InstanceId>,
    known_instances: BTreeSet<InstanceId>,
}

#[allow(dead_code)]
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
mod tests {
    use std::collections::HashMap;

    use crate::hir::{
        AcceptedHir, AcceptedHirBlock as HirBlock, AcceptedHirExpr as HirExpr,
        AcceptedHirFunction as HirFunction, AcceptedHirImpl as HirImpl, HirCallTarget, HirField,
        HirFieldLocation, HirImplOwner, HirNameTables, HirParam, HirStruct, HirVarRef,
        HirVarTarget,
    };
    use crate::ids::{CrateId, DefId, FieldId, HirLocalId, LocalDefId};
    use crate::lexer::Span;
    use crate::mir::{
        BasicBlock, BasicBlockId, Constant, Local, LocalDecl, MirCallable, MirCallableKey,
        MirClosureId, MirFunction, MirFunctionId, Mutability, Operand, Place, Terminator,
    };
    use crate::mono::{
        InstanceId, InstanceImplOwner, InstanceOrigin, InstanceRecord, InstanceSymbols,
        MonomorphizedProgram,
    };
    use crate::types::Type;

    type HirExprKind = crate::hir::HirExprKindFor<AcceptedHir>;
    type HirProgram = crate::hir::HirProgramFor<AcceptedHir>;
    type HirStmt = crate::hir::HirStmtFor<AcceptedHir>;

    use super::InstanceDceReport;

    fn prune_unreachable_instances(program: &mut MonomorphizedProgram) -> InstanceDceReport {
        let mut bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(program);
        super::prune_unreachable_instances(program, &mut bodies)
    }

    fn empty_function(id: DefId, name: &str) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn program(
        functions: HashMap<DefId, HirFunction>,
        structs: HashMap<DefId, HirStruct>,
        impls: HashMap<DefId, HirImpl>,
        names: HirNameTables,
        canonical_names: HashMap<DefId, String>,
    ) -> HirProgram {
        HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            functions,
            structs,
            HashMap::new(),
            HashMap::new(),
            impls,
            HashMap::new(),
            names,
            &canonical_names,
        )
    }

    fn instance_record(
        id: InstanceId,
        origin: InstanceOrigin,
        source_name: &str,
        backend_symbol: &str,
        body: Option<HirFunction>,
        provided_by_object: bool,
    ) -> InstanceRecord {
        InstanceRecord {
            id,
            origin,
            substitution: Vec::new(),
            symbols: InstanceSymbols::new(source_name, backend_symbol),
            declared: body.clone(),
            provided_by_object,
            is_specialization: false,
        }
    }

    fn specialized_instance_record(
        id: InstanceId,
        origin: InstanceOrigin,
        source_name: &str,
        backend_symbol: &str,
        body: Option<HirFunction>,
    ) -> InstanceRecord {
        InstanceRecord {
            id,
            origin,
            substitution: vec![crate::ids::TypeId(0)],
            symbols: InstanceSymbols::new(source_name, backend_symbol),
            declared: body.clone(),
            provided_by_object: false,
            is_specialization: true,
        }
    }

    fn monomorphized_with_instances(instances: Vec<InstanceRecord>) -> MonomorphizedProgram {
        let mut type_context = crate::type_context::TypeContext::new();
        type_context.intern_type(&Type::I64);
        let mut functions = HashMap::new();
        let mut function_names = HashMap::new();
        let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
        for record in &instances {
            if let Some(function) = record.declared.as_ref() {
                if matches!(record.origin, InstanceOrigin::Function(_)) {
                    functions
                        .entry(function.id)
                        .or_insert_with(|| function.clone());
                    function_names.insert(record.symbols.source_name.clone(), function.id);
                }
                if !record.provided_by_object {
                    pre_mir_instance_bodies.insert(record.id, function.clone());
                }
            }
        }
        MonomorphizedProgram {
            erased: Default::default(),
            erased_invocations: Default::default(),
            owned_object_calls: Default::default(),
            object_schemas: Default::default(),
            vtables: Default::default(),
            program: program(
                functions,
                HashMap::new(),
                HashMap::new(),
                HirNameTables {
                    functions_by_name: function_names,
                    ..HirNameTables::default()
                },
                HashMap::new(),
            ),
            instances: instances
                .into_iter()
                .map(|record| (record.id, record))
                .collect(),
            pre_mir_instance_bodies,
            generated_drop_instances: Default::default(),
            type_context,
            source_map: crate::source_map::SemanticSourceMap::default(),
        }
    }

    fn unit_return_function(id: DefId, name: &str) -> HirFunction {
        let mut function = empty_function(id, name);
        function.body.stmts.push(HirStmt::Return(Some(HirExpr {
            kind: HirExprKind::Unit,
            ty: Type::Unit,
            span: Span::test(),
        })));
        function
    }

    fn function_returning_expr(id: DefId, name: &str, value: HirExpr) -> HirFunction {
        let mut function = empty_function(id, name);
        function.body.stmts.push(HirStmt::Return(Some(value)));
        function
    }

    fn callable_field_struct(id: DefId, name: &str, field_id: FieldId, field: &str) -> HirStruct {
        HirStruct {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: field_id,
                name: field.to_string(),
                ty: Type::function(Vec::new(), Type::Unit),
                public: false,
            }],
        }
    }

    fn callable_field_location(owner: DefId, field_id: FieldId, name: &str) -> HirFieldLocation {
        HirFieldLocation {
            owner,
            field_id,
            name: name.to_string(),
        }
    }

    fn add_param(function: &mut HirFunction, name: &str, ty: Type) {
        function.params.push(HirParam {
            name: name.to_string(),
            local_id: HirLocalId(0),
            ty,
            mutable: false,
            is_ref: false,
        });
    }

    fn instance_ref_expr(name: &str, instance_id: InstanceId) -> HirExpr {
        HirExpr {
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: name.to_string(),
                target: HirVarTarget::Instance(instance_id),
            }),
            ty: Type::function(Vec::new(), Type::Unit),
            span: Span::test(),
        }
    }

    #[test]
    fn prune_unreachable_instances_keeps_main_instance() {
        let main_id = DefId::new(CrateId(0), LocalDefId(10));
        let main_instance = InstanceId(0);
        let mut program = monomorphized_with_instances(vec![instance_record(
            main_instance,
            InstanceOrigin::Function(main_id),
            "main",
            "main",
            Some(unit_return_function(main_id, "main")),
            false,
        )]);

        let report = prune_unreachable_instances(&mut program);

        assert_eq!(report.retained_instances, 1);
        assert_eq!(report.removed_instances, 0);
        assert!(program.instances.contains_key(&main_instance));
    }

    #[test]
    fn prune_unreachable_instances_without_main_keeps_instances_for_artifacts() {
        let library_id = DefId::new(CrateId(0), LocalDefId(130));
        let library_instance = InstanceId(0);
        let mut program = monomorphized_with_instances(vec![instance_record(
            library_instance,
            InstanceOrigin::Function(library_id),
            "library_export",
            "library_export",
            Some(unit_return_function(library_id, "library_export")),
            false,
        )]);

        let report = prune_unreachable_instances(&mut program);

        assert_eq!(report.retained_instances, 1);
        assert_eq!(report.removed_instances, 0);
        assert!(program.instances.contains_key(&library_instance));
    }

    #[test]
    fn prune_unreachable_instances_removes_unreachable_function_instance() {
        let main_id = DefId::new(CrateId(0), LocalDefId(10));
        let unused_id = DefId::new(CrateId(0), LocalDefId(11));
        let main_instance = InstanceId(0);
        let unused_instance = InstanceId(1);
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(unit_return_function(main_id, "main")),
                false,
            ),
            instance_record(
                unused_instance,
                InstanceOrigin::Function(unused_id),
                "unused",
                "unused",
                Some(unit_return_function(unused_id, "unused")),
                false,
            ),
        ]);

        let report = prune_unreachable_instances(&mut program);

        assert_eq!(report.retained_instances, 1);
        assert_eq!(report.removed_instances, 1);
        assert!(program.instances.contains_key(&main_instance));
        assert!(!program.instances.contains_key(&unused_instance));
    }

    #[test]
    fn prune_unreachable_instances_does_not_root_by_backend_symbol() {
        let main_id = DefId::new(CrateId(0), LocalDefId(12));
        let helper_id = DefId::new(CrateId(0), LocalDefId(13));
        let main_instance = InstanceId(0);
        let helper_instance = InstanceId(1);
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(unit_return_function(main_id, "main")),
                false,
            ),
            instance_record(
                helper_instance,
                InstanceOrigin::Function(helper_id),
                "helper",
                "main",
                Some(unit_return_function(helper_id, "helper")),
                false,
            ),
        ]);

        prune_unreachable_instances(&mut program);

        assert!(program.instances.contains_key(&main_instance));
        assert!(!program.instances.contains_key(&helper_instance));
    }

    #[test]
    fn prune_unreachable_instances_follows_direct_instance_call_edge() {
        let main_id = DefId::new(CrateId(0), LocalDefId(20));
        let helper_id = DefId::new(CrateId(0), LocalDefId(21));
        let main_instance = InstanceId(0);
        let helper_instance = InstanceId(1);
        let main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Call(
                    Box::new(instance_ref_expr("helper", helper_instance)),
                    Vec::new(),
                    Some(HirCallTarget::Instance(helper_instance)),
                ),
                ty: Type::Unit,
                span: Span::test(),
            },
        );
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                helper_instance,
                InstanceOrigin::Function(helper_id),
                "helper",
                "helper",
                Some(unit_return_function(helper_id, "helper")),
                false,
            ),
        ]);

        let report = prune_unreachable_instances(&mut program);

        assert_eq!(report.retained_instances, 2);
        assert!(program.instances.contains_key(&main_instance));
        assert!(program.instances.contains_key(&helper_instance));
    }

    #[test]
    fn prune_unreachable_instances_follows_function_value_instance_edge() {
        let main_id = DefId::new(CrateId(0), LocalDefId(30));
        let helper_id = DefId::new(CrateId(0), LocalDefId(31));
        let main_instance = InstanceId(0);
        let helper_instance = InstanceId(1);
        let main_body = function_returning_expr(
            main_id,
            "main",
            instance_ref_expr("helper", helper_instance),
        );
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                helper_instance,
                InstanceOrigin::Function(helper_id),
                "helper",
                "helper",
                Some(unit_return_function(helper_id, "helper")),
                false,
            ),
        ]);

        prune_unreachable_instances(&mut program);

        assert!(program.instances.contains_key(&helper_instance));
    }

    #[test]
    fn prune_unreachable_instances_records_missing_instance_edges_without_name_fallback() {
        let main_id = DefId::new(CrateId(0), LocalDefId(40));
        let real_helper_id = DefId::new(CrateId(0), LocalDefId(41));
        let main_instance = InstanceId(0);
        let real_helper_instance = InstanceId(1);
        let missing_instance = InstanceId(99);
        let main_body = function_returning_expr(
            main_id,
            "main",
            instance_ref_expr("real_helper", missing_instance),
        );
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                real_helper_instance,
                InstanceOrigin::Function(real_helper_id),
                "real_helper",
                "real_helper",
                Some(unit_return_function(real_helper_id, "real_helper")),
                false,
            ),
        ]);

        let report = prune_unreachable_instances(&mut program);

        assert_eq!(report.missing_instance_edges, vec![missing_instance]);
        assert!(!program.instances.contains_key(&real_helper_instance));
    }

    #[test]
    fn prune_unreachable_instances_keeps_reachable_generic_specialization() {
        let main_id = DefId::new(CrateId(0), LocalDefId(50));
        let identity_id = DefId::new(CrateId(0), LocalDefId(51));
        let main_instance = InstanceId(0);
        let specialization = InstanceId(1);
        let main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Call(
                    Box::new(instance_ref_expr("identity", specialization)),
                    vec![HirExpr {
                        kind: HirExprKind::IntLiteral(1),
                        ty: Type::I64,
                        span: Span::test(),
                    }],
                    Some(HirCallTarget::Instance(specialization)),
                ),
                ty: Type::I64,
                span: Span::test(),
            },
        );
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            specialized_instance_record(
                specialization,
                InstanceOrigin::Function(identity_id),
                "identity",
                "identity_mono_0",
                Some(unit_return_function(identity_id, "identity")),
            ),
        ]);

        prune_unreachable_instances(&mut program);

        assert!(program.instances.contains_key(&specialization));
    }

    #[test]
    fn prune_unreachable_instances_removes_unused_generic_specialization() {
        let main_id = DefId::new(CrateId(0), LocalDefId(60));
        let identity_id = DefId::new(CrateId(0), LocalDefId(61));
        let main_instance = InstanceId(0);
        let specialization = InstanceId(1);
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(unit_return_function(main_id, "main")),
                false,
            ),
            specialized_instance_record(
                specialization,
                InstanceOrigin::Function(identity_id),
                "identity",
                "identity_mono_0",
                Some(unit_return_function(identity_id, "identity")),
            ),
        ]);

        prune_unreachable_instances(&mut program);

        assert!(!program.instances.contains_key(&specialization));
    }

    #[test]
    fn prune_unreachable_instances_does_not_keep_closure_edges_from_unreachable_owner() {
        let main_id = DefId::new(CrateId(0), LocalDefId(62));
        let owner_id = DefId::new(CrateId(0), LocalDefId(63));
        let callee_id = DefId::new(CrateId(0), LocalDefId(64));
        let main_instance = InstanceId(0);
        let owner_instance = InstanceId(1);
        let callee_instance = InstanceId(2);
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(unit_return_function(main_id, "main")),
                false,
            ),
            instance_record(
                owner_instance,
                InstanceOrigin::Function(owner_id),
                "unused_owner",
                "unused_owner",
                Some(unit_return_function(owner_id, "unused_owner")),
                false,
            ),
            instance_record(
                callee_instance,
                InstanceOrigin::Function(callee_id),
                "closure_callee",
                "closure_callee",
                Some(unit_return_function(callee_id, "closure_callee")),
                false,
            ),
        ]);
        let unit_id = program.type_context.intern_type(&Type::Unit);
        let mut bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut program);
        let closure_id = MirFunctionId::Closure(Box::new(MirClosureId {
            owner: MirFunctionId::Instance(owner_instance),
            local_index: 0,
        }));
        bodies.push_nested(MirFunction {
            id: closure_id.clone(),
            name: "unused_owner_closure".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                            MirCallableKey::Instance(callee_instance),
                        ))),
                        args: Vec::new(),
                        destination: Place {
                            local: Local(0),
                            projection: Vec::new(),
                        },
                        target: BasicBlockId(1),
                        span: Some(crate::lexer::Span::test()),
                    }),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![LocalDecl {
                ty: unit_id,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: unit_id,
            ownership: Default::default(),
        });

        super::prune_unreachable_instances(&mut program, &mut bodies);

        assert!(program.instances.contains_key(&main_instance));
        assert!(!program.instances.contains_key(&owner_instance));
        assert!(!program.instances.contains_key(&callee_instance));
        assert!(!bodies
            .nested_functions()
            .iter()
            .any(|function| function.id == closure_id));

        let mir = crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(
            &program, &bodies,
        );
        assert!(!mir.functions.contains_key(&closure_id));
        assert!(!mir
            .functions
            .contains_key(&MirFunctionId::Instance(callee_instance)));
    }

    #[test]
    fn prune_unreachable_instances_keeps_explicit_runtime_instance_root() {
        let main_id = DefId::new(CrateId(0), LocalDefId(65));
        let runtime_id = DefId::new(CrateId(0), LocalDefId(66));
        let main_instance = InstanceId(0);
        let runtime_instance = InstanceId(1);
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(unit_return_function(main_id, "main")),
                false,
            ),
            instance_record(
                runtime_instance,
                InstanceOrigin::Function(runtime_id),
                "not_drop",
                "not_drop_backend",
                Some(unit_return_function(runtime_id, "not_drop")),
                false,
            ),
        ]);
        let mut bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut program);
        bodies.insert_runtime_instance_root(runtime_instance);

        super::prune_unreachable_instances(&mut program, &mut bodies);

        assert!(program.instances.contains_key(&runtime_instance));
    }

    #[test]
    fn prune_unreachable_instances_keeps_reachable_object_backed_instance() {
        let main_id = DefId::new(CrateId(0), LocalDefId(70));
        let dep_id = DefId::new(CrateId(1), LocalDefId(71));
        let main_instance = InstanceId(0);
        let object_instance = InstanceId(1);
        let main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Call(
                    Box::new(instance_ref_expr("dep::print", object_instance)),
                    Vec::new(),
                    Some(HirCallTarget::Instance(object_instance)),
                ),
                ty: Type::Unit,
                span: Span::test(),
            },
        );
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                object_instance,
                InstanceOrigin::Function(dep_id),
                "dep::print",
                "dep__print",
                Some(unit_return_function(dep_id, "dep::print")),
                true,
            ),
        ]);

        prune_unreachable_instances(&mut program);

        assert!(program.instances.contains_key(&object_instance));
    }

    #[test]
    fn prune_unreachable_instances_removes_unreachable_object_backed_instance() {
        let main_id = DefId::new(CrateId(0), LocalDefId(80));
        let dep_id = DefId::new(CrateId(1), LocalDefId(81));
        let main_instance = InstanceId(0);
        let object_instance = InstanceId(1);
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(unit_return_function(main_id, "main")),
                false,
            ),
            instance_record(
                object_instance,
                InstanceOrigin::Function(dep_id),
                "dep::print",
                "dep__print",
                Some(unit_return_function(dep_id, "dep::print")),
                true,
            ),
        ]);

        prune_unreachable_instances(&mut program);

        assert!(!program.instances.contains_key(&object_instance));
    }

    #[test]
    fn prune_unreachable_instances_maps_direct_function_target_to_zero_substitution_instance() {
        let main_id = DefId::new(CrateId(0), LocalDefId(90));
        let helper_id = DefId::new(CrateId(0), LocalDefId(91));
        let main_instance = InstanceId(0);
        let helper_instance = InstanceId(1);
        let main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_display_name".to_string(),
                    target: HirVarTarget::Function(helper_id),
                }),
                ty: Type::function(Vec::new(), Type::Unit),
                span: Span::test(),
            },
        );
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                helper_instance,
                InstanceOrigin::Function(helper_id),
                "helper",
                "helper",
                Some(unit_return_function(helper_id, "helper")),
                false,
            ),
        ]);

        prune_unreachable_instances(&mut program);

        assert!(program.instances.contains_key(&helper_instance));
    }

    #[test]
    fn prune_unreachable_instances_does_not_map_source_name_var_to_zero_substitution_instance() {
        let main_id = DefId::new(CrateId(0), LocalDefId(100));
        let helper_id = DefId::new(CrateId(0), LocalDefId(101));
        let main_instance = InstanceId(0);
        let helper_instance = InstanceId(1);
        let main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Var("helper".to_string()),
                ty: Type::function(Vec::new(), Type::Unit),
                span: Span::test(),
            },
        );
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                helper_instance,
                InstanceOrigin::Function(helper_id),
                "helper",
                "helper",
                Some(unit_return_function(helper_id, "helper")),
                false,
            ),
        ]);

        prune_unreachable_instances(&mut program);

        assert!(!program.instances.contains_key(&helper_instance));
    }

    #[test]
    fn prune_unreachable_instances_roots_main_without_display_name_table() {
        let main_id = DefId::new(CrateId(0), LocalDefId(105));
        let helper_id = DefId::new(CrateId(0), LocalDefId(106));
        let main_instance = InstanceId(0);
        let helper_instance = InstanceId(1);
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(unit_return_function(main_id, "main")),
                false,
            ),
            instance_record(
                helper_instance,
                InstanceOrigin::Function(helper_id),
                "helper",
                "helper",
                Some(unit_return_function(helper_id, "helper")),
                false,
            ),
        ]);
        program.program.names.functions_by_name.clear();

        prune_unreachable_instances(&mut program);

        assert!(program.instances.contains_key(&main_instance));
        assert!(!program.instances.contains_key(&helper_instance));
    }

    #[test]
    fn prune_unreachable_instances_does_not_map_non_callable_var_name() {
        let main_id = DefId::new(CrateId(0), LocalDefId(120));
        let helper_id = DefId::new(CrateId(0), LocalDefId(121));
        let main_instance = InstanceId(0);
        let helper_instance = InstanceId(1);
        let main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Var("helper".to_string()),
                ty: Type::I64,
                span: Span::test(),
            },
        );
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                helper_instance,
                InstanceOrigin::Function(helper_id),
                "helper",
                "helper",
                Some(unit_return_function(helper_id, "helper")),
                false,
            ),
        ]);

        prune_unreachable_instances(&mut program);

        assert!(!program.instances.contains_key(&helper_instance));
    }

    #[test]
    fn prune_unreachable_instances_does_not_map_callable_var_by_name() {
        let main_id = DefId::new(CrateId(0), LocalDefId(122));
        let helper_id = DefId::new(CrateId(0), LocalDefId(123));
        let main_instance = InstanceId(0);
        let helper_instance = InstanceId(1);
        let main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Var("helper".to_string()),
                ty: Type::function(Vec::new(), Type::Unit),
                span: Span::test(),
            },
        );
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                helper_instance,
                InstanceOrigin::Function(helper_id),
                "helper",
                "helper",
                Some(unit_return_function(helper_id, "helper")),
                false,
            ),
        ]);

        prune_unreachable_instances(&mut program);

        assert!(!program.instances.contains_key(&helper_instance));
    }

    #[test]
    fn prune_unreachable_instances_does_not_map_callable_var_by_backend_symbol() {
        let main_id = DefId::new(CrateId(0), LocalDefId(124));
        let helper_id = DefId::new(CrateId(0), LocalDefId(125));
        let main_instance = InstanceId(0);
        let helper_instance = InstanceId(1);
        let main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Var("helper_backend".to_string()),
                ty: Type::function(Vec::new(), Type::Unit),
                span: Span::test(),
            },
        );
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                helper_instance,
                InstanceOrigin::Function(helper_id),
                "helper_source",
                "helper_backend",
                Some(unit_return_function(helper_id, "helper_source")),
                false,
            ),
        ]);

        prune_unreachable_instances(&mut program);

        assert!(!program.instances.contains_key(&helper_instance));
    }

    #[test]
    fn prune_unreachable_instances_does_not_map_callable_var_to_declared_alias() {
        let main_id = DefId::new(CrateId(0), LocalDefId(140));
        let impl_id = DefId::new(CrateId(1), LocalDefId(141));
        let method_id = DefId::new(CrateId(1), LocalDefId(142));
        let main_instance = InstanceId(0);
        let method_instance = InstanceId(1);
        let main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Call(
                    Box::new(HirExpr {
                        kind: HirExprKind::Var("String_from_str".to_string()),
                        ty: Type::function(Vec::new(), Type::Unit),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    None,
                ),
                ty: Type::Unit,
                span: Span::test(),
            },
        );
        let declared = unit_return_function(method_id, "stdlib::from_str");
        let object_method = InstanceRecord {
            id: method_instance,
            origin: InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(impl_id),
                method: method_id,
            },
            substitution: Vec::new(),
            symbols: InstanceSymbols::new("stdlib__String_from_str", "stdlib__String_from_str"),
            declared: Some(declared),
            provided_by_object: true,
            is_specialization: false,
        };
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            object_method,
        ]);

        prune_unreachable_instances(&mut program);

        assert!(!program.instances.contains_key(&method_instance));
    }

    #[test]
    fn prune_unreachable_instances_does_not_map_callable_var_to_impl_alias() {
        let main_id = DefId::new(CrateId(0), LocalDefId(150));
        let impl_id = DefId::new(CrateId(1), LocalDefId(151));
        let method_id = DefId::new(CrateId(1), LocalDefId(152));
        let main_instance = InstanceId(0);
        let method_instance = InstanceId(1);
        let main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Call(
                    Box::new(HirExpr {
                        kind: HirExprKind::Var("String_from_str".to_string()),
                        ty: Type::function(Vec::new(), Type::Unit),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    None,
                ),
                ty: Type::Unit,
                span: Span::test(),
            },
        );
        let object_method = InstanceRecord {
            id: method_instance,
            origin: InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(impl_id),
                method: method_id,
            },
            substitution: Vec::new(),
            symbols: InstanceSymbols::new("stdlib__String_from_str", "stdlib__String_from_str"),
            declared: Some(unit_return_function(method_id, "stdlib::from_str")),
            provided_by_object: true,
            is_specialization: false,
        };
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            object_method,
        ]);
        program.program.impls.insert(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("String".to_string()),
                type_name: "String".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: Vec::new().into(),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "from_str".to_string(),
                    unit_return_function(method_id, "stdlib::from_str"),
                )]),
            },
        );

        prune_unreachable_instances(&mut program);

        assert!(!program.instances.contains_key(&method_instance));
    }

    #[test]
    fn prune_unreachable_instances_keeps_drop_impl_for_automatic_drop() {
        let main_id = DefId::new(CrateId(0), LocalDefId(170));
        let drop_trait_id = DefId::new(CrateId(0), LocalDefId(171));
        let drop_trait_method_id = DefId::new(CrateId(0), LocalDefId(172));
        let impl_id = DefId::new(CrateId(0), LocalDefId(173));
        let drop_method_id = DefId::new(CrateId(0), LocalDefId(174));
        let main_instance = InstanceId(0);
        let drop_instance = InstanceId(1);
        let drop_method = unit_return_function(drop_method_id, "Box_drop");
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(unit_return_function(main_id, "main")),
                false,
            ),
            instance_record(
                drop_instance,
                InstanceOrigin::ImplMethod {
                    owner: InstanceImplOwner::Named(impl_id),
                    method: drop_method_id,
                },
                "Box_drop",
                "Box_drop",
                Some(drop_method.clone()),
                false,
            ),
        ]);
        program.program.traits.insert(
            drop_trait_id,
            crate::hir::HirTraitFor::<AcceptedHir> {
                target: None,
                predicates: Vec::new(),
                id: drop_trait_id,
                name: "Drop".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "drop".to_string(),
                    crate::hir::HirFunctionSig {
                        id: drop_trait_method_id,
                        name: "drop".to_string(),
                        generic_params: Vec::new(),
                        params: Vec::new(),
                        ret: Type::Unit,
                        generic_bounds: HashMap::new().into(),
                        self_receiver: Some(crate::types::ReceiverMode::Move),
                        is_unsafe: false,
                    },
                )]),
            },
        );
        program.program.impls.insert(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: Vec::new().into(),
                trait_name: Some("Drop".to_string()),
                trait_id: Some(drop_trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("drop".to_string(), drop_method)]),
            },
        );
        program
            .program
            .rebuild_indexes_with_canonical_names(&HashMap::from([
                (main_id, "main".to_string()),
                (drop_trait_id, "stdlib::drop::Drop".to_string()),
            ]));

        let mut bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut program);
        bodies.insert_runtime_instance_root(drop_instance);
        super::prune_unreachable_instances(&mut program, &mut bodies);

        assert!(program.instances.contains_key(&drop_instance));
    }

    #[test]
    fn prune_unreachable_instances_does_not_map_field_access_to_receiver_method_alias() {
        let main_id = DefId::new(CrateId(0), LocalDefId(170));
        let struct_id = DefId::new(CrateId(0), LocalDefId(171));
        let impl_id = DefId::new(CrateId(1), LocalDefId(171));
        let method_id = DefId::new(CrateId(1), LocalDefId(172));
        let field_id = FieldId(0);
        let main_instance = InstanceId(0);
        let method_instance = InstanceId(1);
        let receiver_ty = Type::Struct {
            id: struct_id,
            args: Vec::new(),
        };
        let mut main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Call(
                    Box::new(HirExpr {
                        kind: HirExprKind::FieldAccess(
                            Box::new(HirExpr {
                                kind: HirExprKind::Var("foo".to_string()),
                                ty: receiver_ty.clone(),
                                span: Span::test(),
                            }),
                            "println".to_string(),
                            Some(callable_field_location(struct_id, field_id, "println")),
                        ),
                        ty: Type::function(Vec::new(), Type::Unit),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    None,
                ),
                ty: Type::Unit,
                span: Span::test(),
            },
        );
        add_param(&mut main_body, "foo", receiver_ty);
        let mut method = unit_return_function(method_id, "I64_println");
        method.is_method = true;
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                method_instance,
                InstanceOrigin::ImplMethod {
                    owner: InstanceImplOwner::Named(impl_id),
                    method: method_id,
                },
                "I64_println",
                "I64_println",
                Some(method),
                false,
            ),
        ]);
        program.program.structs.insert(
            struct_id,
            callable_field_struct(struct_id, "Printer", field_id, "println"),
        );

        prune_unreachable_instances(&mut program);

        assert!(!program.instances.contains_key(&method_instance));
    }

    #[test]
    fn prune_unreachable_instances_does_not_map_field_access_to_method_alias_without_target() {
        let main_id = DefId::new(CrateId(0), LocalDefId(174));
        let struct_id = DefId::new(CrateId(0), LocalDefId(177));
        let impl_id = DefId::new(CrateId(1), LocalDefId(175));
        let method_id = DefId::new(CrateId(1), LocalDefId(176));
        let field_id = FieldId(0);
        let main_instance = InstanceId(0);
        let method_instance = InstanceId(1);
        let receiver_ty = Type::Struct {
            id: struct_id,
            args: Vec::new(),
        };
        let mut main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Call(
                    Box::new(HirExpr {
                        kind: HirExprKind::FieldAccess(
                            Box::new(HirExpr {
                                kind: HirExprKind::Var("foo".to_string()),
                                ty: receiver_ty.clone(),
                                span: Span::test(),
                            }),
                            "println".to_string(),
                            Some(callable_field_location(struct_id, field_id, "println")),
                        ),
                        ty: Type::function(Vec::new(), Type::Unit),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    None,
                ),
                ty: Type::Unit,
                span: Span::test(),
            },
        );
        add_param(&mut main_body, "foo", receiver_ty);
        let mut method = unit_return_function(method_id, "I64_println");
        method.is_method = true;
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                method_instance,
                InstanceOrigin::ImplMethod {
                    owner: InstanceImplOwner::Named(impl_id),
                    method: method_id,
                },
                "I64_println",
                "I64_println",
                Some(method),
                false,
            ),
        ]);
        program.program.structs.insert(
            struct_id,
            callable_field_struct(struct_id, "Printer", field_id, "println"),
        );

        prune_unreachable_instances(&mut program);

        assert!(!program.instances.contains_key(&method_instance));
    }

    #[test]
    fn prune_unreachable_instances_does_not_map_field_access_to_short_nominal_method_alias() {
        let main_id = DefId::new(CrateId(0), LocalDefId(180));
        let struct_id = DefId::new(CrateId(0), LocalDefId(181));
        let method_id = DefId::new(CrateId(0), LocalDefId(182));
        let field_id = FieldId(0);
        let main_instance = InstanceId(0);
        let method_instance = InstanceId(1);
        let receiver_ty = Type::Struct {
            id: struct_id,
            args: Vec::new(),
        };
        let mut main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Call(
                    Box::new(HirExpr {
                        kind: HirExprKind::FieldAccess(
                            Box::new(HirExpr {
                                kind: HirExprKind::Var("foo".to_string()),
                                ty: receiver_ty.clone(),
                                span: Span::test(),
                            }),
                            "show".to_string(),
                            Some(callable_field_location(struct_id, field_id, "show")),
                        ),
                        ty: Type::function(Vec::new(), Type::Unit),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    None,
                ),
                ty: Type::Unit,
                span: Span::test(),
            },
        );
        add_param(&mut main_body, "foo", receiver_ty);
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                method_instance,
                InstanceOrigin::ImplMethod {
                    owner: InstanceImplOwner::Named(DefId::new(CrateId(0), LocalDefId(183))),
                    method: method_id,
                },
                "Foo_show",
                "Foo_show",
                Some(unit_return_function(method_id, "Foo_show")),
                false,
            ),
        ]);
        program.program.structs.insert(
            struct_id,
            callable_field_struct(struct_id, "Foo", field_id, "show"),
        );
        program
            .program
            .indexes
            .structs_by_id
            .insert(struct_id, "module::Foo".to_string());

        prune_unreachable_instances(&mut program);

        assert!(!program.instances.contains_key(&method_instance));
    }

    #[test]
    fn prune_unreachable_instances_does_not_map_field_access_to_named_nominal_alias() {
        let main_id = DefId::new(CrateId(0), LocalDefId(190));
        let struct_id = DefId::new(CrateId(0), LocalDefId(191));
        let method_id = DefId::new(CrateId(0), LocalDefId(192));
        let field_id = FieldId(0);
        let main_instance = InstanceId(0);
        let method_instance = InstanceId(1);
        let receiver_ty = Type::Struct {
            id: struct_id,
            args: Vec::new(),
        };
        let mut main_body = function_returning_expr(
            main_id,
            "main",
            HirExpr {
                kind: HirExprKind::Call(
                    Box::new(HirExpr {
                        kind: HirExprKind::FieldAccess(
                            Box::new(HirExpr {
                                kind: HirExprKind::Var("foo".to_string()),
                                ty: receiver_ty.clone(),
                                span: Span::test(),
                            }),
                            "show".to_string(),
                            Some(callable_field_location(struct_id, field_id, "show")),
                        ),
                        ty: Type::function(Vec::new(), Type::Unit),
                        span: Span::test(),
                    }),
                    Vec::new(),
                    None,
                ),
                ty: Type::Unit,
                span: Span::test(),
            },
        );
        add_param(&mut main_body, "foo", receiver_ty);
        let mut program = monomorphized_with_instances(vec![
            instance_record(
                main_instance,
                InstanceOrigin::Function(main_id),
                "main",
                "main",
                Some(main_body),
                false,
            ),
            instance_record(
                method_instance,
                InstanceOrigin::ImplMethod {
                    owner: InstanceImplOwner::Named(DefId::new(CrateId(0), LocalDefId(193))),
                    method: method_id,
                },
                "AliasFoo_show",
                "AliasFoo_show",
                Some(unit_return_function(method_id, "AliasFoo_show")),
                false,
            ),
        ]);
        program.program.structs.insert(
            struct_id,
            callable_field_struct(struct_id, "Foo", field_id, "show"),
        );
        program
            .program
            .indexes
            .structs_by_id
            .insert(struct_id, "module::Foo".to_string());
        program
            .program
            .names
            .structs_by_name
            .insert("AliasFoo".to_string(), struct_id);

        prune_unreachable_instances(&mut program);

        assert!(!program.instances.contains_key(&method_instance));
    }
}
