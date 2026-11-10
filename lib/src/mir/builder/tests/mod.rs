use super::*;
use crate::hir::{
    BinOp, HirAssociatedTypeDef, HirExtern, HirField, HirImplOwner, HirImplReceiverPattern,
    HirLiteralPattern, HirNameTables, HirParam, HirPattern, HirStruct, HirVarRef, HirVarTarget,
    HirVariantFields, HirVariantLocation, UnaryOp,
};
use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, InstanceId, LocalDefId, TypeId, VariantId};
use crate::lexer::Span;
use crate::mir::{
    AggregateKind, Constant, LocalSource, MirCallable, MirCallableKey, MirClosureId,
    MirFunction, MirInstanceBodies, MirIntrinsicId, MirProgram, Mutability, Operand, Projection,
    ReferenceOrigin, Rvalue, StatementData, StatementKind, Terminator,
};
use crate::type_services::projection::ProjectionProvider;
use crate::types::{GenericParamDecl, GenericParamId, Type};

use super::projection_provider::MirProjectionResolutionProvider;

type HirMatchArm = crate::hir::HirMatchArmFor<AcceptedHir>;

type HirStructLiteralField = crate::hir::HirStructLiteralFieldFor<AcceptedHir>;

type HirTrait = crate::hir::HirTraitFor<AcceptedHir>;

fn span() -> Span {
    Span::test()
}

fn expr(kind: HirExprKind, ty: Type) -> HirExpr {
    HirExpr {
        kind,
        ty,
        span: span(),
    }
}

fn test_type_context(program: &HirProgram) -> RefCell<crate::type_context::TypeContext> {
    let mut type_context = crate::type_context::TypeContext::new();
    let _ = crate::hir::collect_hir_type_ids(program, &mut type_context);
    for ty in [
        Type::Unit,
        Type::I64,
        Type::I32,
        Type::U64,
        Type::F64,
        Type::Bool,
    ] {
        type_context.intern_type(&ty);
    }
    RefCell::new(type_context)
}

fn empty_program() -> HirProgram {
    let id = DefId::new(CrateId(0), LocalDefId(0));
    HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(
            id,
            HirStruct {
                id,
                name: "TestStruct".to_string(),
                generic_params: vec![],
                fields: vec![],
            },
        )]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            structs_by_name: std::collections::HashMap::from([("TestStruct".to_string(), id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(id, "TestStruct".to_string())]),
    )
}

fn empty_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: std::collections::HashMap::new().into(),
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

fn program_with_struct(fields: Vec<&str>) -> HirProgram {
    let struct_fields = fields
        .into_iter()
        .enumerate()
        .map(|(index, name)| HirField {
            id: FieldId(index as u32),
            name: name.to_string(),
            ty: Type::I64,
            public: true,
        })
        .collect();
    let id = DefId::new(CrateId(0), LocalDefId(0));
    let structs = std::collections::HashMap::from([(
        id,
        HirStruct {
            id,
            name: "Foo".to_string(),
            generic_params: vec![],
            fields: struct_fields,
        },
    )]);
    HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        structs,
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            structs_by_name: std::collections::HashMap::from([("Foo".to_string(), id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(id, "Foo".to_string())]),
    )
}

fn struct_ty(local: u32) -> Type {
    Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(local)),
        args: vec![],
    }
}

fn option_program(variant_fields: Vec<(VariantId, &str, HirVariantFields)>) -> HirProgram {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let variants = variant_fields
        .into_iter()
        .map(|(id, name, fields)| crate::hir::HirVariant {
            id,
            name: name.to_string(),
            fields,
        })
        .collect();
    HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(
            enum_id,
            crate::hir::HirEnum {
                id: enum_id,
                name: "Option".to_string(),
                generic_params: vec![],
                variants,
            },
        )]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            enums_by_name: std::collections::HashMap::from([("Option".to_string(), enum_id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(enum_id, "Option".to_string())]),
    )
}

fn option_ty() -> Type {
    Type::Enum {
        id: DefId::new(CrateId(0), LocalDefId(40)),
        args: vec![],
    }
}

fn match_arm(pattern: HirPattern, guard: Option<HirExpr>, body: HirExpr) -> HirMatchArm {
    HirMatchArm {
        pattern,
        guard,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(body.clone())],
            ty: body.ty,
        },
    }
}

fn builder_with_var<'a>(
    program: &'a HirProgram,
    type_context: &'a RefCell<crate::type_context::TypeContext>,
    name: &'a str,
    ty: Type,
) -> MirBuilder<'a> {
    let mut builder = MirBuilder::new(program, type_context);
    let ty_id = builder.type_id_for(&ty);
    builder.locals.push(LocalDecl {
        ty: ty_id,
        mutability: Mutability::Not,
        name: Some(name.to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.var_map.insert(name.to_string(), Local(0));
    builder
}

fn assert_builder_agreement_clean(builder: MirBuilder<'_>, ret_type: Type) {
    assert_builder_agreement_clean_with_contract(builder, ret_type, Default::default());
}

fn assert_builder_agreement_clean_with_contract(
    builder: MirBuilder<'_>,
    ret_type: Type,
    backend_contract: crate::mir::MirBackendContract,
) {
    let report = builder_agreement_report_with_contract(builder, ret_type, backend_contract);
    assert!(
        report.is_clean(),
        "unexpected MIR agreement report: {report:?}"
    );
}

fn builder_agreement_report_with_contract(
    mut builder: MirBuilder<'_>,
    ret_type: Type,
    backend_contract: crate::mir::MirBackendContract,
) -> crate::mir::agreement::MirAgreementReport {
    let closure_captures = builder.closures.take_captures();
    let id = DefId::new(CrateId(0), LocalDefId(999));
    let function_id = MirFunctionId::Function(id);
    let ret_type = builder
        .locals
        .first()
        .map(|local| local.ty)
        .unwrap_or_else(|| builder.type_id_for(&ret_type));
    let program = builder.program;
    let mut type_context = builder.type_context.borrow().clone();
    let functions = std::collections::BTreeMap::from([(
        function_id.clone(),
        MirFunction {
            id: function_id.clone(),
            name: "test".to_string(),
            basic_blocks: builder.blocks,
            local_decls: builder.locals,
            closure_captures,
            arg_count: 0,
            ret_type,
            ownership: builder.ownership,
        },
    )]);
    let mut backend_contract = backend_contract;
    MirBuilder::populate_backend_contract_nominal_layouts(
        &mut backend_contract,
        program,
        &mut type_context,
    );
    MirBuilder::populate_backend_contract_function_bodies(
        &mut backend_contract,
        &functions,
        &type_context,
    );
    MirBuilder::populate_backend_contract_runtime_requirements(&mut backend_contract, &functions);
    let mir = MirProgram {
        functions,
        type_context,
        backend_contract,
    };
    crate::mir::agreement::check_mir_runtime_agreement(&mir)
}

fn assert_mir_agreement_clean(mir: &MirProgram) {
    let report = crate::mir::agreement::check_mir_runtime_agreement(mir);
    assert!(
        report.is_clean(),
        "unexpected MIR agreement report: {report:?}"
    );
}

mod aggregates;
mod array_for_tail_drops;
mod backend_contract;
mod callables;
mod closures;
mod drops;
mod functions;
mod instances;
mod intrinsics;
mod match_moves;
mod matches;
mod operators;
mod places;
mod projections;
