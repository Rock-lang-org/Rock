use super::*;

#[test]
fn declare_runtime_only_declares_contract_requirements() {
    let context = Context::create();

    let mut bounds_codegen = CodeGen::new(&context, "bounds");
    bounds_codegen.declare_runtime(&BTreeSet::from([crate::mir::MirRuntimeHelper::BoundsCheck]));
    assert!(bounds_codegen.module.get_function("puts").is_some());
    assert!(bounds_codegen.module.get_function("exit").is_some());
    assert!(bounds_codegen.module.get_function("malloc").is_none());

    let mut heap_codegen = CodeGen::new(&context, "heap");
    heap_codegen.declare_runtime(&BTreeSet::from([crate::mir::MirRuntimeHelper::HeapAlloc]));
    assert!(heap_codegen.module.get_function("puts").is_none());
    assert!(heap_codegen.module.get_function("exit").is_none());
    assert!(heap_codegen.module.get_function("malloc").is_some());

    let mut empty_codegen = CodeGen::new(&context, "empty");
    empty_codegen.declare_runtime(&BTreeSet::new());
    assert!(empty_codegen.module.get_function("puts").is_none());
    assert!(empty_codegen.module.get_function("exit").is_none());
    assert!(empty_codegen.module.get_function("malloc").is_none());
}

#[test]
fn compile_mir_program_rejects_missing_heap_allocation_requirement_before_lowering() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "missing_heap_requirement");
    let instance_id = InstanceId(301);
    let function_id = DefId::new(CrateId(0), LocalDefId(301));
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let closure_id = crate::mir::MirClosureId {
        owner: mir_id.clone(),
        local_index: 0,
    };
    let mut type_context = crate::type_context::TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let closure_ty = type_id(&mut type_context, Type::function(Vec::new(), Type::Unit));
    let function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(1),
                    projection: vec![],
                },
                crate::mir::Rvalue::Closure(crate::mir::MirClosure {
                    id: closure_id,
                    display_name: "captured".to_string(),
                    captures: vec![crate::mir::MirClosureCapture {
                        name: "capture".to_string(),
                        local: crate::mir::Local(0),
                        kind: crate::mir::MirClosureCaptureKind::ByValue,
                        span: None,
                    }],
                }),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: unit,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            crate::mir::LocalDecl {
                ty: closure_ty,
                mutability: crate::mir::Mutability::Mut,
                name: Some("closure".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, function)]),
        type_context: type_context.clone(),
        backend_contract: local_instance_contract(
            &type_context,
            instance_id,
            function_id,
            "main",
            Vec::new(),
            unit,
            false,
        ),
    };

    let error = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        error.message.contains("runtime requirement mismatch")
            && error.message.contains("HeapAlloc"),
        "unexpected error: {error}"
    );
}

#[test]
fn compile_mir_program_rejects_stale_heap_allocation_requirement_before_lowering() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "stale_heap_requirement");
    let instance_id = InstanceId(302);
    let function_id = DefId::new(CrateId(0), LocalDefId(302));
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mut type_context = crate::type_context::TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: unit,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let mut backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "main",
        Vec::new(),
        unit,
        false,
    );
    backend_contract
        .runtime_requirements
        .insert(crate::mir::MirRuntimeHelper::HeapAlloc);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, function)]),
        type_context,
        backend_contract,
    };

    let error = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        error.message.contains("runtime requirement mismatch")
            && error.message.contains("HeapAlloc"),
        "unexpected error: {error}"
    );
}

#[test]
fn mir_callable_resolved_key_missing_from_contract_fails_without_name_lookup() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let instance = InstanceId(210);
    let present_key = crate::mir::MirCallableKey::Instance(instance);
    let missing_key = crate::mir::MirCallableKey::Instance(InstanceId(211));
    codegen.functions.insert(
        "Box_show_exact".to_string(),
        codegen.module.add_function(
            "Box_show_exact",
            context.i64_type().fn_type(&[], false),
            None,
        ),
    );
    codegen.functions.insert(
        "show".to_string(),
        codegen
            .module
            .add_function("show", context.i64_type().fn_type(&[], false), None),
    );
    codegen
        .callable_symbols_by_key
        .insert(present_key.clone(), "Box_show_exact".to_string());

    let symbol = codegen
        .resolve_mir_callable_symbol(&crate::mir::MirCallable::Resolved(present_key))
        .unwrap();

    assert_eq!(symbol, "Box_show_exact");

    let err = codegen
        .resolve_mir_callable_symbol(&crate::mir::MirCallable::Resolved(missing_key))
        .unwrap_err();

    assert!(
        err.message.contains("Unknown MIR callable key"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_missing_backend_contract() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let instance_id = InstanceId(98);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "missing_contract".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    1,
                ))),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_resolved_callable_missing_from_backend_contract() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(CrateId(0), LocalDefId(198));
    let instance_id = InstanceId(198);
    let missing_key = crate::mir::MirCallableKey::Instance(InstanceId(199));
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "missing_resolved_callable".to_string(),
        basic_blocks: vec![
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Call {
                    func: crate::mir::Operand::Constant(crate::mir::Constant::Callable(
                        crate::mir::MirCallable::Resolved(missing_key.clone()),
                    )),
                    args: vec![],
                    destination: crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: None,
                }),
            },
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Return),
            },
        ],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        crate::mir::MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "missing_resolved_callable".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("ResolvedCallableWithoutContract"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_wrong_callable_key_even_when_source_alias_registered() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let caller_def = DefId::new(CrateId(0), LocalDefId(201));
    let callee_def = DefId::new(CrateId(0), LocalDefId(202));
    let callee_instance = InstanceId(202);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let mir_id = crate::mir::MirFunctionId::Function(caller_def);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "source_alias_callable".to_string(),
        basic_blocks: vec![
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Call {
                    func: crate::mir::Operand::Constant(crate::mir::Constant::Callable(
                        crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Function(
                            callee_def,
                        )),
                    )),
                    args: vec![],
                    destination: crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: None,
                }),
            },
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Return),
            },
        ],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Function(caller_def);
    let callee_key = crate::mir::MirCallableKey::Instance(callee_instance);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        crate::mir::MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(caller_def),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "source_alias_callable".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract.callables.insert(
        callee_key.clone(),
        crate::mir::MirCallableDecl {
            key: callee_key,
            source_def_id: Some(callee_def),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "callee_alias_callable".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("ResolvedCallableWithoutContract"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_direct_intrinsic_callable_without_contract() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(CrateId(0), LocalDefId(199));
    let instance_id = InstanceId(200);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "direct_intrinsic_bypass".to_string(),
        basic_blocks: vec![
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Call {
                    func: crate::mir::Operand::Constant(crate::mir::Constant::Callable(
                        crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Intrinsic(
                            crate::mir::MirIntrinsicId::I64Add,
                        )),
                    )),
                    args: vec![
                        crate::mir::Operand::Constant(crate::mir::Constant::Int(1)),
                        crate::mir::Operand::Constant(crate::mir::Constant::Int(2)),
                    ],
                    destination: crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: None,
                }),
            },
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Return),
            },
        ],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        crate::mir::MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "direct_intrinsic_bypass".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            || err.message.contains("Unknown MIR callable"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_unused_callable_contract_with_unknown_type_id() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let unknown_type = crate::ids::TypeId(u32::MAX);
    let function_id = DefId::new(CrateId(0), LocalDefId(191));
    let instance_id = InstanceId(191);
    let extern_instance = InstanceId(192);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "unused_unknown_callable_contract".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    0,
                ))),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let extern_key = crate::mir::MirCallableKey::Instance(extern_instance);
    let mut backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "unused_unknown_callable_contract",
        Vec::new(),
        i64_type,
        false,
    );
    backend_contract.callables.insert(
        extern_key.clone(),
        crate::mir::MirCallableDecl {
            key: extern_key,
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(192))),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "unknown_return_extern".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[i64_type],
                unknown_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("unknown return semantic type"),
        "unexpected error: {err}"
    );
}
