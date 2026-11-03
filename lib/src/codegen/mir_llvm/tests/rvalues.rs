use super::*;

#[test]
fn mir_codegen_lowers_binary_integer_add_rvalue() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    compile_test_function(
        &mut codegen,
        &type_context,
        "add_values",
        Type::I64,
        vec![Type::I64, Type::I64],
        vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, Type::I64, "lhs"),
            local_decl(&type_context, Type::I64, "rhs"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::BinaryOp(
                MirBinOp::Add,
                Operand::Copy(root_place(1)),
                Operand::Copy(root_place(2)),
            ),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("add i64"));
    assert!(ir.contains("ret i64"));
}

#[test]
fn mir_codegen_lowers_unsigned_less_than_with_unsigned_predicate() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    compile_test_function(
        &mut codegen,
        &type_context,
        "u64_less_than",
        Type::Bool,
        vec![Type::U64, Type::U64],
        vec![
            local_decl(&type_context, Type::Bool, "return_place"),
            local_decl(&type_context, Type::U64, "lhs"),
            local_decl(&type_context, Type::U64, "rhs"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::BinaryOp(
                MirBinOp::Lt,
                Operand::Copy(root_place(1)),
                Operand::Copy(root_place(2)),
            ),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("icmp ult"));
    assert!(!ir.contains("icmp slt"));
}

#[test]
fn mir_codegen_lowers_unsigned_right_shift_logically() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    compile_test_function(
        &mut codegen,
        &type_context,
        "u64_shift_right",
        Type::U64,
        vec![Type::U64, Type::U64],
        vec![
            local_decl(&type_context, Type::U64, "return_place"),
            local_decl(&type_context, Type::U64, "lhs"),
            local_decl(&type_context, Type::U64, "rhs"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::BinaryOp(
                MirBinOp::Shr,
                Operand::Copy(root_place(1)),
                Operand::Copy(root_place(2)),
            ),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("lshr"));
    assert!(!ir.contains("ashr"));
}

#[test]
fn mir_codegen_zero_extends_u8_less_than_integer_constant() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    compile_test_function(
        &mut codegen,
        &type_context,
        "u8_less_than_constant",
        Type::Bool,
        vec![Type::U8],
        vec![
            local_decl(&type_context, Type::Bool, "return_place"),
            local_decl(&type_context, Type::U8, "lhs"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::BinaryOp(
                MirBinOp::Lt,
                Operand::Copy(root_place(1)),
                Operand::Constant(crate::mir::Constant::Int(255)),
            ),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("icmp ult"));
    assert!(ir.contains("zext"));
    assert!(!ir.contains("sext"));
    assert!(!ir.contains("icmp slt"));
}

#[test]
fn mir_codegen_zero_extends_u8_right_shift_integer_constant() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    compile_test_function(
        &mut codegen,
        &type_context,
        "u8_shift_right_constant",
        Type::U8,
        vec![Type::U8],
        vec![
            local_decl(&type_context, Type::U8, "return_place"),
            local_decl(&type_context, Type::U8, "lhs"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::BinaryOp(
                MirBinOp::Shr,
                Operand::Copy(root_place(1)),
                Operand::Constant(crate::mir::Constant::Int(1)),
            ),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("lshr"));
    assert!(ir.contains("zext"));
    assert!(!ir.contains("sext"));
    assert!(!ir.contains("ashr"));
}

#[test]
fn mir_codegen_infers_unsigned_type_for_left_integer_constant_less_than() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    compile_test_function(
        &mut codegen,
        &type_context,
        "constant_less_than_u8",
        Type::Bool,
        vec![Type::U8],
        vec![
            local_decl(&type_context, Type::Bool, "return_place"),
            local_decl(&type_context, Type::U8, "rhs"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::BinaryOp(
                MirBinOp::Lt,
                Operand::Constant(crate::mir::Constant::Int(0)),
                Operand::Copy(root_place(1)),
            ),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("icmp ult"));
    assert!(ir.contains("zext"));
    assert!(!ir.contains("icmp slt"));
    assert!(!ir.contains("sext"));
}

#[test]
fn mir_codegen_infers_unsigned_type_for_left_integer_constant_right_shift() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    compile_test_function(
        &mut codegen,
        &type_context,
        "constant_shift_right_u8",
        Type::U64,
        vec![Type::U8],
        vec![
            local_decl(&type_context, Type::U64, "return_place"),
            local_decl(&type_context, Type::U8, "rhs"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::BinaryOp(
                MirBinOp::Shr,
                Operand::Constant(crate::mir::Constant::Int(128)),
                Operand::Copy(root_place(1)),
            ),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("lshr"));
    assert!(ir.contains("zext"));
    assert!(!ir.contains("ashr"));
    assert!(!ir.contains("sext"));
}

#[test]
fn mir_codegen_lowers_unary_integer_neg_rvalue() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    compile_test_function(
        &mut codegen,
        &type_context,
        "neg_value",
        Type::I64,
        vec![Type::I64],
        vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, Type::I64, "value"),
        ],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::UnaryOp(MirUnaryOp::Neg, Operand::Copy(root_place(1))),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("sub i64 0"));
    assert!(ir.contains("ret i64"));
}

#[test]
fn mir_codegen_lowers_string_constant_rvalue() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());

    compile_test_function(
        &mut codegen,
        &type_context,
        "string_constant",
        Type::Str,
        vec![],
        vec![local_decl(&type_context, Type::Str, "return_place")],
        vec![StatementData::assign(
            root_place(0),
            Rvalue::Use(Operand::Constant(Constant::String("abc".to_string()))),
            None,
        )],
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("@str"));
    assert!(ir.contains("ret { ptr, i64 }"));
}
