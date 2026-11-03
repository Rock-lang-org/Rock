use super::*;

#[test]
fn mir_closure_symbols_include_canonical_owner_identity() {
    let left = MirClosureId {
        owner: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(50))),
        local_index: 0,
    };
    let right = MirClosureId {
        owner: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(51))),
        local_index: 0,
    };

    let left_symbol = CodeGen::mir_closure_symbol("a.b", &left);
    let right_symbol = CodeGen::mir_closure_symbol("a_b", &right);

    assert_ne!(left_symbol, right_symbol);
}

#[test]
fn mir_codegen_lowers_closure_values_from_mir_bodies() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let parent_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(40)));
    let callable_ty = Type::function(vec![Type::I64], Type::I64);
    let callable_type_id = type_id(&type_context, callable_ty.clone());
    let i64_type = type_id(&type_context, Type::I64);
    let closure_id = MirClosureId {
        owner: parent_id.clone(),
        local_index: 0,
    };
    let closure_function_id = MirFunctionId::Closure(Box::new(closure_id.clone()));
    let closure_symbol = CodeGen::mir_closure_symbol("make_closure", &closure_id);
    let parent_capture = MirClosureCapture {
        name: "captured".to_string(),
        local: Local(1),
        kind: MirClosureCaptureKind::ByValue,
        span: None,
    };
    let closure_capture = MirClosureCapture {
        name: "captured".to_string(),
        local: Local(2),
        kind: MirClosureCaptureKind::ByValue,
        span: None,
    };

    let parent = MirFunction {
        id: parent_id.clone(),
        name: "make_closure".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
                    root_place(1),
                    Rvalue::Use(Operand::Constant(Constant::Int(41))),
                    None,
                ),
                StatementData::assign(
                    root_place(0),
                    Rvalue::Closure(MirClosure {
                        id: closure_id.clone(),
                        display_name: "make_closure.closure0".to_string(),
                        captures: vec![parent_capture],
                    }),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            local_decl(&type_context, callable_ty.clone(), "return_place"),
            local_decl(&type_context, Type::I64, "captured"),
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: callable_type_id,
        ownership: Default::default(),
    };
    let closure = MirFunction {
        id: closure_function_id.clone(),
        name: "make_closure.closure0".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Copy(root_place(2))),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, Type::I64, "arg"),
            local_decl(&type_context, Type::I64, "captured"),
        ],
        closure_captures: vec![closure_capture],
        arg_count: 1,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let mut backend_contract =
        local_body_contract(parent_id.clone(), "make_closure", callable_type_id);
    insert_local_body_contract(
        &mut backend_contract,
        closure_function_id.clone(),
        &closure_symbol,
        &[i64_type],
        i64_type,
    );
    backend_contract
        .runtime_requirements
        .insert(crate::mir::MirRuntimeHelper::HeapAlloc);
    let mir = MirProgram {
        functions: BTreeMap::from([(parent_id.clone(), parent), (closure_function_id, closure)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("__mir_closure"));
    assert!(ir.contains("callable_code"));
    assert!(ir.contains("callable_env"));
    assert!(ir.contains("mir_closure_env_alloc"));
    assert!(ir.contains("mir_closure_capture"));
    assert!(!ir.contains("__lambda_"));
}

#[test]
fn mir_codegen_accepts_resolved_closure_callable_constants() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let parent_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(41)));
    let callable_ty = Type::function(vec![], Type::I64);
    let callable_type_id = type_id(&type_context, callable_ty.clone());
    let i64_type = type_id(&type_context, Type::I64);
    let closure_id = MirClosureId {
        owner: parent_id.clone(),
        local_index: 0,
    };
    let closure_function_id = MirFunctionId::Closure(Box::new(closure_id.clone()));
    let closure_symbol = CodeGen::mir_closure_symbol("bad_closure_constant", &closure_id);
    let parent = MirFunction {
        id: parent_id.clone(),
        name: "bad_closure_constant".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Constant(Constant::Callable(
                    MirCallable::Resolved(MirCallableKey::Closure(closure_function_id.clone())),
                ))),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![local_decl(
            &type_context,
            callable_ty.clone(),
            "return_place",
        )],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: callable_type_id,
        ownership: Default::default(),
    };
    let closure = MirFunction {
        id: closure_function_id.clone(),
        name: "bad_closure_constant.closure0".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Constant(Constant::Int(7))),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![local_decl(&type_context, Type::I64, "return_place")],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let mut backend_contract =
        local_body_contract(parent_id.clone(), "bad_closure_constant", callable_type_id);
    insert_local_body_contract(
        &mut backend_contract,
        closure_function_id.clone(),
        &closure_symbol,
        &[],
        i64_type,
    );
    let mir = MirProgram {
        functions: BTreeMap::from([(parent_id.clone(), parent), (closure_function_id, closure)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();
}

#[test]
fn mir_codegen_rejects_closure_signature_mismatch() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let parent_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(42)));
    let expected_callable_ty = Type::function(vec![Type::I32], Type::I64);
    let expected_callable_type_id = type_id(&type_context, expected_callable_ty.clone());
    let i64_type = type_id(&type_context, Type::I64);
    let closure_id = MirClosureId {
        owner: parent_id.clone(),
        local_index: 0,
    };
    let closure_function_id = MirFunctionId::Closure(Box::new(closure_id.clone()));
    let closure_symbol = CodeGen::mir_closure_symbol("bad_closure_signature", &closure_id);
    let parent = MirFunction {
        id: parent_id.clone(),
        name: "bad_closure_signature".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                root_place(0),
                Rvalue::Closure(MirClosure {
                    id: closure_id,
                    display_name: "bad_closure_signature.closure0".to_string(),
                    captures: Vec::new(),
                }),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![local_decl(
            &type_context,
            expected_callable_ty.clone(),
            "return_place",
        )],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: expected_callable_type_id,
        ownership: Default::default(),
    };
    let closure = MirFunction {
        id: closure_function_id.clone(),
        name: "bad_closure_signature.closure0".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Constant(Constant::Int(7))),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, Type::I64, "arg"),
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let mut backend_contract = local_body_contract(
        parent_id.clone(),
        "bad_closure_signature",
        expected_callable_type_id,
    );
    insert_local_body_contract(
        &mut backend_contract,
        closure_function_id.clone(),
        &closure_symbol,
        &[i64_type],
        i64_type,
    );
    let mir = MirProgram {
        functions: BTreeMap::from([(parent_id.clone(), parent), (closure_function_id, closure)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    let error = codegen.compile_program_from_mir(&mir).unwrap_err();
    assert!(error.message.contains("signature"));
    assert!(!codegen.get_ir().contains("callable_code"));
}

#[test]
fn mir_codegen_rejects_closure_capture_layout_mismatch() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let parent_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(43)));
    let callable_ty = Type::function(vec![], Type::I64);
    let callable_type_id = type_id(&type_context, callable_ty.clone());
    let i64_type = type_id(&type_context, Type::I64);
    let closure_id = MirClosureId {
        owner: parent_id.clone(),
        local_index: 0,
    };
    let closure_function_id = MirFunctionId::Closure(Box::new(closure_id.clone()));
    let closure_symbol = CodeGen::mir_closure_symbol("bad_closure_capture_layout", &closure_id);
    let parent_capture = MirClosureCapture {
        name: "captured".to_string(),
        local: Local(1),
        kind: MirClosureCaptureKind::ByValue,
        span: None,
    };
    let closure_capture = MirClosureCapture {
        name: "captured".to_string(),
        local: Local(1),
        kind: MirClosureCaptureKind::ByValue,
        span: None,
    };
    let parent = MirFunction {
        id: parent_id.clone(),
        name: "bad_closure_capture_layout".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
                    root_place(1),
                    Rvalue::Use(Operand::Constant(Constant::Int(11))),
                    None,
                ),
                StatementData::assign(
                    root_place(0),
                    Rvalue::Closure(MirClosure {
                        id: closure_id,
                        display_name: "bad_closure_capture_layout.closure0".to_string(),
                        captures: vec![parent_capture],
                    }),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            local_decl(&type_context, callable_ty.clone(), "return_place"),
            local_decl(&type_context, Type::I64, "captured"),
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: callable_type_id,
        ownership: Default::default(),
    };
    let closure = MirFunction {
        id: closure_function_id.clone(),
        name: "bad_closure_capture_layout.closure0".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Constant(Constant::Int(7))),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, Type::I32, "captured"),
        ],
        closure_captures: vec![closure_capture],
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let mut backend_contract = local_body_contract(
        parent_id.clone(),
        "bad_closure_capture_layout",
        callable_type_id,
    );
    insert_local_body_contract(
        &mut backend_contract,
        closure_function_id.clone(),
        &closure_symbol,
        &[],
        i64_type,
    );
    backend_contract
        .runtime_requirements
        .insert(MirRuntimeHelper::HeapAlloc);
    let mir = MirProgram {
        functions: BTreeMap::from([(parent_id.clone(), parent), (closure_function_id, closure)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    let error = codegen.compile_program_from_mir(&mir).unwrap_err();
    assert!(error.message.contains("capture layout"));
    assert!(!codegen.get_ir().contains("mir_closure_env_alloc"));
}
