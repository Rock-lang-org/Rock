//! Poison-free integer arithmetic, including exceptional division and shifts.

use inkwell::intrinsics::Intrinsic;
use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::IntPredicate;

use crate::mir::integer::{IntegerIntrinsic, IntegerMode, IntegerOp};
use crate::types::Type;

use super::{CodeGen, CodegenError};

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn compile_typed_integer(
        &mut self,
        args: &[BasicValueEnum<'ctx>],
        op: IntegerOp,
        signed: bool,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let a = args[0].into_int_value();
        let b = args[1].into_int_value();
        let (a, b) = if signed {
            self.coerce_int_widths(a, b)?
        } else {
            self.coerce_uint_widths(a, b)?
        };
        let ty = match (a.get_type().get_bit_width(), signed) {
            (8, true) => Type::I8, (16, true) => Type::I16, (32, true) => Type::I32,
            (64, true) => Type::I64, (8, false) => Type::U8, (16, false) => Type::U16,
            (32, false) => Type::U32, (64, false) => Type::U64,
            _ => return Err(CodegenError::internal("unsupported integer width")),
        };
        self.compile_integer_intrinsic(IntegerIntrinsic {
            op,
            mode: if matches!(op, IntegerOp::Div | IntegerOp::Rem) {
                IntegerMode::Checked
            } else {
                IntegerMode::Wrapping
            },
        }, &[a.into(), b.into()], &[ty.clone(), ty])
    }

    pub(crate) fn trap_if(&mut self, condition: IntValue<'ctx>) -> Result<(), CodegenError> {
        let function = self.builder.get_insert_block().and_then(|block| block.get_parent())
            .ok_or_else(|| CodegenError::internal("integer check outside a function"))?;
        let failed = self.context.append_basic_block(function, "arithmetic_failure");
        let continued = self.context.append_basic_block(function, "arithmetic_continue");
        self.builder.build_conditional_branch(condition, failed, continued)
            .map_err(|error| CodegenError::from(error.to_string()))?;
        self.builder.position_at_end(failed);
        let trap = Intrinsic::find("llvm.trap")
            .and_then(|intrinsic| intrinsic.get_declaration(&self.module, &[]))
            .ok_or_else(|| CodegenError::internal("LLVM trap intrinsic unavailable"))?;
        self.builder.build_call(trap, &[], "")
            .map_err(|error| CodegenError::from(error.to_string()))?;
        self.builder.build_unreachable().map_err(|error| CodegenError::from(error.to_string()))?;
        self.builder.position_at_end(continued);
        Ok(())
    }

    pub(crate) fn compile_integer_intrinsic(
        &mut self,
        intrinsic: IntegerIntrinsic,
        args: &[BasicValueEnum<'ctx>],
        types: &[Type],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        use IntegerOp::*;
        let err = |error: inkwell::builder::BuilderError| CodegenError::from(error.to_string());
        if args.len() != intrinsic.arity() || types.len() != args.len() {
            return Err(CodegenError::internal("invalid integer intrinsic arity"));
        }
        let signed = matches!(types[0], Type::I8 | Type::I16 | Type::I32 | Type::I64);
        if !signed && !matches!(types[0], Type::U8 | Type::U16 | Type::U32 | Type::U64) {
            return Err(CodegenError::internal("non-integer arithmetic intrinsic operand"));
        }
        let a = args[0].into_int_value();
        let ty = a.get_type();
        let width = ty.get_bit_width();
        let zero = ty.const_zero();
        let one = ty.const_int(1, false);
        let false_value = self.context.bool_type().const_zero();
        let (value, overflow) = match intrinsic.op {
            Add | Sub | Mul | Neg => {
                let (lhs, rhs) = if intrinsic.op == Neg {
                    (zero, a)
                } else {
                    (a, args[1].into_int_value())
                };
                let operation = match intrinsic.op { Add => "add", Mul => "mul", _ => "sub" };
                let name = format!("llvm.{}{}.with.overflow.i{width}", if signed { "s" } else { "u" }, operation);
                let function = Intrinsic::find(&name)
                    .and_then(|intrinsic| intrinsic.get_declaration(&self.module, &[ty.into()]))
                    .ok_or_else(|| CodegenError::internal("LLVM overflow intrinsic unavailable"))?;
                let pair = self.builder.build_call(function, &[lhs.into(), rhs.into()], "overflow_pair")
                    .map_err(err)?.try_as_basic_value().left()
                    .ok_or_else(|| CodegenError::internal("LLVM overflow intrinsic has no result"))?
                    .into_struct_value();
                (
                    self.builder.build_extract_value(pair, 0, "arithmetic_value").map_err(err)?.into_int_value(),
                    self.builder.build_extract_value(pair, 1, "arithmetic_overflow").map_err(err)?.into_int_value(),
                )
            }
            Div | Rem => {
                let b = args[1].into_int_value();
                let zero_divisor = self.builder.build_int_compare(IntPredicate::EQ, b, zero, "zero_divisor").map_err(err)?;
                self.trap_if(zero_divisor)?;
                let overflow = if signed {
                    let min = ty.const_int(1u64 << (width - 1), false);
                    let is_min = self.builder.build_int_compare(IntPredicate::EQ, a, min, "is_min").map_err(err)?;
                    let is_minus_one = self.builder.build_int_compare(IntPredicate::EQ, b, ty.const_all_ones(), "minus_one").map_err(err)?;
                    self.builder.build_and(is_min, is_minus_one, "division_overflow").map_err(err)?
                } else {
                    false_value
                };
                // Selecting a safe divisor prevents poison even when LLVM folds the operation.
                let divisor = self.builder.build_select(overflow, one, b, "safe_divisor").map_err(err)?.into_int_value();
                let value = match (signed, intrinsic.op) {
                    (true, Div) => self.builder.build_int_signed_div(a, divisor, "signed_div"),
                    (false, Div) => self.builder.build_int_unsigned_div(a, divisor, "unsigned_div"),
                    (true, _) => self.builder.build_int_signed_rem(a, divisor, "signed_rem"),
                    (false, _) => self.builder.build_int_unsigned_rem(a, divisor, "unsigned_rem"),
                }.map_err(err)?;
                (value, overflow)
            }
            Shl | Shr => {
                let count = args[1].into_int_value();
                let out_of_range = self.builder.build_int_compare(IntPredicate::UGE, count,
                    count.get_type().const_int(width as u64, false), "shift_overflow").map_err(err)?;
                let masked = self.builder.build_and(count,
                    count.get_type().const_int((width - 1) as u64, false), "masked_shift").map_err(err)?;
                let masked = self.builder.build_int_cast(masked, ty, "shift_width").map_err(err)?;
                let value = if intrinsic.op == Shl {
                    self.builder.build_left_shift(a, masked, "shift_left")
                } else {
                    self.builder.build_right_shift(a, masked, signed, "shift_right")
                }.map_err(err)?;
                (value, out_of_range)
            }
        };
        if intrinsic.mode == IntegerMode::Checked {
            self.trap_if(overflow)?;
        }
        if intrinsic.mode == IntegerMode::Overflowing {
            let tuple = self.context.struct_type(&[ty.into(), self.context.bool_type().into()], false).get_undef();
            let tuple = self.builder.build_insert_value(tuple, value, 0, "result_value").map_err(err)?.into_struct_value();
            let tuple = self.builder.build_insert_value(tuple, overflow, 1, "result_overflow").map_err(err)?;
            Ok(Some(tuple.into_struct_value().into()))
        } else {
            Ok(Some(value.into()))
        }
    }
}
