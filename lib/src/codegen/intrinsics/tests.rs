use inkwell::context::Context;
use inkwell::AddressSpace;

use crate::codegen::CodeGen;
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, TypeId};
use crate::mir::{MirBackendContract, MirIntrinsicId, MirProgram, MirProjectionKey};
use crate::type_context::TypeContext;
use crate::types::{AssociatedTypeKey, Type};

fn load_projection_contract(
    codegen: &mut CodeGen<'_>,
    type_context: TypeContext,
    projection_key: MirProjectionKey,
    output: TypeId,
) {
    let mut backend_contract = MirBackendContract::default();
    backend_contract
        .projection_outputs
        .insert(projection_key, output);
    let mir = MirProgram {
        functions: std::collections::BTreeMap::new(),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();
}

#[test]
fn compile_intrinsic_values_lowers_integer_add() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_type = context.i64_type().fn_type(
        &[context.i64_type().into(), context.i64_type().into()],
        false,
    );
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);

    let lhs = function.get_nth_param(0).unwrap();
    let rhs = function.get_nth_param(1).unwrap();
    let value = codegen
        .compile_intrinsic_values(MirIntrinsicId::I64Add, &[lhs, rhs], &Type::I64)
        .unwrap()
        .unwrap();
    codegen.builder.build_return(Some(&value)).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("add i64"));
    assert!(ir.contains("ret i64"));
}

#[test]
fn compile_float_inequality_includes_unordered_operands() {
    let context = Context::create();
    for (intrinsic, float_type) in [
        (MirIntrinsicId::F32Ne, context.f32_type()),
        (MirIntrinsicId::F64Ne, context.f64_type()),
    ] {
        let mut codegen = CodeGen::new(&context, "test");
        let function_type = context
            .bool_type()
            .fn_type(&[float_type.into(), float_type.into()], false);
        let function = codegen.module.add_function("test", function_type, None);
        let entry = context.append_basic_block(function, "entry");
        codegen.builder.position_at_end(entry);
        let lhs = function.get_nth_param(0).unwrap();
        let rhs = function.get_nth_param(1).unwrap();
        let value = codegen
            .compile_intrinsic_values(intrinsic, &[lhs, rhs], &Type::Bool)
            .unwrap()
            .unwrap();
        codegen.builder.build_return(Some(&value)).unwrap();

        codegen.module.verify().unwrap();
        assert!(codegen.get_ir().contains("fcmp une"));
    }
}

#[test]
fn compile_intrinsic_values_lowers_atomic_u64_fetch_add() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let pointer_type = context.ptr_type(AddressSpace::default());
    let function_type = context
        .i64_type()
        .fn_type(&[pointer_type.into(), context.i64_type().into()], false);
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);

    let pointer = function.get_nth_param(0).unwrap();
    let value = function.get_nth_param(1).unwrap();
    let previous = codegen
        .compile_intrinsic_values(
            MirIntrinsicId::AtomicU64FetchAdd,
            &[pointer, value],
            &Type::U64,
        )
        .unwrap()
        .unwrap();
    codegen.builder.build_return(Some(&previous)).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("atomicrmw add ptr"));
    assert!(ir.contains("seq_cst"));
}

#[test]
fn compile_intrinsic_values_lowers_aligned_atomic_u64_store() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let pointer_type = context.ptr_type(AddressSpace::default());
    let function_type = context
        .void_type()
        .fn_type(&[pointer_type.into(), context.i64_type().into()], false);
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);

    let pointer = function.get_nth_param(0).unwrap();
    let value = function.get_nth_param(1).unwrap();
    codegen
        .compile_intrinsic_values_with_types(
            MirIntrinsicId::AtomicU64Store,
            &[pointer, value],
            &[Type::Pointer(Box::new(Type::U64)), Type::U64],
            &Type::Unit,
        )
        .unwrap();
    codegen.builder.build_return(None).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("store atomic i64"));
    assert!(ir.contains("seq_cst, align 8"));
}

#[test]
fn compile_intrinsic_values_lowers_raw_wide_pointer() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    let function_type = context
        .void_type()
        .fn_type(&[ptr_ty.into(), context.i64_type().into()], false);
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);

    let ptr = function.get_nth_param(0).unwrap();
    let len = function.get_nth_param(1).unwrap();
    let _value = codegen
        .compile_intrinsic_values(
            MirIntrinsicId::PtrFromParts,
            &[ptr, len],
            &Type::Pointer(Box::new(Type::Slice(Box::new(Type::U8)))),
        )
        .unwrap()
        .unwrap();
    codegen.builder.build_return(None).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("wide_data"));
    assert!(ir.contains("wide_length"));
}

#[test]
fn compile_intrinsic_values_with_type_ids_array_len_handles_slice_ref() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = TypeContext::new();
    let slice_id = type_context.intern_type(&Type::Slice(Box::new(Type::I64)));
    let slice_ref_id = type_context.intern_type(&Type::Reference {
        inner: Box::new(Type::Slice(Box::new(Type::I64))),
        mutable: false,
    });
    let i64_id = type_context.intern_type(&Type::I64);
    codegen.set_type_context(type_context);

    let slice_ty = codegen.slice_layout_type();
    let function_type = context.i64_type().fn_type(&[slice_ty.into()], false);
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);
    let slice = function.get_nth_param(0).unwrap();

    let value = codegen
        .compile_intrinsic_values_with_type_ids(
            MirIntrinsicId::ArrayLen,
            &[slice],
            &[slice_ref_id],
            i64_id,
        )
        .unwrap()
        .unwrap();
    codegen.builder.build_return(Some(&value)).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("extractvalue"));
    assert!(ir.contains("arr_len"));
    assert!(matches!(
        codegen.type_view().ty(slice_id),
        crate::type_context::Ty::Slice(_)
    ));
}

#[test]
fn compile_intrinsic_values_with_type_ids_array_len_handles_array_ref() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = TypeContext::new();
    let array_ref_id = type_context.intern_type(&Type::Reference {
        inner: Box::new(Type::Array(Box::new(Type::I64), 4)),
        mutable: false,
    });
    let i64_id = type_context.intern_type(&Type::I64);
    codegen.set_type_context(type_context);

    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    let function_type = context.i64_type().fn_type(&[ptr_ty.into()], false);
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);
    let array_ref = function.get_nth_param(0).unwrap();

    let value = codegen
        .compile_intrinsic_values_with_type_ids(
            MirIntrinsicId::ArrayLen,
            &[array_ref],
            &[array_ref_id],
            i64_id,
        )
        .unwrap()
        .unwrap();
    codegen.builder.build_return(Some(&value)).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("ret i64 4"));
}

#[test]
fn compile_intrinsic_values_with_type_ids_array_len_handles_projected_array_ref() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = TypeContext::new();
    let trait_id = DefId::new(CrateId(0), LocalDefId(910));
    let assoc_type_id = AssocTypeId(0);
    let base_ty = Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(911)),
        args: Vec::new(),
    };
    let trait_arg_ty = Type::Bool;
    let projected_ref_ty = Type::Reference {
        inner: Box::new(Type::Array(Box::new(Type::I64), 6)),
        mutable: false,
    };
    let base_id = type_context.intern_type(&base_ty);
    let trait_arg_id = type_context.intern_type(&trait_arg_ty);
    let projected_ref_id = type_context.intern_type(&projected_ref_ty);
    let projection_id = type_context.intern_type(&Type::Projection {
        ty: Box::new(base_ty),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: vec![trait_arg_ty],
    });
    let i64_id = type_context.intern_type(&Type::I64);
    load_projection_contract(
        &mut codegen,
        type_context,
        MirProjectionKey {
            base: base_id,
            trait_id,
            assoc_type_id,
            trait_args: vec![trait_arg_id],
        },
        projected_ref_id,
    );

    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    let function_type = context.i64_type().fn_type(&[ptr_ty.into()], false);
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);
    let projected_ref = function.get_nth_param(0).unwrap();

    let value = codegen
        .compile_intrinsic_values_with_type_ids(
            MirIntrinsicId::ArrayLen,
            &[projected_ref],
            &[projection_id],
            i64_id,
        )
        .unwrap()
        .unwrap();
    codegen.builder.build_return(Some(&value)).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("ret i64 6"));
}

#[test]
fn compile_intrinsic_values_with_type_ids_ptr_offset_uses_projected_pointer_stride() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = TypeContext::new();
    let trait_id = DefId::new(CrateId(0), LocalDefId(912));
    let assoc_type_id = AssocTypeId(0);
    let base_ty = Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(913)),
        args: Vec::new(),
    };
    let projected_ptr_ty = Type::Pointer(Box::new(Type::I64));
    let base_id = type_context.intern_type(&base_ty);
    let projected_ptr_id = type_context.intern_type(&projected_ptr_ty);
    let projection_id = type_context.intern_type(&Type::Projection {
        ty: Box::new(base_ty),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: Vec::new(),
    });
    let i64_id = type_context.intern_type(&Type::I64);
    load_projection_contract(
        &mut codegen,
        type_context,
        MirProjectionKey {
            base: base_id,
            trait_id,
            assoc_type_id,
            trait_args: Vec::new(),
        },
        projected_ptr_id,
    );

    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    let function_type = ptr_ty.fn_type(&[ptr_ty.into(), context.i64_type().into()], false);
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);
    let ptr = function.get_nth_param(0).unwrap();
    let index = function.get_nth_param(1).unwrap();

    let value = codegen
        .compile_intrinsic_values_with_type_ids(
            MirIntrinsicId::PtrOffset,
            &[ptr, index],
            &[projection_id, i64_id],
            projection_id,
        )
        .unwrap()
        .unwrap();
    codegen.builder.build_return(Some(&value)).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("getelementptr i64"), "IR was:\n{ir}");
    assert!(!ir.contains("getelementptr i8"), "IR was:\n{ir}");
}

#[test]
fn compile_intrinsic_values_with_type_ids_array_len_rejects_nominal_array_display_name() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = TypeContext::new();
    let array_def = DefId::new(CrateId(0), LocalDefId(901));
    let nominal_array_id = type_context.intern_type(&Type::Struct {
        id: array_def,
        args: Vec::new(),
    });
    let i64_id = type_context.intern_type(&Type::I64);
    codegen.set_type_context(type_context);
    codegen
        .struct_names_by_id
        .insert(array_def, "Array".to_string());

    let slice_ty = codegen.slice_layout_type();
    let function_type = context.i64_type().fn_type(&[slice_ty.into()], false);
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);
    let nominal_array_value = function.get_nth_param(0).unwrap();

    let error = codegen
        .compile_intrinsic_values_with_type_ids(
            MirIntrinsicId::ArrayLen,
            &[nominal_array_value],
            &[nominal_array_id],
            i64_id,
        )
        .expect_err("nominal struct display name must not drive ArrayLen lowering");

    assert!(error.message.contains("ArrayLen expected slice/array"));
}

#[test]
fn compile_intrinsic_values_with_type_ids_arr_ptr_rejects_nominal_array_display_name() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = TypeContext::new();
    let array_def = DefId::new(CrateId(0), LocalDefId(902));
    let nominal_array_id = type_context.intern_type(&Type::Struct {
        id: array_def,
        args: Vec::new(),
    });
    let ret_id = type_context.intern_type(&Type::Pointer(Box::new(Type::U8)));
    codegen.set_type_context(type_context);
    codegen
        .struct_names_by_id
        .insert(array_def, "Array".to_string());

    let slice_ty = codegen.slice_layout_type();
    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    let function_type = ptr_ty.fn_type(&[slice_ty.into()], false);
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);
    let nominal_array_value = function.get_nth_param(0).unwrap();

    let error = codegen
        .compile_intrinsic_values_with_type_ids(
            MirIntrinsicId::ArrPtr,
            &[nominal_array_value],
            &[nominal_array_id],
            ret_id,
        )
        .expect_err("nominal struct display name must not drive ArrPtr lowering");

    assert!(error.message.contains("ArrPtr expected slice/array"));
}

#[test]
fn compile_intrinsic_values_with_type_ids_dispatches_type_only_intrinsic_by_typed_id() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = TypeContext::new();
    let i64_id = type_context.intern_type(&Type::I64);
    codegen.set_type_context(type_context);

    let function_type = context.i64_type().fn_type(&[], false);
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);

    let value = codegen
        .compile_intrinsic_values_with_type_ids(MirIntrinsicId::SizeOf, &[], &[i64_id], i64_id)
        .unwrap()
        .unwrap();
    codegen.builder.build_return(Some(&value)).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("ret i64"));
}

#[test]
fn compile_intrinsic_values_dispatches_by_enum_variant_not_display_name() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_type = context.i64_type().fn_type(
        &[context.i64_type().into(), context.i64_type().into()],
        false,
    );
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);

    let lhs = function.get_nth_param(0).unwrap();
    let rhs = function.get_nth_param(1).unwrap();
    let error = codegen
        .compile_intrinsic_values(MirIntrinsicId::DisplayNameI64Add, &[lhs, rhs], &Type::I64)
        .expect_err("display-only test variant must not dispatch as I64Add");

    assert!(error.message.contains("Unknown intrinsic"));
}

#[test]
fn compile_intrinsic_values_zero_extends_u8_less_than_integer_constant() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_type = context
        .bool_type()
        .fn_type(&[context.i8_type().into()], false);
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);

    let lhs = function.get_nth_param(0).unwrap();
    let rhs = context.i64_type().const_int(255, false).into();
    let value = codegen
        .compile_intrinsic_values(MirIntrinsicId::U8Lt, &[lhs, rhs], &Type::Bool)
        .unwrap()
        .unwrap();
    codegen.builder.build_return(Some(&value)).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("zext"));
    assert!(ir.contains("icmp ult"));
    assert!(!ir.contains("sext"));
    assert!(!ir.contains("icmp slt"));
}

#[test]
fn compile_intrinsic_values_zero_extends_u8_right_shift_integer_constant() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_type = context
        .i64_type()
        .fn_type(&[context.i8_type().into()], false);
    let function = codegen.module.add_function("test", function_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);

    let lhs = function.get_nth_param(0).unwrap();
    let rhs = context.i64_type().const_int(1, false).into();
    let value = codegen
        .compile_intrinsic_values(MirIntrinsicId::U8Shr, &[lhs, rhs], &Type::U64)
        .unwrap()
        .unwrap();
    codegen.builder.build_return(Some(&value)).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("zext"));
    assert!(ir.contains("lshr"));
    assert!(!ir.contains("sext"));
    assert!(!ir.contains("ashr"));
}
