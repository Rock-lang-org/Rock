use inkwell::values::BasicValueEnum;
use inkwell::IntPredicate;

use crate::codegen::{CodeGen, CodegenError};
use crate::types::Type;

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_str_comparison(
        &mut self,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        lhs_ty: &Type,
        negate: bool,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let function = self
            .current_function
            .ok_or_else(|| CodegenError::from("No current function for string comparison"))?;
        let (lhs_ptr, lhs_len) = self.slice_parts_from_value(lhs, lhs_ty)?;
        let (rhs_ptr, rhs_len) = self.slice_parts_from_value(rhs, &Type::Str)?;
        let bool_ty = self.context.bool_type();
        let i8_ty = self.context.i8_type();
        let i64_ty = self.context.i64_type();

        let result = self
            .builder
            .build_alloca(bool_ty, "str_eq_result")
            .map_err(|error| {
                CodegenError::from(format!(
                    "Failed to allocate string comparison result: {error}"
                ))
            })?;
        let index = self
            .builder
            .build_alloca(i64_ty, "str_eq_index")
            .map_err(|error| {
                CodegenError::from(format!(
                    "Failed to allocate string comparison index: {error}"
                ))
            })?;
        self.builder
            .build_store(index, i64_ty.const_zero())
            .map_err(|error| {
                CodegenError::from(format!(
                    "Failed to initialize string comparison index: {error}"
                ))
            })?;

        let compare = self.context.append_basic_block(function, "str_eq_compare");
        let loop_block = self.context.append_basic_block(function, "str_eq_loop");
        let byte_block = self.context.append_basic_block(function, "str_eq_byte");
        let advance = self.context.append_basic_block(function, "str_eq_advance");
        let success = self.context.append_basic_block(function, "str_eq_success");
        let failure = self.context.append_basic_block(function, "str_eq_failure");
        let done = self.context.append_basic_block(function, "str_eq_done");

        let lengths_equal = self
            .builder
            .build_int_compare(IntPredicate::EQ, lhs_len, rhs_len, "str_lengths_equal")
            .map_err(|error| {
                CodegenError::from(format!("Failed to compare string lengths: {error}"))
            })?;
        self.builder
            .build_conditional_branch(lengths_equal, compare, failure)
            .map_err(|error| {
                CodegenError::from(format!("Failed to branch on string lengths: {error}"))
            })?;

        self.builder.position_at_end(compare);
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| {
                CodegenError::from(format!("Failed to enter string comparison loop: {error}"))
            })?;

        self.builder.position_at_end(loop_block);
        let current_index = self
            .builder
            .build_load(i64_ty, index, "str_eq_current_index")
            .map_err(|error| {
                CodegenError::from(format!("Failed to load string comparison index: {error}"))
            })?
            .into_int_value();
        let at_end = self
            .builder
            .build_int_compare(IntPredicate::EQ, current_index, lhs_len, "str_eq_at_end")
            .map_err(|error| {
                CodegenError::from(format!("Failed to check string comparison end: {error}"))
            })?;
        self.builder
            .build_conditional_branch(at_end, success, byte_block)
            .map_err(|error| {
                CodegenError::from(format!(
                    "Failed to branch in string comparison loop: {error}"
                ))
            })?;

        self.builder.position_at_end(byte_block);
        let lhs_byte_ptr = unsafe {
            self.builder
                .build_gep(i8_ty, lhs_ptr, &[current_index], "str_eq_lhs_byte_ptr")
        }
        .map_err(|error| {
            CodegenError::from(format!("Failed to address left string byte: {error}"))
        })?;
        let rhs_byte_ptr = unsafe {
            self.builder
                .build_gep(i8_ty, rhs_ptr, &[current_index], "str_eq_rhs_byte_ptr")
        }
        .map_err(|error| {
            CodegenError::from(format!("Failed to address right string byte: {error}"))
        })?;
        let lhs_byte = self
            .builder
            .build_load(i8_ty, lhs_byte_ptr, "str_eq_lhs_byte")
            .map_err(|error| {
                CodegenError::from(format!("Failed to load left string byte: {error}"))
            })?
            .into_int_value();
        let rhs_byte = self
            .builder
            .build_load(i8_ty, rhs_byte_ptr, "str_eq_rhs_byte")
            .map_err(|error| {
                CodegenError::from(format!("Failed to load right string byte: {error}"))
            })?
            .into_int_value();
        let bytes_equal = self
            .builder
            .build_int_compare(IntPredicate::EQ, lhs_byte, rhs_byte, "str_bytes_equal")
            .map_err(|error| {
                CodegenError::from(format!("Failed to compare string bytes: {error}"))
            })?;
        self.builder
            .build_conditional_branch(bytes_equal, advance, failure)
            .map_err(|error| {
                CodegenError::from(format!("Failed to branch on string bytes: {error}"))
            })?;

        self.builder.position_at_end(advance);
        let next_index = self
            .builder
            .build_int_add(
                current_index,
                i64_ty.const_int(1, false),
                "str_eq_next_index",
            )
            .map_err(|error| {
                CodegenError::from(format!("Failed to advance string comparison: {error}"))
            })?;
        self.builder
            .build_store(index, next_index)
            .map_err(|error| {
                CodegenError::from(format!("Failed to store string comparison index: {error}"))
            })?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| {
                CodegenError::from(format!("Failed to continue string comparison: {error}"))
            })?;

        self.builder.position_at_end(success);
        self.builder
            .build_store(result, bool_ty.const_int(1, false))
            .map_err(|error| {
                CodegenError::from(format!(
                    "Failed to store successful string comparison: {error}"
                ))
            })?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| {
                CodegenError::from(format!(
                    "Failed to finish successful string comparison: {error}"
                ))
            })?;

        self.builder.position_at_end(failure);
        self.builder
            .build_store(result, bool_ty.const_zero())
            .map_err(|error| {
                CodegenError::from(format!("Failed to store failed string comparison: {error}"))
            })?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| {
                CodegenError::from(format!(
                    "Failed to finish failed string comparison: {error}"
                ))
            })?;

        self.builder.position_at_end(done);
        let equal = self
            .builder
            .build_load(bool_ty, result, "str_eq")
            .map_err(|error| {
                CodegenError::from(format!("Failed to load string comparison result: {error}"))
            })?
            .into_int_value();
        let value = if negate {
            self.builder.build_not(equal, "str_ne").map_err(|error| {
                CodegenError::from(format!("Failed to negate string comparison: {error}"))
            })?
        } else {
            equal
        };
        Ok(value.into())
    }
}
