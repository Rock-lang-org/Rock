use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

use crate::codegen::{CodeGen, CodegenError};
use crate::types::Type;

impl<'ctx> CodeGen<'ctx> {
    fn validate_unicode_scalar(
        &self,
        value: inkwell::values::IntValue<'ctx>,
        from_ty: &Type,
    ) -> Result<(), CodegenError> {
        use inkwell::IntPredicate;

        let error = |e: inkwell::builder::BuilderError| CodegenError::from(e.to_string());
        // Check before narrowing so large integers cannot wrap into valid scalars.
        let wide = self.context.i64_type();
        let value = if value.get_type().get_bit_width() < 64 {
            if from_ty.is_signed_integer() {
                self.builder
                    .build_int_s_extend(value, wide, "char_value")
                    .map_err(error)?
            } else {
                self.builder
                    .build_int_z_extend(value, wide, "char_value")
                    .map_err(error)?
            }
        } else {
            value
        };
        let out_of_range = self
            .builder
            .build_int_compare(
                IntPredicate::UGT,
                value,
                wide.const_int(0x10ffff, false),
                "char_out_of_range",
            )
            .map_err(error)?;
        let above = self
            .builder
            .build_int_compare(
                IntPredicate::UGE,
                value,
                wide.const_int(0xd800, false),
                "char_above_surrogate_start",
            )
            .map_err(error)?;
        let below = self
            .builder
            .build_int_compare(
                IntPredicate::ULE,
                value,
                wide.const_int(0xdfff, false),
                "char_below_surrogate_end",
            )
            .map_err(error)?;
        let surrogate = self
            .builder
            .build_and(above, below, "char_surrogate")
            .map_err(error)?;
        let invalid = self
            .builder
            .build_or(out_of_range, surrogate, "char_invalid")
            .map_err(error)?;
        let function = self
            .builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .ok_or_else(|| CodegenError::from("No current function for Unicode scalar check"))?;
        let fail = self.context.append_basic_block(function, "char_invalid");
        let valid = self.context.append_basic_block(function, "char_valid");
        self.builder
            .build_conditional_branch(invalid, fail, valid)
            .map_err(error)?;
        self.builder.position_at_end(fail);
        let trap = inkwell::intrinsics::Intrinsic::find("llvm.trap")
            .and_then(|intrinsic| intrinsic.get_declaration(&self.module, &[]))
            .ok_or_else(|| CodegenError::from("LLVM trap intrinsic is unavailable"))?;
        self.builder.build_call(trap, &[], "").map_err(error)?;
        self.builder.build_unreachable().map_err(error)?;
        self.builder.position_at_end(valid);
        Ok(())
    }

    pub(super) fn compile_mir_cast_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        from_ty: &Type,
        target_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let to_llvm = self.llvm_type(target_ty);
        let from_is_fat_pointer = self.is_fat_pointer_type(from_ty);
        let target_is_fat_pointer = self.is_fat_pointer_type(target_ty);

        if from_ty.is_integer() && target_ty.is_float() {
            if from_ty.is_signed_integer() {
                return Ok(self
                    .builder
                    .build_signed_int_to_float(
                        value.into_int_value(),
                        to_llvm.into_float_type(),
                        "sitofp",
                    )
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            return Ok(self
                .builder
                .build_unsigned_int_to_float(
                    value.into_int_value(),
                    to_llvm.into_float_type(),
                    "uitofp",
                )
                .map_err(|e| CodegenError::from(format!("{}", e)))?
                .into());
        }

        if from_ty.is_float() && target_ty.is_integer() {
            if target_ty.is_signed_integer() {
                return Ok(self
                    .builder
                    .build_float_to_signed_int(
                        value.into_float_value(),
                        to_llvm.into_int_type(),
                        "fptosi",
                    )
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            return Ok(self
                .builder
                .build_float_to_unsigned_int(
                    value.into_float_value(),
                    to_llvm.into_int_type(),
                    "fptoui",
                )
                .map_err(|e| CodegenError::from(format!("{}", e)))?
                .into());
        }

        if matches!(from_ty, Type::Char) && target_ty.is_integer() {
            let from_width = value.into_int_value().get_type().get_bit_width();
            let to_width = to_llvm.into_int_type().get_bit_width();
            if from_width < to_width {
                return Ok(self
                    .builder
                    .build_int_z_extend(value.into_int_value(), to_llvm.into_int_type(), "zext")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            if from_width > to_width {
                return Ok(self
                    .builder
                    .build_int_truncate(value.into_int_value(), to_llvm.into_int_type(), "trunc")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            return Ok(value);
        }

        if from_ty.is_integer() && matches!(target_ty, Type::Char) {
            self.validate_unicode_scalar(value.into_int_value(), from_ty)?;
            let from_width = value.into_int_value().get_type().get_bit_width();
            let to_width = to_llvm.into_int_type().get_bit_width();
            if from_width < to_width {
                return Ok(self
                    .builder
                    .build_int_z_extend(value.into_int_value(), to_llvm.into_int_type(), "zext")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            if from_width > to_width {
                return Ok(self
                    .builder
                    .build_int_truncate(value.into_int_value(), to_llvm.into_int_type(), "trunc")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            return Ok(value);
        }

        if from_ty.is_integer() && target_ty.is_integer() {
            let from_width = value.into_int_value().get_type().get_bit_width();
            let to_width = to_llvm.into_int_type().get_bit_width();
            if from_width < to_width {
                if from_ty.is_signed_integer() {
                    return Ok(self
                        .builder
                        .build_int_s_extend(value.into_int_value(), to_llvm.into_int_type(), "sext")
                        .map_err(|e| CodegenError::from(format!("{}", e)))?
                        .into());
                }
                return Ok(self
                    .builder
                    .build_int_z_extend(value.into_int_value(), to_llvm.into_int_type(), "zext")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            if from_width > to_width {
                return Ok(self
                    .builder
                    .build_int_truncate(value.into_int_value(), to_llvm.into_int_type(), "trunc")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            return Ok(value);
        }

        if from_ty.is_float() && target_ty.is_float() {
            let from_value = value.into_float_value();
            if matches!(from_ty, Type::F32) && matches!(target_ty, Type::F64) {
                return Ok(self
                    .builder
                    .build_float_ext(from_value, self.context.f64_type(), "fpext")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            if matches!(from_ty, Type::F64) && matches!(target_ty, Type::F32) {
                return Ok(self
                    .builder
                    .build_float_trunc(from_value, self.context.f32_type(), "fptrunc")
                    .map_err(|e| CodegenError::from(format!("{}", e)))?
                    .into());
            }
            return Ok(value);
        }

        if from_is_fat_pointer && target_is_fat_pointer {
            if !matches!(value, BasicValueEnum::StructValue(_)) {
                return Err(CodegenError::from(format!(
                    "MIR fat pointer cast expected slice-layout value from {} to {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            }
            return Ok(value);
        }

        if matches!(from_ty, Type::Pointer(_)) && matches!(target_ty, Type::Pointer(_)) {
            if from_is_fat_pointer || target_is_fat_pointer {
                return Err(CodegenError::from(format!(
                    "Cannot cast between thin and fat MIR pointers from {} to {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            }
            let BasicValueEnum::PointerValue(pointer) = value else {
                return Err(CodegenError::from(format!(
                    "MIR pointer-to-pointer cast expected pointer value from {} to {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            };
            return Ok(self
                .builder
                .build_pointer_cast(
                    pointer,
                    self.context.ptr_type(AddressSpace::default()),
                    "ptrcast",
                )
                .map_err(|e| CodegenError::from(format!("{}", e)))?
                .into());
        }

        if matches!(from_ty, Type::Pointer(_)) && target_ty.is_integer() {
            if from_is_fat_pointer {
                return Err(CodegenError::from(format!(
                    "Cannot cast fat MIR pointer {} to integer {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            }
            let BasicValueEnum::PointerValue(pointer) = value else {
                return Err(CodegenError::from(format!(
                    "MIR pointer-to-integer cast expected pointer value from {} to {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            };
            return Ok(self
                .builder
                .build_ptr_to_int(pointer, to_llvm.into_int_type(), "ptrtoint")
                .map_err(|e| CodegenError::from(format!("{}", e)))?
                .into());
        }

        if from_ty.is_integer() && matches!(target_ty, Type::Pointer(_)) {
            if target_is_fat_pointer {
                return Err(CodegenError::from(format!(
                    "Cannot cast integer to fat MIR pointer {} from {}",
                    self.display_type_for_diagnostic(target_ty),
                    self.display_type_for_diagnostic(from_ty)
                )));
            }
            let BasicValueEnum::IntValue(integer) = value else {
                return Err(CodegenError::from(format!(
                    "MIR integer-to-pointer cast expected integer value from {} to {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            };
            return Ok(self
                .builder
                .build_int_to_ptr(
                    integer,
                    self.context.ptr_type(AddressSpace::default()),
                    "inttoptr",
                )
                .map_err(|e| CodegenError::from(format!("{}", e)))?
                .into());
        }

        if matches!(from_ty, Type::Reference { .. }) && target_is_fat_pointer {
            let (data_ptr, len) = self.slice_parts_from_value(value, from_ty)?;
            return self.make_fat_slice_value(data_ptr, len);
        }

        if matches!(from_ty, Type::Reference { .. }) && matches!(target_ty, Type::Pointer(_)) {
            if from_is_fat_pointer || target_is_fat_pointer {
                return Err(CodegenError::from(format!(
                    "Cannot cast fat MIR pointer {} to thin pointer {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            }
            if !matches!(value, BasicValueEnum::PointerValue(_)) {
                return Err(CodegenError::from(format!(
                    "MIR reference-to-pointer cast expected pointer value from {} to {}",
                    self.display_type_for_diagnostic(from_ty),
                    self.display_type_for_diagnostic(target_ty)
                )));
            }
            return Ok(value);
        }

        if from_ty == target_ty {
            return Ok(value);
        }

        Err(CodegenError::from(format!(
            "Unsupported MIR cast from {} to {}",
            self.display_type_for_diagnostic(from_ty),
            self.display_type_for_diagnostic(target_ty)
        )))
    }
}
