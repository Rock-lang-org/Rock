use super::*;

#[test]
fn mir_codegen_lowers_integer_trunc_cast_rvalue() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    compile_test_function(
        &mut codegen,
        &type_context,
        "trunc_value",
        Type::I32,
        vec![Type::I64],
        vec![
            local_decl(&type_context, Type::I32, "return_place"),
            local_decl(&type_context, Type::I64, "value"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::Cast(
                Operand::Copy(root_place(1)),
                type_id(&type_context, Type::I32),
            ),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("trunc i64"));
    assert!(ir.contains("ret i32"));
}

#[test]
fn mir_codegen_lowers_char_to_u8_cast_rvalue() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    compile_test_function(
        &mut codegen,
        &type_context,
        "char_to_u8",
        Type::U8,
        vec![Type::Char],
        vec![
            local_decl(&type_context, Type::U8, "return_place"),
            local_decl(&type_context, Type::Char, "ch"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::Cast(
                Operand::Copy(root_place(1)),
                type_id(&type_context, Type::U8),
            ),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("ret i8"));
}

#[test]
fn mir_codegen_rejects_unsupported_cast_rvalue() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let function_id = MirFunctionId::Instance(InstanceId(0));
    let ret_type = type_id(&type_context, Type::I64);
    let function = MirFunction {
        id: function_id.clone(),
        name: "bad_cast".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                root_place(0),
                Rvalue::Cast(Operand::Constant(crate::mir::Constant::Unit), ret_type),
                None,
            )],
            terminator: Some(Terminator::Return),
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
        backend_contract: local_body_contract(function_id, "bad_cast", ret_type),
    };

    let error = codegen.compile_program_from_mir(&mir).unwrap_err();
    assert!(error.message.contains("Unsupported cast from"));
}

#[test]
fn mir_codegen_rejects_invalid_fat_pointer_casts() {
    let slice_ref_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Slice(Box::new(Type::I64))),
    };
    let raw_slice_ptr_ty = Type::Pointer(Box::new(Type::Slice(Box::new(Type::I64))));
    let raw_str_ptr_ty = Type::Pointer(Box::new(Type::Str));
    let raw_i64_ptr_ty = Type::Pointer(Box::new(Type::I64));

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let fat_ref_to_thin_ptr = catch_unwind(AssertUnwindSafe(|| {
        compile_test_function_result(
            &mut codegen,
            &type_context,
            "fat_ref_to_thin_ptr",
            raw_i64_ptr_ty.clone(),
            vec![slice_ref_ty.clone()],
            vec![
                local_decl(&type_context, raw_i64_ptr_ty.clone(), "return_place"),
                local_decl(&type_context, slice_ref_ty.clone(), "slice_ref"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Cast(
                    Operand::Copy(root_place(1)),
                    type_id(&type_context, raw_i64_ptr_ty.clone()),
                ),
                None,
            )],
        )
    }));
    assert!(fat_ref_to_thin_ptr.is_ok());
    let fat_ref_to_thin_ptr = fat_ref_to_thin_ptr.unwrap().unwrap_err();
    assert!(fat_ref_to_thin_ptr
        .message
        .contains("Cannot cast fat MIR pointer"));

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let fat_raw_ptr_to_int = catch_unwind(AssertUnwindSafe(|| {
        compile_test_function_result(
            &mut codegen,
            &type_context,
            "fat_raw_ptr_to_int",
            Type::I64,
            vec![raw_slice_ptr_ty.clone()],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, raw_slice_ptr_ty.clone(), "raw_slice_ptr"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Cast(
                    Operand::Copy(root_place(1)),
                    type_id(&type_context, Type::I64),
                ),
                None,
            )],
        )
    }));
    assert!(fat_raw_ptr_to_int.is_ok());
    let fat_raw_ptr_to_int = fat_raw_ptr_to_int.unwrap().unwrap_err();
    assert!(fat_raw_ptr_to_int
        .message
        .contains("Cannot cast fat MIR pointer"));

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let int_to_fat_raw_ptr = catch_unwind(AssertUnwindSafe(|| {
        compile_test_function_result(
            &mut codegen,
            &type_context,
            "int_to_fat_raw_ptr",
            raw_str_ptr_ty.clone(),
            vec![Type::I64],
            vec![
                local_decl(&type_context, raw_str_ptr_ty.clone(), "return_place"),
                local_decl(&type_context, Type::I64, "bits"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Cast(
                    Operand::Copy(root_place(1)),
                    type_id(&type_context, raw_str_ptr_ty.clone()),
                ),
                None,
            )],
        )
    }));
    assert!(int_to_fat_raw_ptr.is_ok());
    let int_to_fat_raw_ptr = int_to_fat_raw_ptr.unwrap().unwrap_err();
    assert!(int_to_fat_raw_ptr
        .message
        .contains("Cannot cast integer to fat MIR pointer"));
}
