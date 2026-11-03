use super::*;

#[test]
fn mir_codegen_lowers_control_flow() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(0));
    let ret_type = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "control_flow".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Goto(crate::mir::BasicBlockId(1))),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::SwitchInt {
                    discr: Operand::Copy(root_place(1)),
                    targets: vec![
                        (0, crate::mir::BasicBlockId(2)),
                        (1, crate::mir::BasicBlockId(3)),
                    ],
                    otherwise: crate::mir::BasicBlockId(4),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Drop {
                    place: root_place(0),
                    target: crate::mir::BasicBlockId(4),
                }),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(7))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(0))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: type_id(&type_context, Type::I64),
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            local_decl(&type_context, Type::I64, "drop_place"),
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type,
        ownership: Default::default(),
    };
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract: local_body_contract(function_id, "control_flow", ret_type),
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("switch_eq"));
    assert!(ir.contains("br i1"));
    assert!(ir.contains("br label"));
    assert_ir_order(&ir, &["entry:", "br label %bb1", "bb1:"]);
    assert_ir_order(
        &ir,
        &[
            "bb1:",
            "switch_eq = icmp eq",
            "br i1",
            "label %bb2, label %switch.next0",
            "switch.next0:",
            "switch_eq1 = icmp eq",
            "br i1",
            "label %bb3, label %switch.next1",
            "switch.next1:",
            "br label %bb4",
        ],
    );
    assert_ir_order(&ir, &["bb2:", "br label %bb4", "bb3:"]);
    assert!(!ir.contains("drop_place_hir"));
}

#[test]
fn mir_codegen_reports_invalid_terminator_target() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(0));
    let ret_type = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "bad_target".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Goto(crate::mir::BasicBlockId(99))),
        }],
        local_decls: vec![local_decl(&type_context, Type::I64, "return_place")],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type,
        ownership: Default::default(),
    };
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract: local_body_contract(function_id, "bad_target", ret_type),
    };

    let error = codegen.compile_program_from_mir(&mir).unwrap_err();
    assert!(error.message.contains("invalid MIR basic block target"));
}

#[test]
fn mir_codegen_lowers_canonical_calls() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let callee_instance = InstanceId(7);
    let callee_key = MirCallableKey::Instance(callee_instance);
    let callee_symbol = "canonical_callee";
    let callable_ty = Type::function(vec![Type::I64], Type::I64);
    let i64_id = type_id(&type_context, Type::I64);

    let function_id = MirFunctionId::Instance(InstanceId(0));
    let function = MirFunction {
        id: function_id.clone(),
        name: "canonical_calls".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(1),
                    Rvalue::Use(Operand::Constant(Constant::Callable(
                        MirCallable::Resolved(callee_key.clone()),
                    ))),
                    None,
                )],
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        callee_key.clone(),
                    ))),
                    args: vec![Operand::Constant(Constant::Int(41))],
                    destination: root_place(0),
                    target: crate::mir::BasicBlockId(1),
                    span: None,
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, callable_ty, "callable_value"),
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_id,
        ownership: Default::default(),
    };
    let mut backend_contract = local_body_contract(function_id.clone(), "canonical_calls", i64_id);
    insert_object_callable_contract(
        &mut backend_contract,
        callee_key.clone(),
        callee_symbol,
        &[i64_id],
        i64_id,
    );
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("__callable_wrap_canonical_callee_0"));
    assert!(ir.contains("call i64 @canonical_callee(i64 41)"));
    assert!(ir.contains("store i64 %mir_call, ptr %return_place"));

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let higher_order_instance = InstanceId(8);
    let callback_instance = InstanceId(9);
    let higher_order_key = MirCallableKey::Instance(higher_order_instance);
    let callback_key = MirCallableKey::Instance(callback_instance);
    let higher_order_symbol = "higher_order_callee";
    let callback_symbol = "callback_callee";
    let callback_ty = Type::function(vec![Type::I64], Type::I64);
    let i64_id = type_id(&type_context, Type::I64);
    let callback_type_id = type_id(&type_context, callback_ty.clone());

    let function_id = MirFunctionId::Instance(InstanceId(0));
    let function = MirFunction {
        id: function_id.clone(),
        name: "canonical_callable_arg".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        higher_order_key.clone(),
                    ))),
                    args: vec![Operand::Constant(Constant::Callable(
                        MirCallable::Resolved(callback_key.clone()),
                    ))],
                    destination: root_place(0),
                    target: crate::mir::BasicBlockId(1),
                    span: None,
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![local_decl(&type_context, Type::I64, "return_place")],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_id,
        ownership: Default::default(),
    };
    let mut backend_contract =
        local_body_contract(function_id.clone(), "canonical_callable_arg", i64_id);
    insert_object_callable_contract(
        &mut backend_contract,
        higher_order_key.clone(),
        higher_order_symbol,
        &[callback_type_id],
        i64_id,
    );
    insert_object_callable_contract(
        &mut backend_contract,
        callback_key.clone(),
        callback_symbol,
        &[i64_id],
        i64_id,
    );
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("__callable_wrap_callback_callee_0"));
    assert!(ir.contains("call i64 @higher_order_callee"));

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let callback_instance = InstanceId(10);
    let callback_key = MirCallableKey::Instance(callback_instance);
    let callback_symbol = "pointer_abi_callback";
    let callback_ty = Type::function(vec![Type::I64], Type::I64);
    let i64_id = type_id(&type_context, Type::I64);

    let callback_signature = MirCallableSignature {
        params: vec![crate::mir::MirParamAbi {
            semantic_ty: i64_id,
            pass_mode: MirPassMode::Pointer,
        }],
        ret: crate::mir::MirReturnAbi {
            semantic_ty: i64_id,
            abi_ty: i64_id,
        },
    };

    let function_id = MirFunctionId::Instance(InstanceId(0));
    let function = MirFunction {
        id: function_id.clone(),
        name: "pointer_abi_callable_value".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                root_place(1),
                Rvalue::Use(Operand::Constant(Constant::Callable(
                    MirCallable::Resolved(callback_key.clone()),
                ))),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, callback_ty, "callable_value"),
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_id,
        ownership: Default::default(),
    };
    let mut backend_contract =
        local_body_contract(function_id.clone(), "pointer_abi_callable_value", i64_id);
    insert_object_callable_contract_with_signature(
        &mut backend_contract,
        callback_key.clone(),
        callback_symbol,
        callback_signature,
    );
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("__callable_wrap_pointer_abi_callback_0"));
    assert!(ir.contains("call i64 @pointer_abi_callback(ptr %mir_pointer_arg"));

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(0));
    let missing_instance = InstanceId(99);
    let ret_type = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "unknown_canonical_call".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    MirCallableKey::Instance(missing_instance),
                ))),
                args: vec![],
                destination: root_place(0),
                target: crate::mir::BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![local_decl(&type_context, Type::I64, "return_place")],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type,
        ownership: Default::default(),
    };
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract: local_body_contract(function_id, "unknown_canonical_call", ret_type),
    };

    let error = codegen.compile_program_from_mir(&mir).unwrap_err();
    assert!(error.message.contains("ResolvedCallableWithoutContract"));
    assert!(error.message.contains("Instance(InstanceId(99))"));
}

#[test]
fn mir_codegen_uses_contract_return_abi_for_main() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(0));
    let ret_type = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: ret_type,
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type,
        ownership: Default::default(),
    };
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract: local_body_contract(function_id, "main", ret_type),
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("define internal i64 @__rock_main"));
    assert!(ir.contains("define i32 @main(i32"));
    assert!(ir.contains("call i64 @__rock_main()"));
    assert!(ir.contains("define i64 @__rock_runtime_arg_count()"));
    assert!(ir.contains("define ptr @__rock_runtime_arg_at(i64"));
}

#[test]
fn mir_codegen_declares_and_verifies_empty_return_function() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(0));
    let ret_type = type_id(&type_context, Type::Unit);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: ret_type,
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type,
        ownership: Default::default(),
    };
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract: local_body_contract(function_id, "main", ret_type),
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("define internal void @__rock_main"));
    assert!(ir.contains("define i32 @main(i32"));
    assert!(ir.contains("call void @__rock_main()"));
    assert!(ir.contains("ret void"));
    assert!(ir.contains("ret i32 0"));

    let mut malformed = mir.clone();
    malformed.backend_contract = local_body_contract_with_return_abi(
        MirFunctionId::Instance(InstanceId(0)),
        "main",
        ret_type,
        type_id(&type_context, Type::I32),
    );
    let errors = crate::mir::backend_contract::validate_backend_contract_against_mir(
        &malformed.backend_contract,
        malformed.functions.values(),
    );
    assert!(errors.iter().any(|error| matches!(
            error,
            crate::mir::backend_contract::MirBackendContractError::LocalBodyReturnAbiTypeMismatch { .. },
        )));
}

#[test]
fn mir_codegen_returns_constant_assignment_to_return_place() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(0));
    let ret_type = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    42,
                ))),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: type_id(&type_context, Type::I64),
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type,
        ownership: Default::default(),
    };
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract: local_body_contract(function_id, "main", ret_type),
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("define internal i64 @__rock_main"));
    assert!(ir.contains("ret i64 42"));
    assert!(ir.contains("trunc i64 %rock_main to i32"));
    assert!(!ir.contains("store i64 42"));
}

#[test]
fn mir_codegen_binds_parameters_to_argument_locals() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(0));
    let i64_id = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "identity".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Copy(crate::mir::Place {
                    local: crate::mir::Local(1),
                    projection: vec![],
                })),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Not,
                name: Some("arg".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_id,
        ownership: Default::default(),
    };
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract: local_body_contract_with_params(
            function_id,
            "identity",
            &[i64_id],
            i64_id,
        ),
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("define i64 @identity(i64"));
    assert!(ir.contains("store i64 %0, ptr %arg"));
    assert!(ir.contains("load i64, ptr %arg"));
    assert!(ir.contains("store i64 %mir_place, ptr %return_place"));
}

#[test]
fn mir_codegen_sanitizes_invalid_local_display_names() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(0));
    let ret_type = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "bad_local".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: ret_type,
            mutability: Mutability::Mut,
            name: Some("bad\0local".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type,
        ownership: Default::default(),
    };
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract: local_body_contract(function_id, "bad_local", ret_type),
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();
    assert!(codegen.current_function.is_none());
    assert!(codegen.get_ir().contains("%local0 = alloca i64"));
}
