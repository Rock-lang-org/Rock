use super::*;

#[test]
fn agreement_rejects_unit_callable_placeholder_assignment() {
    let id = DefId::new(CrateId(0), LocalDefId(1));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![],
                },
                Rvalue::Use(Operand::Constant(Constant::Unit)),
                None,
            )],
            terminator: None,
        }],
        local_decls: vec![LocalDecl {
            ty: type_id(&mut type_context, Type::function(vec![], Type::Unit)),
            mutability: Mutability::Not,
            name: Some("callable".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: type_id(&mut type_context, Type::Unit),
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.non_unit_placeholder_units, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_allows_unit_assignment_to_projected_unit_field() {
    let id = DefId::new(CrateId(0), LocalDefId(2));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let tuple = type_id(&mut type_context, Type::Tuple(vec![Type::Unit, Type::I64]));
    let unit = type_id(&mut type_context, Type::Unit);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(1),
                    projection: vec![Projection::Field {
                        index: 0,
                        identity: None,
                    }],
                },
                Rvalue::Use(Operand::Constant(Constant::Unit)),
                None,
            )],
            terminator: None,
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: tuple,
                mutability: Mutability::Not,
                name: Some("tuple".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let callable_key = MirCallableKey::Function(id);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        callable_key.clone(),
        MirCallableDecl {
            key: callable_key.clone(),
            source_def_id: Some(id),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "main".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], unit, MirPassMode::Direct),
        },
    );
    backend_contract
        .function_bodies
        .insert(function_id.clone(), callable_key);
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert!(
        report.is_clean(),
        "unexpected MIR agreement report: {report:?}"
    );
}

#[test]
fn agreement_rejects_unit_callable_placeholder_call_terminator() {
    let id = DefId::new(CrateId(0), LocalDefId(3));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Unit),
                args: vec![],
                destination: Place {
                    local: Local(0),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![LocalDecl {
            ty: type_id(&mut type_context, Type::Unit),
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: type_id(&mut type_context, Type::Unit),
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.non_unit_placeholder_units, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_unit_local_callable_placeholder_call_terminator() {
    let id = DefId::new(CrateId(0), LocalDefId(4));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Copy(Place {
                    local: Local(0),
                    projection: vec![],
                }),
                args: vec![],
                destination: Place {
                    local: Local(1),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![
            LocalDecl {
                ty: type_id(&mut type_context, Type::Unit),
                mutability: Mutability::Not,
                name: Some("callable".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: type_id(&mut type_context, Type::Unit),
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: type_id(&mut type_context, Type::Unit),
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.non_unit_placeholder_units, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_bounds_check_assertion_missing_index_operand() {
    let id = DefId::new(CrateId(0), LocalDefId(5));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let array_base = Place {
        local: Local(1),
        projection: vec![],
    };
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::new(
                StatementKind::Assert(MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![Operand::Copy(array_base)],
                }),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: type_id(&mut type_context, Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Value(4))),
                mutability: Mutability::Not,
                name: Some("array".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let key = MirCallableKey::Function(id);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(id),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "main".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], unit, MirPassMode::Direct),
        },
    );
    backend_contract
        .function_bodies
        .insert(function_id.clone(), key);
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.malformed_runtime_requirements, 1);
    assert_eq!(report.missing_runtime_requirements, 0);
    assert_eq!(report.stale_runtime_requirements, 0);
    assert_eq!(report.invalid_backend_contract, 0);
    assert!(
        !report.is_clean(),
        "unexpected clean agreement report: {report:?}"
    );
}

#[test]
fn agreement_rejects_callable_assignment_to_non_function_destination() {
    let mut program = intrinsic_call_program(
        crate::mir::MirIntrinsicId::SizeOf,
        vec![Operand::Constant(Constant::TypeId(TypeId(0)))],
    );
    let function = program.functions.values_mut().next().unwrap();
    function.basic_blocks[0] = BasicBlock {
        statements: vec![StatementData::assign(
            Place {
                local: Local(0),
                projection: vec![],
            },
            Rvalue::Use(Operand::Constant(Constant::Callable(
                MirCallable::Resolved(MirCallableKey::Intrinsic(
                    crate::mir::MirIntrinsicId::SizeOf,
                )),
            ))),
            None,
        )],
        terminator: Some(Terminator::Return),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_constant_operands, 1);
}
