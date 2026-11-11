use super::*;

#[test]
fn build_monomorphized_uses_explicit_instance_body_table() {
    use crate::mono::{InstanceOrigin, InstanceRecord, MonomorphizedProgram};

    let function_id = DefId::new(CrateId(0), LocalDefId(9));
    let instance_id = InstanceId(9);
    let backend_symbol = "main__explicit_body";
    let function = empty_function(function_id, "main");
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(function_id, function.clone())]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([("main".to_string(), function_id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(function_id, "main".to_string())]),
    );
    let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
    pre_mir_instance_bodies.insert(instance_id, function);
    let mut monomorphized = MonomorphizedProgram {
        program,
        instances: std::collections::BTreeMap::from([(
            instance_id,
            InstanceRecord {
                id: instance_id,
                origin: InstanceOrigin::Function(function_id),
                substitution: Vec::new(),
                symbols: crate::mono::InstanceSymbols::new(
                    "main".to_string(),
                    backend_symbol.to_string(),
                ),
                declared: None,
                provided_by_object: false,
                is_specialization: false,
            },
        )]),
        pre_mir_instance_bodies,
        generated_drop_instances: Default::default(),
        type_context: crate::type_context::TypeContext::new(),
        source_map: crate::source_map::SemanticSourceMap::default(),
    };
    let mut bodies = MirBuilder::take_mir_instance_bodies(&mut monomorphized).unwrap();
    let unit_ty = monomorphized.type_context.intern_type(&Type::Unit);
    let closure_id = crate::mir::MirFunctionId::Closure(Box::new(crate::mir::MirClosureId {
        owner: crate::mir::MirFunctionId::Instance(instance_id),
        local_index: 0,
    }));
    bodies.push_nested(MirFunction {
        id: closure_id.clone(),
        name: "lambda_0".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: unit_ty,
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit_ty,
        ownership: Default::default(),
    });

    let mir = MirBuilder::build_monomorphized_with_instance_bodies(&monomorphized, &bodies).unwrap();

    let id = crate::mir::MirFunctionId::Instance(instance_id);
    let function = mir.function(id.clone()).expect("instance MIR body");
    assert_eq!(function.id, id);
    assert!(mir
        .backend_contract
        .artifact_exports
        .iter()
        .any(|export| export.backend_symbol == backend_symbol && export.has_body));
    let key = crate::mir::MirCallableKey::Instance(instance_id);
    assert!(mir.backend_contract.callable(&key).is_some());
    assert_eq!(mir.backend_contract.function_bodies.get(&id), Some(&key));
    let closure_key = crate::mir::MirCallableKey::Closure(closure_id.clone());
    assert!(mir.backend_contract.callable(&closure_key).is_some());
    assert_eq!(
        mir.backend_contract.function_bodies.get(&closure_id),
        Some(&closure_key),
    );
}

#[test]
fn take_mir_instance_bodies_converts_and_drains_hir_instance_bodies() {
    use crate::mono::{InstanceOrigin, InstanceRecord, MonomorphizedProgram};

    let function_id = DefId::new(CrateId(0), LocalDefId(12));
    let instance_id = InstanceId(12);
    let function = empty_function(function_id, "answer");
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(function_id, function.clone())]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([(
                "answer".to_string(),
                function_id,
            )]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(function_id, "answer".to_string())]),
    );
    let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
    pre_mir_instance_bodies.insert(instance_id, function.clone());
    let mut monomorphized = MonomorphizedProgram {
        program,
        instances: std::collections::BTreeMap::from([(
            instance_id,
            InstanceRecord {
                id: instance_id,
                origin: InstanceOrigin::Function(function_id),
                substitution: Vec::new(),
                symbols: crate::mono::InstanceSymbols::new(
                    "answer".to_string(),
                    "answer__i64".to_string(),
                ),
                declared: Some(function.clone()),
                provided_by_object: false,
                is_specialization: false,
            },
        )]),
        pre_mir_instance_bodies,
        generated_drop_instances: Default::default(),
        type_context: crate::type_context::TypeContext::new(),
        source_map: crate::source_map::SemanticSourceMap::default(),
    };

    let bodies = MirBuilder::take_mir_instance_bodies(&mut monomorphized).unwrap();

    assert!(monomorphized
        .pre_mir_instance_bodies
        .get(instance_id)
        .is_none());
    let body = bodies.get(instance_id).expect("MIR instance body");
    assert_eq!(
        body.function.id,
        crate::mir::MirFunctionId::Instance(instance_id)
    );
}

#[test]
fn build_monomorphized_program_keys_functions_by_instance_id() {
    use crate::mono::{InstanceOrigin, InstanceRecord, MonomorphizedProgram};

    let function_id = DefId::new(CrateId(0), LocalDefId(10));
    let instance_id = InstanceId(7);
    let backend_symbol = "main__i64";
    let function = HirFunction {
        id: function_id,
        name: "main".to_string(),
        generic_params: Vec::new(),
        generic_bounds: std::collections::HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::I64,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::IntLiteral(42),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            })],
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(function_id, function.clone())]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([("main".to_string(), function_id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(function_id, "main".to_string())]),
    );
    let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
    pre_mir_instance_bodies.insert(instance_id, function);
    let mut instances = std::collections::BTreeMap::new();
    instances.insert(
        instance_id,
        InstanceRecord {
            id: instance_id,
            origin: InstanceOrigin::Function(function_id),
            substitution: Vec::new(),
            symbols: crate::mono::InstanceSymbols::new(
                "main".to_string(),
                backend_symbol.to_string(),
            ),
            declared: None,
            provided_by_object: false,
            is_specialization: false,
        },
    );
    let monomorphized = MonomorphizedProgram {
        program,
        instances,
        pre_mir_instance_bodies,
        generated_drop_instances: Default::default(),
        type_context: crate::type_context::TypeContext::new(),
        source_map: crate::source_map::SemanticSourceMap::default(),
    };

    let mir = MirBuilder::build_monomorphized(&monomorphized).unwrap();

    let id = crate::mir::MirFunctionId::Instance(instance_id);
    let function = mir.function(id.clone()).expect("instance MIR body");
    assert_eq!(function.id, id);
    assert_eq!(function.name, backend_symbol);
}

#[test]
fn mir_builder_records_type_ids_for_return_locals_and_casts() {
    use crate::hir::{HirVarRef, HirVarTarget};
    use crate::mono::{InstanceOrigin, InstanceRecord, MonomorphizedProgram};

    let function_id = DefId::new(CrateId(0), LocalDefId(80));
    let instance_id = InstanceId(80);
    let local_id = crate::ids::HirLocalId(0);
    let cast_value = expr(
        HirExprKind::Cast(
            Box::new(expr(HirExprKind::IntLiteral(1), Type::I64)),
            Type::U64,
        ),
        Type::U64,
    );
    let body = HirBlock {
        stmts: vec![
            HirStmt::Let {
                name: "x".to_string(),
                local_id,
                ty: Type::U64,
                value: cast_value,
                mutable: false,
            },
            HirStmt::Expr(expr(
                HirExprKind::ResolvedVar(HirVarRef {
                    name: "x".to_string(),
                    target: HirVarTarget::Local(local_id),
                }),
                Type::U64,
            )),
        ],
        ty: Type::U64,
    };
    let function = HirFunction {
        id: function_id,
        name: "main".to_string(),
        generic_params: Vec::new(),
        generic_bounds: std::collections::HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::U64,
        body,
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(function_id, function.clone())]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([("main".to_string(), function_id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(function_id, "main".to_string())]),
    );
    let mut type_context = crate::type_context::TypeContext::new();
    let _ = crate::hir::collect_hir_type_ids(&program, &mut type_context);
    let u64_id = type_context.id_for_type(&Type::U64).unwrap();
    let mut pre_mir_instance_bodies = crate::mono::PreMirInstanceBodies::new();
    pre_mir_instance_bodies.insert(instance_id, function);
    let mut instances = std::collections::BTreeMap::new();
    instances.insert(
        instance_id,
        InstanceRecord {
            id: instance_id,
            origin: InstanceOrigin::Function(function_id),
            substitution: Vec::new(),
            symbols: crate::mono::InstanceSymbols::new("main".to_string(), "main".to_string()),
            declared: None,
            provided_by_object: false,
            is_specialization: false,
        },
    );
    let monomorphized = MonomorphizedProgram {
        program,
        instances,
        pre_mir_instance_bodies,
        generated_drop_instances: Default::default(),
        type_context,
        source_map: crate::source_map::SemanticSourceMap::default(),
    };

    let mir = MirBuilder::build_monomorphized(&monomorphized).unwrap();
    let main = mir
        .function(crate::mir::MirFunctionId::Instance(instance_id))
        .unwrap();
    let view = crate::type_context::TypeView::new(&monomorphized.type_context);

    assert_eq!(main.ret_type, u64_id);
    assert!(main.local_decls.iter().any(|local| local.ty == u64_id));
    assert!(main
        .basic_blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|stmt| {
            matches!(
                &stmt.kind,
                crate::mir::StatementKind::Assign(_, crate::mir::Rvalue::Cast(_, target))
                    if *target == u64_id && matches!(view.ty(*target), crate::type_context::Ty::U64)
            )
        }));
}
