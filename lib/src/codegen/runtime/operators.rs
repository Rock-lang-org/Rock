use inkwell::values::BasicValueEnum;
use inkwell::{FloatPredicate, IntPredicate};

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{MirBinOp, MirUnaryOp};
use crate::types::Type;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn compile_binop(
        &mut self,
        op: MirBinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let string_like = matches!(ty, Type::Str)
            || matches!(
                ty,
                Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::Str)
            );
        if matches!(op, MirBinOp::Eq | MirBinOp::Ne) && string_like {
            return self
                .compile_str_comparison(lhs, rhs, ty, matches!(op, MirBinOp::Ne))
                .map(Some);
        }

        if ty.is_float() || matches!(lhs, BasicValueEnum::FloatValue(_)) {
            let l = lhs.into_float_value();
            let r = rhs.into_float_value();
            let val = match op {
                MirBinOp::Add => self.builder.build_float_add(l, r, "fadd"),
                MirBinOp::Sub => self.builder.build_float_sub(l, r, "fsub"),
                MirBinOp::Mul => self.builder.build_float_mul(l, r, "fmul"),
                MirBinOp::Div => self.builder.build_float_div(l, r, "fdiv"),
                MirBinOp::Mod => self.builder.build_float_rem(l, r, "frem"),
                MirBinOp::Eq => {
                    return Ok(Some(
                        self.builder
                            .build_float_compare(FloatPredicate::OEQ, l, r, "feq")
                            .map_err(|e| CodegenError::from(format!("{}", e)))?
                            .into(),
                    ))
                }
                MirBinOp::Ne => {
                    return Ok(Some(
                        self.builder
                            .build_float_compare(FloatPredicate::ONE, l, r, "fne")
                            .map_err(|e| CodegenError::from(format!("{}", e)))?
                            .into(),
                    ))
                }
                MirBinOp::Lt => {
                    return Ok(Some(
                        self.builder
                            .build_float_compare(FloatPredicate::OLT, l, r, "flt")
                            .map_err(|e| CodegenError::from(format!("{}", e)))?
                            .into(),
                    ))
                }
                MirBinOp::Le => {
                    return Ok(Some(
                        self.builder
                            .build_float_compare(FloatPredicate::OLE, l, r, "fle")
                            .map_err(|e| CodegenError::from(format!("{}", e)))?
                            .into(),
                    ))
                }
                MirBinOp::Gt => {
                    return Ok(Some(
                        self.builder
                            .build_float_compare(FloatPredicate::OGT, l, r, "fgt")
                            .map_err(|e| CodegenError::from(format!("{}", e)))?
                            .into(),
                    ))
                }
                MirBinOp::Ge => {
                    return Ok(Some(
                        self.builder
                            .build_float_compare(FloatPredicate::OGE, l, r, "fge")
                            .map_err(|e| CodegenError::from(format!("{}", e)))?
                            .into(),
                    ))
                }
                _ => {
                    return Err(CodegenError::from(format!(
                        "Unsupported float operation: {:?}",
                        op
                    )))
                }
            }
            .map_err(|e| CodegenError::from(format!("Failed to build float op: {}", e)))?;
            return Ok(Some(val.into()));
        }

        let l = lhs.into_int_value();
        let r = rhs.into_int_value();
        let (l, r) = if ty.is_unsigned_integer() {
            self.coerce_uint_widths(l, r)?
        } else {
            self.coerce_int_widths(l, r)?
        };

        let val: BasicValueEnum = match op {
            MirBinOp::Add => self.builder.build_int_add(l, r, "add"),
            MirBinOp::Sub => self.builder.build_int_sub(l, r, "sub"),
            MirBinOp::Mul => self.builder.build_int_mul(l, r, "mul"),
            MirBinOp::And => self.builder.build_and(l, r, "and"),
            MirBinOp::Or => self.builder.build_or(l, r, "or"),
            MirBinOp::BitAnd => self.builder.build_and(l, r, "bitand"),
            MirBinOp::BitOr => self.builder.build_or(l, r, "bitor"),
            MirBinOp::BitXor => self.builder.build_xor(l, r, "bitxor"),
            MirBinOp::Shl => self.builder.build_left_shift(l, r, "shl"),
            MirBinOp::Shr => self
                .builder
                .build_right_shift(l, r, !ty.is_unsigned_integer(), "shr"),
            MirBinOp::Div => {
                if ty.is_signed_integer() {
                    self.builder.build_int_signed_div(l, r, "sdiv")
                } else {
                    self.builder.build_int_unsigned_div(l, r, "udiv")
                }
            }
            MirBinOp::Mod => {
                if ty.is_signed_integer() {
                    self.builder.build_int_signed_rem(l, r, "srem")
                } else {
                    self.builder.build_int_unsigned_rem(l, r, "urem")
                }
            }
            MirBinOp::Eq => self.builder.build_int_compare(IntPredicate::EQ, l, r, "eq"),
            MirBinOp::Ne => self.builder.build_int_compare(IntPredicate::NE, l, r, "ne"),
            MirBinOp::Lt => self.builder.build_int_compare(
                if ty.is_unsigned_integer() {
                    IntPredicate::ULT
                } else {
                    IntPredicate::SLT
                },
                l,
                r,
                "lt",
            ),
            MirBinOp::Le => self.builder.build_int_compare(
                if ty.is_unsigned_integer() {
                    IntPredicate::ULE
                } else {
                    IntPredicate::SLE
                },
                l,
                r,
                "le",
            ),
            MirBinOp::Gt => self.builder.build_int_compare(
                if ty.is_unsigned_integer() {
                    IntPredicate::UGT
                } else {
                    IntPredicate::SGT
                },
                l,
                r,
                "gt",
            ),
            MirBinOp::Ge => self.builder.build_int_compare(
                if ty.is_unsigned_integer() {
                    IntPredicate::UGE
                } else {
                    IntPredicate::SGE
                },
                l,
                r,
                "ge",
            ),
        }
        .map_err(|e| CodegenError::from(format!("{}", e)))?
        .into();

        Ok(Some(val))
    }

    pub(crate) fn compile_unaryop(
        &mut self,
        op: MirUnaryOp,
        val: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        match op {
            MirUnaryOp::Neg => {
                if ty.is_float() {
                    let result = self
                        .builder
                        .build_float_neg(val.into_float_value(), "fneg")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to negate float: {}", e))
                        })?;
                    Ok(Some(result.into()))
                } else if ty.is_signed_integer() {
                    let result = self
                        .builder
                        .build_int_neg(val.into_int_value(), "neg")
                        .map_err(|e| CodegenError::from(format!("Failed to negate int: {}", e)))?;
                    Ok(Some(result.into()))
                } else {
                    Err(CodegenError::layout(format!(
                        "Unary negation `-` is not valid for type {} (only signed integers and floats)",
                        self.display_type_for_diagnostic(ty)
                    )))
                }
            }
            MirUnaryOp::Not => {
                let result = self
                    .builder
                    .build_not(val.into_int_value(), "not")
                    .map_err(|e| CodegenError::from(format!("Failed to build not: {}", e)))?;
                Ok(Some(result.into()))
            }
            MirUnaryOp::BitNot => {
                let result = self
                    .builder
                    .build_not(val.into_int_value(), "bitnot")
                    .map_err(|e| CodegenError::from(format!("Failed to build bitnot: {}", e)))?;
                Ok(Some(result.into()))
            }
        }
    }
}
