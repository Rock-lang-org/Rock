use inkwell::values::BasicValueEnum;
use inkwell::{AtomicOrdering, AtomicRMWBinOp, FloatPredicate, IntPredicate};

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::MirIntrinsicId;
use crate::types::Type;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn compile_intrinsic_values(
        &mut self,
        intrinsic: MirIntrinsicId,
        compiled_args: &[BasicValueEnum<'ctx>],
        ret_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        match intrinsic {
            MirIntrinsicId::I8Add
            | MirIntrinsicId::I16Add
            | MirIntrinsicId::I32Add
            | MirIntrinsicId::I64Add => {
                self.compile_int_intrinsic(compiled_args, "add", |this, lhs, rhs| {
                    this.builder.build_int_add(lhs, rhs, "add")
                })
            }
            MirIntrinsicId::U8Add
            | MirIntrinsicId::U16Add
            | MirIntrinsicId::U32Add
            | MirIntrinsicId::U64Add => {
                self.compile_uint_intrinsic(compiled_args, "add", |this, lhs, rhs| {
                    this.builder.build_int_add(lhs, rhs, "add")
                })
            }
            MirIntrinsicId::I8Sub
            | MirIntrinsicId::I16Sub
            | MirIntrinsicId::I32Sub
            | MirIntrinsicId::I64Sub => {
                self.compile_int_intrinsic(compiled_args, "sub", |this, lhs, rhs| {
                    this.builder.build_int_sub(lhs, rhs, "sub")
                })
            }
            MirIntrinsicId::U8Sub
            | MirIntrinsicId::U16Sub
            | MirIntrinsicId::U32Sub
            | MirIntrinsicId::U64Sub => {
                self.compile_uint_intrinsic(compiled_args, "sub", |this, lhs, rhs| {
                    this.builder.build_int_sub(lhs, rhs, "sub")
                })
            }
            MirIntrinsicId::I8Mul
            | MirIntrinsicId::I16Mul
            | MirIntrinsicId::I32Mul
            | MirIntrinsicId::I64Mul => {
                self.compile_int_intrinsic(compiled_args, "mul", |this, lhs, rhs| {
                    this.builder.build_int_mul(lhs, rhs, "mul")
                })
            }
            MirIntrinsicId::U8Mul
            | MirIntrinsicId::U16Mul
            | MirIntrinsicId::U32Mul
            | MirIntrinsicId::U64Mul => {
                self.compile_uint_intrinsic(compiled_args, "mul", |this, lhs, rhs| {
                    this.builder.build_int_mul(lhs, rhs, "mul")
                })
            }
            MirIntrinsicId::I8Div
            | MirIntrinsicId::I16Div
            | MirIntrinsicId::I32Div
            | MirIntrinsicId::I64Div => {
                self.compile_typed_integer(compiled_args, crate::mir::integer::IntegerOp::Div, true)
            }
            MirIntrinsicId::U8Div
            | MirIntrinsicId::U16Div
            | MirIntrinsicId::U32Div
            | MirIntrinsicId::U64Div => self.compile_typed_integer(
                compiled_args,
                crate::mir::integer::IntegerOp::Div,
                false,
            ),
            MirIntrinsicId::I8Mod
            | MirIntrinsicId::I16Mod
            | MirIntrinsicId::I32Mod
            | MirIntrinsicId::I64Mod => {
                self.compile_typed_integer(compiled_args, crate::mir::integer::IntegerOp::Rem, true)
            }
            MirIntrinsicId::U8Mod
            | MirIntrinsicId::U16Mod
            | MirIntrinsicId::U32Mod
            | MirIntrinsicId::U64Mod => self.compile_typed_integer(
                compiled_args,
                crate::mir::integer::IntegerOp::Rem,
                false,
            ),
            MirIntrinsicId::I8Neg
            | MirIntrinsicId::I16Neg
            | MirIntrinsicId::I32Neg
            | MirIntrinsicId::I64Neg => {
                let val = compiled_args[0].into_int_value();
                Ok(Some(
                    self.builder
                        .build_int_neg(val, "neg")
                        .map_err(|e| CodegenError::from(format!("Failed to build neg: {}", e)))?
                        .into(),
                ))
            }
            MirIntrinsicId::I8Not
            | MirIntrinsicId::I16Not
            | MirIntrinsicId::I32Not
            | MirIntrinsicId::I64Not
            | MirIntrinsicId::U8Not
            | MirIntrinsicId::U16Not
            | MirIntrinsicId::U32Not
            | MirIntrinsicId::U64Not
            | MirIntrinsicId::BoolNot => {
                let val = compiled_args[0].into_int_value();
                Ok(Some(
                    self.builder
                        .build_not(val, "not")
                        .map_err(|e| CodegenError::from(format!("Failed to build not: {}", e)))?
                        .into(),
                ))
            }
            MirIntrinsicId::I8And
            | MirIntrinsicId::I16And
            | MirIntrinsicId::I32And
            | MirIntrinsicId::I64And
            | MirIntrinsicId::BoolAnd => {
                self.compile_int_intrinsic(compiled_args, "and", |this, lhs, rhs| {
                    this.builder.build_and(lhs, rhs, "and")
                })
            }
            MirIntrinsicId::U8And
            | MirIntrinsicId::U16And
            | MirIntrinsicId::U32And
            | MirIntrinsicId::U64And => {
                self.compile_uint_intrinsic(compiled_args, "and", |this, lhs, rhs| {
                    this.builder.build_and(lhs, rhs, "and")
                })
            }
            MirIntrinsicId::I8Or
            | MirIntrinsicId::I16Or
            | MirIntrinsicId::I32Or
            | MirIntrinsicId::I64Or
            | MirIntrinsicId::BoolOr => {
                self.compile_int_intrinsic(compiled_args, "or", |this, lhs, rhs| {
                    this.builder.build_or(lhs, rhs, "or")
                })
            }
            MirIntrinsicId::U8Or
            | MirIntrinsicId::U16Or
            | MirIntrinsicId::U32Or
            | MirIntrinsicId::U64Or => {
                self.compile_uint_intrinsic(compiled_args, "or", |this, lhs, rhs| {
                    this.builder.build_or(lhs, rhs, "or")
                })
            }
            MirIntrinsicId::I8Xor
            | MirIntrinsicId::I16Xor
            | MirIntrinsicId::I32Xor
            | MirIntrinsicId::I64Xor
            | MirIntrinsicId::BoolXor => {
                self.compile_int_intrinsic(compiled_args, "xor", |this, lhs, rhs| {
                    this.builder.build_xor(lhs, rhs, "xor")
                })
            }
            MirIntrinsicId::U8Xor
            | MirIntrinsicId::U16Xor
            | MirIntrinsicId::U32Xor
            | MirIntrinsicId::U64Xor => {
                self.compile_uint_intrinsic(compiled_args, "xor", |this, lhs, rhs| {
                    this.builder.build_xor(lhs, rhs, "xor")
                })
            }
            MirIntrinsicId::I8Shl
            | MirIntrinsicId::I16Shl
            | MirIntrinsicId::I32Shl
            | MirIntrinsicId::I64Shl => {
                self.compile_typed_integer(compiled_args, crate::mir::integer::IntegerOp::Shl, true)
            }
            MirIntrinsicId::U8Shl
            | MirIntrinsicId::U16Shl
            | MirIntrinsicId::U32Shl
            | MirIntrinsicId::U64Shl => self.compile_typed_integer(
                compiled_args,
                crate::mir::integer::IntegerOp::Shl,
                false,
            ),
            MirIntrinsicId::I8Shr
            | MirIntrinsicId::I16Shr
            | MirIntrinsicId::I32Shr
            | MirIntrinsicId::I64Shr => {
                self.compile_typed_integer(compiled_args, crate::mir::integer::IntegerOp::Shr, true)
            }
            MirIntrinsicId::U8Shr
            | MirIntrinsicId::U16Shr
            | MirIntrinsicId::U32Shr
            | MirIntrinsicId::U64Shr => self.compile_typed_integer(
                compiled_args,
                crate::mir::integer::IntegerOp::Shr,
                false,
            ),
            MirIntrinsicId::I8Lt
            | MirIntrinsicId::I16Lt
            | MirIntrinsicId::I32Lt
            | MirIntrinsicId::I64Lt => {
                self.compile_int_compare_intrinsic(compiled_args, IntPredicate::SLT)
            }
            MirIntrinsicId::I8Le
            | MirIntrinsicId::I16Le
            | MirIntrinsicId::I32Le
            | MirIntrinsicId::I64Le => {
                self.compile_int_compare_intrinsic(compiled_args, IntPredicate::SLE)
            }
            MirIntrinsicId::I8Gt
            | MirIntrinsicId::I16Gt
            | MirIntrinsicId::I32Gt
            | MirIntrinsicId::I64Gt => {
                self.compile_int_compare_intrinsic(compiled_args, IntPredicate::SGT)
            }
            MirIntrinsicId::I8Ge
            | MirIntrinsicId::I16Ge
            | MirIntrinsicId::I32Ge
            | MirIntrinsicId::I64Ge => {
                self.compile_int_compare_intrinsic(compiled_args, IntPredicate::SGE)
            }
            MirIntrinsicId::U8Lt
            | MirIntrinsicId::U16Lt
            | MirIntrinsicId::U32Lt
            | MirIntrinsicId::U64Lt => {
                self.compile_uint_compare_intrinsic(compiled_args, IntPredicate::ULT)
            }
            MirIntrinsicId::U8Le
            | MirIntrinsicId::U16Le
            | MirIntrinsicId::U32Le
            | MirIntrinsicId::U64Le => {
                self.compile_uint_compare_intrinsic(compiled_args, IntPredicate::ULE)
            }
            MirIntrinsicId::U8Gt
            | MirIntrinsicId::U16Gt
            | MirIntrinsicId::U32Gt
            | MirIntrinsicId::U64Gt => {
                self.compile_uint_compare_intrinsic(compiled_args, IntPredicate::UGT)
            }
            MirIntrinsicId::U8Ge
            | MirIntrinsicId::U16Ge
            | MirIntrinsicId::U32Ge
            | MirIntrinsicId::U64Ge => {
                self.compile_uint_compare_intrinsic(compiled_args, IntPredicate::UGE)
            }
            MirIntrinsicId::I8Eq
            | MirIntrinsicId::I16Eq
            | MirIntrinsicId::I32Eq
            | MirIntrinsicId::I64Eq
            | MirIntrinsicId::BoolEq => {
                self.compile_int_compare_intrinsic(compiled_args, IntPredicate::EQ)
            }
            MirIntrinsicId::U8Eq
            | MirIntrinsicId::U16Eq
            | MirIntrinsicId::U32Eq
            | MirIntrinsicId::U64Eq => {
                self.compile_uint_compare_intrinsic(compiled_args, IntPredicate::EQ)
            }
            MirIntrinsicId::I8Ne
            | MirIntrinsicId::I16Ne
            | MirIntrinsicId::I32Ne
            | MirIntrinsicId::I64Ne
            | MirIntrinsicId::BoolNe => {
                self.compile_int_compare_intrinsic(compiled_args, IntPredicate::NE)
            }
            MirIntrinsicId::U8Ne
            | MirIntrinsicId::U16Ne
            | MirIntrinsicId::U32Ne
            | MirIntrinsicId::U64Ne => {
                self.compile_uint_compare_intrinsic(compiled_args, IntPredicate::NE)
            }
            MirIntrinsicId::F32Add | MirIntrinsicId::F64Add => {
                self.compile_float_intrinsic(compiled_args, "fadd", |this, lhs, rhs| {
                    this.builder.build_float_add(lhs, rhs, "fadd")
                })
            }
            MirIntrinsicId::F32Sub | MirIntrinsicId::F64Sub => {
                self.compile_float_intrinsic(compiled_args, "fsub", |this, lhs, rhs| {
                    this.builder.build_float_sub(lhs, rhs, "fsub")
                })
            }
            MirIntrinsicId::F32Mul | MirIntrinsicId::F64Mul => {
                self.compile_float_intrinsic(compiled_args, "fmul", |this, lhs, rhs| {
                    this.builder.build_float_mul(lhs, rhs, "fmul")
                })
            }
            MirIntrinsicId::F32Div | MirIntrinsicId::F64Div => {
                self.compile_float_intrinsic(compiled_args, "fdiv", |this, lhs, rhs| {
                    this.builder.build_float_div(lhs, rhs, "fdiv")
                })
            }
            MirIntrinsicId::F32Mod | MirIntrinsicId::F64Mod => {
                self.compile_float_intrinsic(compiled_args, "frem", |this, lhs, rhs| {
                    this.builder.build_float_rem(lhs, rhs, "frem")
                })
            }
            MirIntrinsicId::F32Neg | MirIntrinsicId::F64Neg => {
                let val = compiled_args[0].into_float_value();
                Ok(Some(
                    self.builder
                        .build_float_neg(val, "fneg")
                        .map_err(|e| CodegenError::from(format!("Failed to build fneg: {}", e)))?
                        .into(),
                ))
            }
            MirIntrinsicId::F32Lt | MirIntrinsicId::F64Lt => {
                self.compile_float_compare_intrinsic(compiled_args, FloatPredicate::OLT)
            }
            MirIntrinsicId::F32Le | MirIntrinsicId::F64Le => {
                self.compile_float_compare_intrinsic(compiled_args, FloatPredicate::OLE)
            }
            MirIntrinsicId::F32Gt | MirIntrinsicId::F64Gt => {
                self.compile_float_compare_intrinsic(compiled_args, FloatPredicate::OGT)
            }
            MirIntrinsicId::F32Ge | MirIntrinsicId::F64Ge => {
                self.compile_float_compare_intrinsic(compiled_args, FloatPredicate::OGE)
            }
            MirIntrinsicId::F32Eq | MirIntrinsicId::F64Eq => {
                self.compile_float_compare_intrinsic(compiled_args, FloatPredicate::OEQ)
            }
            MirIntrinsicId::F32Ne | MirIntrinsicId::F64Ne => {
                self.compile_float_compare_intrinsic(compiled_args, FloatPredicate::UNE)
            }
            MirIntrinsicId::AtomicU64Exchange
            | MirIntrinsicId::AtomicU64FetchAdd
            | MirIntrinsicId::AtomicU64FetchSub => {
                if compiled_args.len() != 2 {
                    return Err(CodegenError::from(format!(
                        "Atomic U64 operation expected 2 arguments, found {}",
                        compiled_args.len()
                    )));
                }
                let op = match intrinsic {
                    MirIntrinsicId::AtomicU64Exchange => AtomicRMWBinOp::Xchg,
                    MirIntrinsicId::AtomicU64FetchAdd => AtomicRMWBinOp::Add,
                    MirIntrinsicId::AtomicU64FetchSub => AtomicRMWBinOp::Sub,
                    _ => unreachable!(),
                };
                let value = self
                    .builder
                    .build_atomicrmw(
                        op,
                        compiled_args[0].into_pointer_value(),
                        compiled_args[1].into_int_value(),
                        AtomicOrdering::SequentiallyConsistent,
                    )
                    .map_err(|error| {
                        CodegenError::from(format!("Failed atomic U64 operation: {error}"))
                    })?;
                Ok(Some(value.into()))
            }
            MirIntrinsicId::PtrFromParts => {
                let [BasicValueEnum::PointerValue(ptr), BasicValueEnum::IntValue(len)] =
                    compiled_args
                else {
                    return Err(CodegenError::backend_contract(
                        "PtrFromParts requires a pointer and length",
                    ));
                };
                if len.get_type().get_bit_width() != 64
                    || !matches!(ret_ty, Type::Pointer(inner) if matches!(inner.as_ref(), Type::Slice(_) | Type::Str))
                {
                    return Err(CodegenError::backend_contract(
                        "invalid raw wide-pointer signature",
                    ));
                }
                let str_ref_ty = self.slice_layout_type();
                let mut val = str_ref_ty.get_undef();
                val = self
                    .builder
                    .build_insert_value(val, *ptr, 0, "wide_data")
                    .map_err(|e| CodegenError::from(format!("Failed PtrFromParts ptr: {}", e)))?
                    .into_struct_value();
                val = self
                    .builder
                    .build_insert_value(val, *len, 1, "wide_length")
                    .map_err(|e| CodegenError::from(format!("Failed PtrFromParts len: {}", e)))?
                    .into_struct_value();
                Ok(Some(val.into()))
            }
            other => Err(CodegenError::from(format!("Unknown intrinsic: {other}"))),
        }
    }
}
