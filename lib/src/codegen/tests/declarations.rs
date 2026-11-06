use super::*;

#[test]
fn mangle_function_symbol_distinguishes_separator_and_underscore_paths() {
    let first = CodeGen::mangle_function_symbol("foo::bar__baz");
    let second = CodeGen::mangle_function_symbol("foo__bar::baz");

    assert_eq!(first, "s3_666f6f_s8_6261725f5f62617a");
    assert_eq!(second, "s8_666f6f5f5f626172_s3_62617a");
    assert_ne!(first, second);
}

#[test]
fn compile_mir_program_uses_mir_body() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(CrateId(0), LocalDefId(1));
    let instance_id = InstanceId(0);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    type_id(&mut type_context, Type::I32);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    99,
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
    let backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "main",
        Vec::new(),
        i64_type,
        false,
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("ret i64 99"), "IR was:\n{}", ir);
    assert!(!ir.contains("ret i64 1"), "IR was:\n{}", ir);
}

#[test]
fn mir_llvm_module_exposes_focused_compile_api() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(91));
    let instance_id = InstanceId(91);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    type_id(&mut type_context, Type::I32);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    7,
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
    let backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "main",
        Vec::new(),
        i64_type,
        false,
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    super::super::mir_llvm::compile_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("ret i64 7"), "IR was:\n{}", ir);
}

#[test]
fn compile_mir_program_declares_instance_from_mir_body_without_hir_body() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(CrateId(0), LocalDefId(7));
    let instance_id = InstanceId(7);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    type_id(&mut type_context, Type::I32);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
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
    let backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "main",
        Vec::new(),
        i64_type,
        false,
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("ret i64 42"), "IR was:\n{}", ir);
}

#[test]
fn compile_mir_program_declares_instance_from_backend_contract() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(CrateId(0), LocalDefId(97));
    let instance_id = InstanceId(97);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "contract_instance".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    123,
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
    let key = crate::mir::MirCallableKey::Instance(instance_id);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        key.clone(),
        crate::mir::MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "contract_instance_symbol".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract.function_bodies.insert(mir_id.clone(), key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(
        ir.contains("define i64 @contract_instance_symbol()"),
        "IR was:\n{}",
        ir
    );
    assert!(ir.contains("ret i64 123"), "IR was:\n{}", ir);
}

#[test]
fn compile_mir_program_lowers_resolved_intrinsic_from_backend_contract() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(CrateId(0), LocalDefId(200));
    let instance_id = InstanceId(201);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let intrinsic_key = crate::mir::MirCallableKey::Intrinsic(crate::mir::MirIntrinsicId::I64Add);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "resolved_intrinsic_contract".to_string(),
        basic_blocks: vec![
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Call {
                    func: crate::mir::Operand::Constant(crate::mir::Constant::Callable(
                        crate::mir::MirCallable::Resolved(intrinsic_key.clone()),
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
            llvm_symbol: "resolved_intrinsic_contract".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract.callables.insert(
        intrinsic_key.clone(),
        crate::mir::MirCallableDecl {
            key: intrinsic_key.clone(),
            source_def_id: None,
            kind: crate::mir::MirCallableKind::Intrinsic(crate::mir::MirIntrinsicId::I64Add),
            llvm_symbol: "I64Add".to_string(),
            linkage: crate::mir::MirLinkage::Internal,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[i64_type, i64_type],
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

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(codegen.callables.symbol_for_key(&intrinsic_key).is_some());
    assert!(!ir.contains("call i64 @I64Add"), "IR was:\n{}", ir);
    assert!(ir.contains("ret i64"), "IR was:\n{}", ir);
}

#[test]
fn compile_mir_program_declares_extern_from_backend_contract_without_hir_extern() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let extern_id = DefId::new(CrateId(0), LocalDefId(90));
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let extern_key = crate::mir::MirCallableKey::Extern(extern_id);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        extern_key.clone(),
        crate::mir::MirCallableDecl {
            key: extern_key,
            source_def_id: Some(extern_id),
            kind: crate::mir::MirCallableKind::Extern {
                link_name: "ffi::answer".to_string(),
                variadic: false,
            },
            llvm_symbol: "ffi::answer".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[i64_type],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::new(),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    assert!(codegen.module.get_function("answer").is_some());
    assert_eq!(
        codegen
            .callables
            .symbol_for_key(&crate::mir::MirCallableKey::Extern(extern_id))
            .map(String::as_str),
        Some("ffi::answer")
    );
    assert!(codegen.functions.contains_key("ffi::answer"));
}

#[test]
fn compile_mir_program_registers_struct_layout_from_backend_contract_without_hir_struct() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let struct_id = DefId::new(CrateId(0), LocalDefId(91));
    let instance_id = InstanceId(91);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let point_type = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "make_point".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: Vec::new(),
                },
                crate::mir::Rvalue::Aggregate(
                    crate::mir::AggregateKind::Struct {
                        id: struct_id,
                        display_name: "Point".to_string(),
                    },
                    vec![
                        crate::mir::Operand::Constant(crate::mir::Constant::Int(1)),
                        crate::mir::Operand::Constant(crate::mir::Constant::Int(2)),
                    ],
                ),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: point_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: point_type,
        ownership: Default::default(),
    };
    let mut backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        DefId::new(CrateId(0), LocalDefId(92)),
        "make_point",
        Vec::new(),
        point_type,
        false,
    );
    backend_contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("x".to_string(), i64_type), ("y".to_string(), i64_type)],
            generic_params: Vec::new(),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("{ i64 1, i64 2 }"), "IR was:\n{}", ir);
    assert_eq!(
        codegen.nominal_layouts.struct_fields(&struct_id),
        Some(&vec![
            ("x".to_string(), i64_type),
            ("y".to_string(), i64_type),
        ])
    );
}

#[test]
fn compile_mir_program_declares_closure_callable_from_backend_contract() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let callable_ty = Type::function(vec![Type::I64], Type::I64);
    let callable_type = type_id(&mut type_context, callable_ty.clone());
    let parent_def = DefId::new(CrateId(0), LocalDefId(88));
    let parent_id = crate::mir::MirFunctionId::Function(parent_def);
    let closure_id = crate::mir::MirClosureId {
        owner: parent_id.clone(),
        local_index: 0,
    };
    let closure_function_id = crate::mir::MirFunctionId::Closure(Box::new(closure_id.clone()));
    let parent_capture = crate::mir::MirClosureCapture {
        name: "captured".to_string(),
        local: crate::mir::Local(1),
        kind: crate::mir::MirClosureCaptureKind::ByValue,
        span: None,
    };
    let closure_capture = crate::mir::MirClosureCapture {
        name: "captured".to_string(),
        local: crate::mir::Local(2),
        kind: crate::mir::MirClosureCaptureKind::ByValue,
        span: None,
    };
    let parent = crate::mir::MirFunction {
        id: parent_id.clone(),
        name: "make_contract_closure".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![
                crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(1),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(5),
                    )),
                    None,
                ),
                crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Closure(crate::mir::MirClosure {
                        id: closure_id.clone(),
                        display_name: "make_contract_closure.closure0".to_string(),
                        captures: vec![parent_capture],
                    }),
                    None,
                ),
            ],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: callable_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Not,
                name: Some("captured".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: callable_type,
        ownership: Default::default(),
    };
    let closure = crate::mir::MirFunction {
        id: closure_function_id.clone(),
        name: "make_contract_closure.closure0".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Copy(crate::mir::Place {
                    local: crate::mir::Local(2),
                    projection: vec![],
                })),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Not,
                name: Some("arg".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Not,
                name: Some("captured".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![closure_capture],
        arg_count: 1,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let parent_key = crate::mir::MirCallableKey::Function(parent_def);
    let closure_key = crate::mir::MirCallableKey::Closure(closure_function_id.clone());
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        parent_key.clone(),
        crate::mir::MirCallableDecl {
            key: parent_key.clone(),
            source_def_id: Some(parent_def),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: parent_id.clone(),
            },
            llvm_symbol: "make_contract_closure".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                callable_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract.callables.insert(
        closure_key.clone(),
        crate::mir::MirCallableDecl {
            key: closure_key.clone(),
            source_def_id: None,
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: closure_function_id.clone(),
            },
            llvm_symbol: "contract_closure_symbol".to_string(),
            linkage: crate::mir::MirLinkage::Internal,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[i64_type],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(parent_id.clone(), parent_key);
    backend_contract
        .function_bodies
        .insert(closure_function_id.clone(), closure_key);
    backend_contract
        .runtime_requirements
        .insert(crate::mir::MirRuntimeHelper::HeapAlloc);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(parent_id, parent), (closure_function_id, closure)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(
        ir.contains("define internal i64 @contract_closure_symbol(ptr %0, i64 %1)"),
        "IR was:\n{}",
        ir,
    );
    assert!(!ir.contains("__mir_closure"), "IR was:\n{}", ir);
}
