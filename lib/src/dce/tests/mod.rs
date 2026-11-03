mod edges;
mod name_mapping;
mod roots;

use std::collections::HashMap;

use crate::hir::{
    AcceptedHir, AcceptedHirBlock as HirBlock, AcceptedHirExpr as HirExpr,
    AcceptedHirFunction as HirFunction, AcceptedHirImpl as HirImpl, HirCallTarget, HirField,
    HirFieldLocation, HirImplOwner, HirNameTables, HirParam, HirStruct, HirVarRef, HirVarTarget,
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
