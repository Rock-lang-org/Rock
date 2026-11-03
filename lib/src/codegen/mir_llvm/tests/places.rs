use super::*;

fn projection_type_with_output(
    type_context: &RefCell<TypeContext>,
    base_ty: Type,
    trait_id: DefId,
    assoc_type_id: AssocTypeId,
    trait_args: Vec<Type>,
    output_ty: Type,
) -> (Type, MirProjectionKey, crate::ids::TypeId) {
    let base = type_id(type_context, base_ty.clone());
    let trait_arg_ids = trait_args
        .iter()
        .cloned()
        .map(|arg| type_id(type_context, arg))
        .collect();
    let output = type_id(type_context, output_ty);
    let key = MirProjectionKey {
        base,
        trait_id,
        assoc_type_id,
        trait_args: trait_arg_ids,
    };
    let projection = Type::Projection {
        ty: Box::new(base_ty),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args,
    };

    (projection, key, output)
}

#[test]
fn mir_codegen_reports_non_integer_index_projection() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let result = catch_unwind(AssertUnwindSafe(|| {
        compile_test_function_result(
            &mut codegen,
            &type_context,
            "bad_index_projection",
            Type::I64,
            vec![],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(
                    &type_context,
                    Type::Array(Box::new(Type::I64), 2),
                    "array_value",
                ),
                local_decl(&type_context, Type::F64, "index"),
            ],
            vec![
                StatementData::assign(
                    root_place(1),
                    Rvalue::Aggregate(
                        AggregateKind::Array,
                        vec![
                            Operand::Constant(Constant::Int(1)),
                            Operand::Constant(Constant::Int(2)),
                        ],
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(2),
                    Rvalue::Use(Operand::Constant(Constant::Float(0.0))),
                    None,
                ),
                StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Copy(index_place(1, 2))),
                    None,
                ),
            ],
        )
    }));

    assert!(result.is_ok());
    let result = result.unwrap();
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("index"));
}

#[test]
fn mir_codegen_lowers_pointer_deref_place() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let ptr_ty = Type::Pointer(Box::new(Type::U8));

    compile_test_function(
        &mut codegen,
        &type_context,
        "load_deref",
        Type::U8,
        vec![ptr_ty.clone()],
        vec![
            local_decl(&type_context, Type::U8, "return_place"),
            local_decl(&type_context, ptr_ty, "ptr"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::Use(Operand::Copy(Place {
                local: Local(1),
                projection: vec![Projection::Deref],
            })),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("load i8, ptr"));
    assert!(ir.contains("ret i8"));
}

#[test]
fn mir_codegen_lowers_raw_pointer_index_place_without_bounds_check() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let ptr_ty = Type::Pointer(Box::new(Type::I64));

    compile_test_function(
        &mut codegen,
        &type_context,
        "load_ptr_index",
        Type::I64,
        vec![ptr_ty.clone(), Type::I64],
        vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, ptr_ty, "ptr"),
            local_decl(&type_context, Type::I64, "index"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::Use(Operand::Copy(Place {
                local: Local(1),
                projection: vec![Projection::Index(Local(2))],
            })),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("getelementptr i64, ptr"));
    assert!(!ir.contains("arr_oob"));
}

#[test]
fn mir_codegen_normalizes_projected_array_place_before_index() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let trait_id = DefId::new(CrateId(0), LocalDefId(940));
    let base_struct_id = DefId::new(CrateId(0), LocalDefId(941));
    let assoc_type_id = AssocTypeId(0);
    let (projected_array_ty, projection_key, output_id) = projection_type_with_output(
        &type_context,
        Type::Struct {
            id: base_struct_id,
            args: Vec::new(),
        },
        trait_id,
        assoc_type_id,
        vec![Type::Bool],
        Type::Array(Box::new(Type::I64), 2),
    );
    let function_id = MirFunctionId::Instance(InstanceId(0));
    let ret_type = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "projected_array_index".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
                    root_place(2),
                    Rvalue::Use(Operand::Constant(Constant::Int(1))),
                    None,
                ),
                StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Copy(index_place(1, 2))),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, projected_array_ty, "projected_array"),
            local_decl(&type_context, Type::I64, "index"),
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type,
        ownership: Default::default(),
    };
    let mut backend_contract =
        local_body_contract(function_id.clone(), "projected_array_index", ret_type);
    backend_contract.projection_traits.insert(trait_id);
    backend_contract
        .projection_outputs
        .insert(projection_key, output_id);
    insert_struct_layout_contract(&mut backend_contract, base_struct_id, Vec::new());
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id, function)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("getelementptr [2 x i64], ptr %projected_array"));
}

#[test]
fn mir_contract_validation_normalizes_projected_field_type_before_index() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(930));
    let struct_id = DefId::new(CrateId(0), LocalDefId(942));
    let trait_id = DefId::new(CrateId(0), LocalDefId(943));
    let projection_base_id = DefId::new(CrateId(0), LocalDefId(944));
    let assoc_type_id = AssocTypeId(0);
    let unit = type_id(&type_context, Type::Unit);
    let (projected_array_ty, projection_key, output_id) = projection_type_with_output(
        &type_context,
        Type::Struct {
            id: projection_base_id,
            args: Vec::new(),
        },
        trait_id,
        assoc_type_id,
        Vec::new(),
        Type::Array(Box::new(Type::I64), 2),
    );
    let projected_array_id = type_id(&type_context, projected_array_ty);
    let owner_ty = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let owner_type_id = type_id(&type_context, owner_ty.clone());
    let index_ty = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "drop_projected_field_index".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Drop {
                    place: Place {
                        local: Local(1),
                        projection: vec![
                            Projection::Field {
                                index: 0,
                                identity: None,
                            },
                            Projection::Index(Local(2)),
                        ],
                    },
                    target: crate::mir::BasicBlockId(1),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: owner_type_id,
                mutability: Mutability::Mut,
                name: Some("owner".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: index_ty,
                mutability: Mutability::Mut,
                name: Some("index".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let callable_key = MirCallableKey::Instance(InstanceId(930));
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_traits.insert(trait_id);
    backend_contract
        .projection_outputs
        .insert(projection_key, output_id);
    insert_struct_layout_contract(&mut backend_contract, projection_base_id, Vec::new());
    backend_contract.nominal_layouts.insert(
        struct_id,
        MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("items".to_string(), projected_array_id)],
            generic_params: Vec::new(),
        },
    );
    backend_contract.callables.insert(
        callable_key.clone(),
        MirCallableDecl {
            key: callable_key.clone(),
            source_def_id: None,
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "drop_projected_field_index".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], unit, MirPassMode::Direct),
        },
    );
    backend_contract
        .function_bodies
        .insert(function_id.clone(), callable_key);
    let mir = MirProgram {
        functions: BTreeMap::from([(function_id, function)]),
        type_context: type_context.borrow().clone(),
        backend_contract,
    };

    compile_valid_mir_program(&mut codegen, &mir).unwrap();
}
