use super::*;

#[test]
fn compile_mir_program_rejects_drop_glue_contract_without_receiver_param() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let struct_id = DefId::new(CrateId(0), LocalDefId(82));
    let box_type = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let box_type_id = type_id(&mut type_context, box_type);
    let function_id = DefId::new(CrateId(0), LocalDefId(83));
    let instance_id = InstanceId(83);
    let drop_instance = InstanceId(84);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "drop_contract_missing_receiver".to_string(),
        basic_blocks: vec![
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Drop {
                    place: crate::mir::Place {
                        local: crate::mir::Local(1),
                        projection: vec![],
                    },
                    target: crate::mir::BasicBlockId(1),
                }),
            },
            crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(0),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            },
        ],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            crate::mir::LocalDecl {
                ty: box_type_id,
                mutability: crate::mir::Mutability::Not,
                name: Some("box_value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        crate::mir::MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "drop_contract_missing_receiver".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract.callables.insert(
        drop_key.clone(),
        crate::mir::MirCallableDecl {
            key: drop_key.clone(),
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(84))),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "Box_drop".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                type_id(&mut type_context, Type::Unit),
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    backend_contract.drop_glue.insert(box_type_id, drop_key);
    backend_contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), i64_type)],
            generic_params: Vec::new(),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("receiver parameter"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_unused_drop_glue_contract_with_bad_signature() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let struct_id = DefId::new(CrateId(0), LocalDefId(182));
    let box_type_id = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    let function_id = DefId::new(CrateId(0), LocalDefId(183));
    let instance_id = InstanceId(183);
    let drop_instance = InstanceId(184);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "unused_bad_drop_contract".to_string(),
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
    let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
    let mut backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "unused_bad_drop_contract",
        Vec::new(),
        i64_type,
        false,
    );
    backend_contract.callables.insert(
        drop_key.clone(),
        crate::mir::MirCallableDecl {
            key: drop_key.clone(),
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(184))),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "Box_drop".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[box_type_id, i64_type],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    backend_contract.drop_glue.insert(box_type_id, drop_key);
    backend_contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), i64_type)],
            generic_params: Vec::new(),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("expected 1 receiver parameter")
            && err.message.contains("expected ()"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_unused_drop_glue_contract_with_unknown_type_id() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let unit_type = type_id(&mut type_context, Type::Unit);
    let struct_id = DefId::new(CrateId(0), LocalDefId(185));
    let box_type_id = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    let unknown_type = crate::ids::TypeId(u32::MAX);
    let function_id = DefId::new(CrateId(0), LocalDefId(186));
    let instance_id = InstanceId(186);
    let drop_instance = InstanceId(187);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "unused_unknown_drop_contract".to_string(),
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
    let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
    let mut backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "unused_unknown_drop_contract",
        Vec::new(),
        i64_type,
        false,
    );
    backend_contract.callables.insert(
        drop_key.clone(),
        crate::mir::MirCallableDecl {
            key: drop_key.clone(),
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(187))),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "Box_drop".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[box_type_id],
                unit_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    backend_contract.drop_glue.insert(unknown_type, drop_key);
    backend_contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), i64_type)],
            generic_params: Vec::new(),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("unknown dropped type"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_unused_drop_glue_contract_with_unknown_return_type_id() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let struct_id = DefId::new(CrateId(0), LocalDefId(188));
    let box_type_id = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    let unknown_type = crate::ids::TypeId(u32::MAX);
    let function_id = DefId::new(CrateId(0), LocalDefId(189));
    let instance_id = InstanceId(189);
    let drop_instance = InstanceId(190);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "unused_unknown_return_drop_contract".to_string(),
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
    let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
    let mut backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "unused_unknown_return_drop_contract",
        Vec::new(),
        i64_type,
        false,
    );
    backend_contract.callables.insert(
        drop_key.clone(),
        crate::mir::MirCallableDecl {
            key: drop_key.clone(),
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(190))),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "Box_drop".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[box_type_id],
                unknown_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    backend_contract.drop_glue.insert(box_type_id, drop_key);
    backend_contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), i64_type)],
            generic_params: Vec::new(),
        },
    );
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

#[test]
fn compile_mir_program_rejects_drop_glue_contract_with_wrong_receiver_pass_mode() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let unit_type = type_id(&mut type_context, Type::Unit);
    let struct_id = DefId::new(CrateId(0), LocalDefId(85));
    let box_type_id = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    let function_id = DefId::new(CrateId(0), LocalDefId(86));
    let instance_id = InstanceId(86);
    let drop_instance = InstanceId(87);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "drop_contract_wrong_receiver".to_string(),
        basic_blocks: vec![
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Drop {
                    place: crate::mir::Place {
                        local: crate::mir::Local(1),
                        projection: vec![],
                    },
                    target: crate::mir::BasicBlockId(1),
                }),
            },
            crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(0),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            },
        ],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            crate::mir::LocalDecl {
                ty: box_type_id,
                mutability: crate::mir::Mutability::Not,
                name: Some("box_value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        crate::mir::MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "drop_contract_wrong_receiver".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract.callables.insert(
        drop_key.clone(),
        crate::mir::MirCallableDecl {
            key: drop_key.clone(),
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(87))),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "Box_drop".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature {
                params: vec![crate::mir::MirParamAbi {
                    semantic_ty: box_type_id,
                    pass_mode: crate::mir::MirPassMode::Pointer,
                }],
                ret: crate::mir::MirReturnAbi {
                    semantic_ty: unit_type,
                    abi_ty: unit_type,
                },
            },
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    backend_contract.drop_glue.insert(box_type_id, drop_key);
    backend_contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), i64_type)],
            generic_params: Vec::new(),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("receiver pass mode"),
        "unexpected error: {err}"
    );
}
