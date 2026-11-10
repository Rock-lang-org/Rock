use super::*;

#[test]
fn mir_codegen_lowers_bounds_check_assertion() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    compile_test_function_with_contract(
        &mut codegen,
        &type_context,
        "check_bounds",
        Type::I64,
        vec![],
        vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Value(4)), "arr"),
        ],
        vec![
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![
                        Operand::Copy(root_place(1)),
                        Operand::Constant(crate::mir::Constant::Int(2)),
                    ],
                },
                None,
            ),
            StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(0))),
                None,
            ),
        ],
        bounds_check_contract(),
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("arr_oob"));
    assert!(ir.contains("oob_exit"));
}

#[test]
fn mir_codegen_lowers_slice_bounds_check_assertion() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let slice_ty = Type::Slice(Box::new(Type::I64));

    compile_test_function_with_contract(
        &mut codegen,
        &type_context,
        "check_slice_bounds",
        Type::I64,
        vec![slice_ty.clone()],
        vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, slice_ty, "slice"),
        ],
        vec![
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![
                        Operand::Copy(root_place(1)),
                        Operand::Constant(crate::mir::Constant::Int(0)),
                    ],
                },
                None,
            ),
            StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(0))),
                None,
            ),
        ],
        bounds_check_contract(),
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("slice_len"));
    assert!(ir.contains("arr_oob"));
    assert!(ir.contains("oob_exit"));
}

#[test]
fn mir_codegen_lowers_slice_reference_bounds_check_assertion() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let slice_ref_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Slice(Box::new(Type::I64))),
    };

    compile_test_function_with_contract(
        &mut codegen,
        &type_context,
        "check_slice_ref_bounds",
        Type::I64,
        vec![slice_ref_ty.clone()],
        vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, slice_ref_ty, "slice_ref"),
        ],
        vec![
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![
                        Operand::Copy(root_place(1)),
                        Operand::Constant(crate::mir::Constant::Int(0)),
                    ],
                },
                None,
            ),
            StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(0))),
                None,
            ),
        ],
        bounds_check_contract(),
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("slice_ref_len"));
    assert!(ir.contains("arr_oob"));
    assert!(ir.contains("oob_exit"));
}

#[test]
fn mir_codegen_lowers_string_reference_bounds_check_assertion() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let str_ref_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Str),
    };

    compile_test_function_with_contract(
        &mut codegen,
        &type_context,
        "check_str_ref_bounds",
        Type::I64,
        vec![str_ref_ty.clone()],
        vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, str_ref_ty, "str_ref"),
        ],
        vec![
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![
                        Operand::Copy(root_place(1)),
                        Operand::Constant(crate::mir::Constant::Int(0)),
                    ],
                },
                None,
            ),
            StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(0))),
                None,
            ),
        ],
        bounds_check_contract(),
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("slice_ref_len"));
    assert!(ir.contains("arr_oob"));
    assert!(ir.contains("oob_exit"));
}
