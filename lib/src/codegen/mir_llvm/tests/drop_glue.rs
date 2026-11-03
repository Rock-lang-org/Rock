use super::*;

fn drop_glue_contract(
    dropped_ty: crate::ids::TypeId,
    key: MirCallableKey,
    source_def_id: DefId,
    symbol: &str,
    unit_ty: crate::ids::TypeId,
) -> MirBackendContract {
    let mut contract = MirBackendContract::default();
    contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(source_def_id),
            kind: MirCallableKind::ObjectProvided,
            llvm_symbol: symbol.to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(
                &[dropped_ty],
                unit_ty,
                MirPassMode::Direct,
            ),
        },
    );
    contract.drop_glue.insert(dropped_ty, key);
    contract
}

fn undeclarable_drop_glue_contract(
    dropped_ty: TypeId,
    key: MirCallableKey,
    symbol: &str,
    unit_ty: TypeId,
) -> MirBackendContract {
    let mut contract = MirBackendContract::default();
    contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: None,
            kind: MirCallableKind::RuntimeHelper(MirRuntimeHelper::DropGlue),
            llvm_symbol: symbol.to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(
                &[dropped_ty],
                unit_ty,
                MirPassMode::Direct,
            ),
        },
    );
    contract.drop_glue.insert(dropped_ty, key);
    contract
}

#[test]
fn mir_codegen_calls_drop_impl_before_drop_target() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(20));
    let struct_id = DefId::new(CrateId(0), LocalDefId(80));
    let drop_method_id = DefId::new(CrateId(0), LocalDefId(83));
    let box_ty = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let box_type_id = type_id(&type_context, box_ty.clone());
    let unit_type = type_id(&type_context, Type::Unit);
    let ret_type = type_id(&type_context, Type::I64);
    let field_type = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "drop_box".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(1),
                    Rvalue::Aggregate(
                        AggregateKind::Struct {
                            id: struct_id,
                            display_name: "Box".to_string(),
                        },
                        vec![Operand::Constant(Constant::Int(7))],
                    ),
                    None,
                )],
                terminator: Some(Terminator::Drop {
                    place: root_place(1),
                    target: crate::mir::BasicBlockId(1),
                }),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(Constant::Int(0))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, box_ty, "box_value"),
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type,
        ownership: Default::default(),
    };
    let mut backend_contract = drop_glue_contract(
        box_type_id,
        MirCallableKey::Instance(InstanceId(120)),
        drop_method_id,
        "Box_drop",
        unit_type,
    );
    insert_local_body_contract(
        &mut backend_contract,
        function_id.clone(),
        "drop_box",
        &[],
        ret_type,
    );
    insert_struct_layout_contract(
        &mut backend_contract,
        struct_id,
        vec![("value".to_string(), field_type)],
    );
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert_ir_order(&ir, &["call void @Box_drop", "br label %bb1"]);
}

#[test]
fn mir_codegen_rejects_drop_glue_contract_codegen_will_not_declare() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(22));
    let struct_id = DefId::new(CrateId(0), LocalDefId(87));
    let box_ty = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let box_type_id = type_id(&type_context, box_ty.clone());
    let unit_type = type_id(&type_context, Type::Unit);
    let ret_type = type_id(&type_context, Type::I64);
    let field_type = type_id(&type_context, Type::I64);

    let function = MirFunction {
        id: function_id.clone(),
        name: "drop_box_missing_glue".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(1),
                    Rvalue::Aggregate(
                        AggregateKind::Struct {
                            id: struct_id,
                            display_name: "Box".to_string(),
                        },
                        vec![Operand::Constant(Constant::Int(7))],
                    ),
                    None,
                )],
                terminator: Some(Terminator::Drop {
                    place: root_place(1),
                    target: crate::mir::BasicBlockId(1),
                }),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(Constant::Int(0))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, box_ty, "box_value"),
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type,
        ownership: Default::default(),
    };
    let mut backend_contract = undeclarable_drop_glue_contract(
        box_type_id,
        MirCallableKey::Instance(InstanceId(122)),
        "Box_drop",
        unit_type,
    );
    insert_local_body_contract(
        &mut backend_contract,
        function_id.clone(),
        "drop_box_missing_glue",
        &[],
        ret_type,
    );
    backend_contract.nominal_layouts.insert(
        struct_id,
        MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), field_type)],
            generic_params: Vec::new(),
        },
    );
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    let error = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        error.message.contains("Invalid MIR backend contract")
            && error.message.contains("DropGlueCallableKindMismatch"),
        "unexpected error: {}",
        error.message
    );
}

#[test]
fn mir_drop_without_required_glue_is_codegen_error() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(23));
    let struct_id = DefId::new(CrateId(0), LocalDefId(88));
    let box_ty = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let i64_ty = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "drop_box_without_glue".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(1),
                    Rvalue::Aggregate(
                        AggregateKind::Struct {
                            id: struct_id,
                            display_name: "Box".to_string(),
                        },
                        vec![Operand::Constant(Constant::Int(7))],
                    ),
                    None,
                )],
                terminator: Some(Terminator::Drop {
                    place: root_place(1),
                    target: crate::mir::BasicBlockId(1),
                }),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(Constant::Int(0))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, box_ty, "box_value"),
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: type_id(&type_context, Type::I64),
        ownership: Default::default(),
    };
    let mut backend_contract = local_body_contract(
        function_id.clone(),
        "drop_box_without_glue",
        type_id(&type_context, Type::I64),
    );
    backend_contract.nominal_layouts.insert(
        struct_id,
        MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), i64_ty)],
            generic_params: Vec::new(),
        },
    );
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    let error = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        error
            .message
            .contains("Missing drop glue contract for MIR drop")
            && error.message.contains("drop_box_without_glue"),
        "unexpected error: {}",
        error.message
    );
}

#[test]
fn projected_return_place_drop_without_required_glue_is_codegen_error() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(24));
    let struct_id = DefId::new(CrateId(0), LocalDefId(89));
    let owner_ty = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let i64_ty = type_id(&type_context, Type::I64);
    let owner_type_id = type_id(&type_context, owner_ty.clone());
    let function = MirFunction {
        id: function_id.clone(),
        name: "drop_return_field_without_glue".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Drop {
                    place: field_place(0, 0),
                    target: crate::mir::BasicBlockId(1),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![LocalDecl {
            ty: owner_type_id,
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: owner_type_id,
        ownership: Default::default(),
    };
    let mut backend_contract = local_body_contract(
        function_id.clone(),
        "drop_return_field_without_glue",
        owner_type_id,
    );
    backend_contract.nominal_layouts.insert(
        struct_id,
        MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), i64_ty)],
            generic_params: Vec::new(),
        },
    );
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    let error = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        error
            .message
            .contains("Missing drop glue contract for MIR drop")
            && error.message.contains("drop_return_field_without_glue"),
        "unexpected error: {}",
        error.message
    );
}

#[test]
fn mir_codegen_does_not_call_drop_impl_for_return_place() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(21));
    let struct_id = DefId::new(CrateId(0), LocalDefId(83));
    let drop_method_id = DefId::new(CrateId(0), LocalDefId(86));
    let box_ty = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let return_ty = type_id(&type_context, box_ty.clone());
    let unit_type = type_id(&type_context, Type::Unit);
    let field_type = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "return_box".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Drop {
                    place: root_place(0),
                    target: crate::mir::BasicBlockId(1),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![LocalDecl {
            ty: return_ty,
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: return_ty,
        ownership: Default::default(),
    };
    let mut backend_contract = drop_glue_contract(
        return_ty,
        MirCallableKey::Instance(InstanceId(121)),
        drop_method_id,
        "Box_drop",
        unit_type,
    );
    insert_local_body_contract(
        &mut backend_contract,
        function_id.clone(),
        "return_box",
        &[],
        return_ty,
    );
    insert_struct_layout_contract(
        &mut backend_contract,
        struct_id,
        vec![("value".to_string(), field_type)],
    );
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(!ir.contains("call void @Box_drop"));
}
