use super::*;

#[test]
fn agreement_rejects_resolved_callable_key_missing_from_backend_contract() {
    let caller = DefId::new(CrateId(0), LocalDefId(40));
    let function_id = MirFunctionId::Function(caller);
    let missing_key = MirCallableKey::Instance(InstanceId(401));
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    missing_key.clone(),
                ))),
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
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let caller_key = MirCallableKey::Function(caller);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(caller),
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
        .insert(function_id.clone(), caller_key);
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_backend_contract, 1);
    assert_eq!(report.missing_callable_metadata, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_missing_function_callable_metadata() {
    let id = DefId::new(CrateId(0), LocalDefId(9));
    let missing = DefId::new(CrateId(0), LocalDefId(10));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    MirCallableKey::Function(missing),
                ))),
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
            source: crate::mir::LocalSource::ReturnPlace,
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

    assert_eq!(report.missing_callable_metadata, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_accepts_resolved_instance_callable_backed_by_contract() {
    let id = DefId::new(CrateId(0), LocalDefId(11));
    let dependency = DefId::new(CrateId(1), LocalDefId(12));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    MirCallableKey::Instance(InstanceId(2)),
                ))),
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
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let mut backend_contract = MirBackendContract::default();
    let dependency_key = MirCallableKey::Instance(InstanceId(2));
    backend_contract.callables.insert(
        dependency_key.clone(),
        MirCallableDecl {
            key: dependency_key,
            source_def_id: Some(dependency),
            kind: MirCallableKind::ObjectProvided,
            llvm_symbol: "dependency::make".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], unit, MirPassMode::Direct),
        },
    );
    let caller_key = MirCallableKey::Function(id);
    backend_contract.callables.insert(
        caller_key.clone(),
        MirCallableDecl {
            key: caller_key.clone(),
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
        .insert(function_id.clone(), caller_key);
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
fn agreement_rejects_function_body_without_callable_symbol_metadata() {
    let caller = DefId::new(CrateId(0), LocalDefId(13));
    let callee = DefId::new(CrateId(0), LocalDefId(14));
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let caller_function = MirFunction {
        id: MirFunctionId::Function(caller),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    MirCallableKey::Function(callee),
                ))),
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
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let callee_function = MirFunction {
        id: MirFunctionId::Function(callee),
        name: "callee".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([
            (MirFunctionId::Function(caller), caller_function),
            (MirFunctionId::Function(callee), callee_function),
        ]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_callable_metadata, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_instance_body_without_callable_symbol_metadata() {
    let caller = DefId::new(CrateId(0), LocalDefId(15));
    let instance = InstanceId(16);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let caller_function = MirFunction {
        id: MirFunctionId::Function(caller),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    MirCallableKey::Instance(instance),
                ))),
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
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let instance_function = MirFunction {
        id: MirFunctionId::Instance(instance),
        name: "instance".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([
            (MirFunctionId::Function(caller), caller_function),
            (MirFunctionId::Instance(instance), instance_function),
        ]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_callable_metadata, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_missing_callable_metadata_in_assignment_operand() {
    let id = DefId::new(CrateId(0), LocalDefId(22));
    let missing = DefId::new(CrateId(0), LocalDefId(23));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let callable_ty = type_id(&mut type_context, Type::function(vec![], Type::Unit));
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![],
                },
                Rvalue::Use(Operand::Constant(Constant::Callable(
                    MirCallable::Resolved(MirCallableKey::Function(missing)),
                ))),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: callable_ty,
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

    assert_eq!(report.missing_callable_metadata, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_missing_callable_metadata_in_indirect_call_argument() {
    let id = DefId::new(CrateId(0), LocalDefId(24));
    let missing = DefId::new(CrateId(0), LocalDefId(25));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let callable_ty = type_id(&mut type_context, Type::function(vec![], Type::Unit));
    let callee_ty = type_context.intern_ty(Ty::Function {
        params: vec![callable_ty],
        ret: unit,
        safety: FunctionSafety::Safe,
        callable_kind: crate::types::CallableKind::Fn,
        captures: Vec::new(),
    });
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
                args: vec![Operand::Constant(Constant::Callable(
                    MirCallable::Resolved(MirCallableKey::Function(missing)),
                ))],
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
                ty: callee_ty,
                mutability: Mutability::Not,
                name: Some("callee".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: unit,
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_callable_metadata, 1);
    assert!(!report.is_clean());
}
