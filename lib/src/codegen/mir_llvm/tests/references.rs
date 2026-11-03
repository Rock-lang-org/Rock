use super::*;

#[test]
fn mir_reference_promotion_ignores_local_display_name_presence() {
    let named_ir = reference_promotion_ir(Some("scalar"), crate::mir::LocalSource::UserBinding);
    let unnamed_ir = reference_promotion_ir(None, crate::mir::LocalSource::UserBinding);

    assert!(!named_ir.contains("mir_ref_tmp_alloc"));
    assert!(!unnamed_ir.contains("mir_ref_tmp_alloc"));
}

#[test]
fn mir_ref_to_temporary_does_not_emit_heap_promotion() {
    let ir = reference_promotion_ir(Some("scalar"), crate::mir::LocalSource::Temporary);

    assert!(
        !ir.contains("mir_ref_tmp_alloc"),
        "reference to MIR temporary should use the place address directly:\n{ir}"
    );
}

fn reference_promotion_ir(source_name: Option<&str>, source: crate::mir::LocalSource) -> String {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "reference_promotion");
    let type_context = RefCell::new(TypeContext::new());
    let scalar_ref_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };

    compile_test_function(
        &mut codegen,
        &type_context,
        "reference_promotion",
        Type::Unit,
        vec![],
        vec![
            local_decl(&type_context, Type::Unit, "return_place"),
            local_decl_with_name_and_source(&type_context, Type::I64, source_name, source),
            local_decl(&type_context, scalar_ref_ty.clone(), "scalar_ref"),
        ],
        vec![
            StatementData::assign(
                root_place(1),
                Rvalue::Use(Operand::Constant(Constant::Int(7))),
                None,
            ),
            StatementData::assign(
                root_place(2),
                Rvalue::Ref(Mutability::Not, root_place(1)),
                None,
            ),
        ],
    );

    codegen.get_ir()
}

fn local_decl_with_name_and_source(
    type_context: &RefCell<TypeContext>,
    ty: Type,
    name: Option<&str>,
    source: crate::mir::LocalSource,
) -> LocalDecl {
    LocalDecl {
        ty: type_id(type_context, ty),
        mutability: Mutability::Mut,
        name: name.map(str::to_string),
        span: None,
        source,
    }
}

#[test]
fn mir_codegen_lowers_references_and_checks() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = RefCell::new(TypeContext::new());
    let array_ty = Type::Array(Box::new(Type::I64), 3);
    let scalar_ref_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };
    let array_ref_ty = Type::Reference {
        mutable: false,
        inner: Box::new(array_ty.clone()),
    };
    let slice_ref_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Slice(Box::new(Type::I64))),
    };
    let str_ref_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Str),
    };
    let raw_ptr_ty = Type::Pointer(Box::new(Type::I64));
    let raw_slice_ptr_ty = Type::Pointer(Box::new(Type::Slice(Box::new(Type::I64))));

    compile_test_function_with_contract(
        &mut codegen,
        &type_context,
        "refs_and_checks",
        Type::I64,
        vec![
            slice_ref_ty.clone(),
            raw_slice_ptr_ty.clone(),
            str_ref_ty.clone(),
        ],
        vec![
            local_decl(&type_context, Type::I64, "return_place"),
            local_decl(&type_context, slice_ref_ty.clone(), "slice_param"),
            local_decl(&type_context, raw_slice_ptr_ty.clone(), "raw_slice_param"),
            local_decl(&type_context, str_ref_ty.clone(), "str_param"),
            local_decl(&type_context, Type::I64, "scalar"),
            local_decl(&type_context, scalar_ref_ty.clone(), "scalar_ref"),
            local_decl(&type_context, raw_ptr_ty.clone(), "raw_ptr"),
            local_decl(&type_context, Type::I64, "ptr_bits"),
            local_decl(&type_context, array_ty.clone(), "array_value"),
            local_decl(&type_context, slice_ref_ty.clone(), "slice_ref"),
            local_decl(&type_context, array_ref_ty.clone(), "array_ref"),
            local_decl(&type_context, slice_ref_ty.clone(), "cast_slice_ref"),
            local_decl(&type_context, Type::F64, "float_value"),
            local_decl(&type_context, Type::I64, "int_value"),
            local_decl(&type_context, raw_ptr_ty.clone(), "int_ptr"),
            local_decl(&type_context, raw_slice_ptr_ty.clone(), "raw_slice_cast"),
        ],
        vec![
            StatementData::assign(
                root_place(4),
                Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(9))),
                None,
            ),
            StatementData::assign(
                root_place(5),
                Rvalue::Ref(Mutability::Not, root_place(4)),
                None,
            ),
            StatementData::assign(
                root_place(6),
                Rvalue::Cast(
                    Operand::Copy(root_place(5)),
                    type_id(&type_context, raw_ptr_ty.clone()),
                ),
                None,
            ),
            StatementData::assign(
                root_place(7),
                Rvalue::Cast(
                    Operand::Copy(root_place(6)),
                    type_id(&type_context, Type::I64),
                ),
                None,
            ),
            StatementData::assign(
                root_place(8),
                Rvalue::Aggregate(
                    AggregateKind::Array,
                    vec![
                        Operand::Constant(crate::mir::Constant::Int(1)),
                        Operand::Constant(crate::mir::Constant::Int(2)),
                        Operand::Constant(crate::mir::Constant::Int(3)),
                    ],
                ),
                None,
            ),
            StatementData::assign(
                root_place(9),
                Rvalue::Ref(Mutability::Not, root_place(8)),
                None,
            ),
            StatementData::assign(
                root_place(10),
                Rvalue::Ref(Mutability::Not, root_place(8)),
                None,
            ),
            StatementData::assign(
                root_place(11),
                Rvalue::Cast(
                    Operand::Copy(root_place(10)),
                    type_id(&type_context, slice_ref_ty.clone()),
                ),
                None,
            ),
            StatementData::assign(
                root_place(12),
                Rvalue::Cast(
                    Operand::Copy(root_place(4)),
                    type_id(&type_context, Type::F64),
                ),
                None,
            ),
            StatementData::assign(
                root_place(13),
                Rvalue::Cast(
                    Operand::Copy(root_place(12)),
                    type_id(&type_context, Type::I64),
                ),
                None,
            ),
            StatementData::assign(
                root_place(14),
                Rvalue::Cast(
                    Operand::Copy(root_place(13)),
                    type_id(&type_context, raw_ptr_ty.clone()),
                ),
                None,
            ),
            StatementData::assign(
                root_place(15),
                Rvalue::Cast(
                    Operand::Copy(root_place(11)),
                    type_id(&type_context, raw_slice_ptr_ty.clone()),
                ),
                None,
            ),
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![
                        Operand::Copy(root_place(8)),
                        Operand::Constant(crate::mir::Constant::Int(2)),
                    ],
                },
                None,
            ),
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![
                        Operand::Copy(root_place(9)),
                        Operand::Constant(crate::mir::Constant::Int(2)),
                    ],
                },
                None,
            ),
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![
                        Operand::Copy(root_place(11)),
                        Operand::Constant(crate::mir::Constant::Int(2)),
                    ],
                },
                None,
            ),
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
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![
                        Operand::Copy(root_place(2)),
                        Operand::Constant(crate::mir::Constant::Int(0)),
                    ],
                },
                None,
            ),
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![
                        Operand::Copy(root_place(3)),
                        Operand::Constant(crate::mir::Constant::Int(0)),
                    ],
                },
                None,
            ),
            StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Copy(root_place(7))),
                None,
            ),
        ],
        bounds_check_contract(),
    );

    let ir = codegen.get_ir();
    assert!(ir.contains("store ptr %scalar, ptr %scalar_ref"));
    assert!(ir.contains("ptrtoint ptr"));
    assert!(ir.contains("sitofp i64"));
    assert!(ir.contains("fptosi double"));
    assert!(ir.contains("inttoptr i64"));
    assert!(ir.contains("array_ref_slice_ptr"));
    assert!(ir.contains("array_ref_data_ptr"));
    assert!(ir.contains("raw_slice_cast"));
    assert!(ir.contains("slice_ref_ptr"));
    assert!(ir.contains("slice_ref_len"));
    assert!(ir.contains("i64 3"));
    assert!(ir.contains("arr_oob"));
    assert!(ir.contains("call void @exit(i32 1)"));
}
