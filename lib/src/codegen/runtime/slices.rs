use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::IntPredicate;

use crate::codegen::{CodeGen, CodegenError};
use crate::types::Type;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn emit_index_bounds_check(
        &mut self,
        idx: inkwell::values::IntValue<'ctx>,
        len: inkwell::values::IntValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let zero = self.context.i64_type().const_int(0, false);
        let is_neg = self
            .builder
            .build_int_compare(IntPredicate::SLT, idx, zero, "idx_neg")
            .map_err(|e| CodegenError::from(format!("Failed to build neg check: {}", e)))?;
        let is_oob = self
            .builder
            .build_int_compare(IntPredicate::SGE, idx, len, "idx_oob")
            .map_err(|e| CodegenError::from(format!("Failed to build oob check: {}", e)))?;
        let fail = self
            .builder
            .build_or(is_neg, is_oob, "idx_fail")
            .map_err(|e| CodegenError::from(format!("Failed to build bounds or: {}", e)))?;

        let function = self
            .current_function
            .ok_or(CodegenError::from("No current function for bounds check"))?;
        let fail_bb = self.context.append_basic_block(function, "arr_oob");
        let exit_bb = self.context.append_basic_block(function, "oob_exit");
        let ok_bb = self.context.append_basic_block(function, "arr_ok");

        self.builder
            .build_conditional_branch(fail, fail_bb, ok_bb)
            .map_err(|e| CodegenError::from(format!("Failed to build bounds branch: {}", e)))?;

        self.builder.position_at_end(fail_bb);
        if let Some(puts_fn) = self.functions.get("puts").copied() {
            let msg = self
                .builder
                .build_global_string_ptr("index out of bounds", "oob_msg")
                .map_err(|e| CodegenError::from(format!("Failed to create oob msg: {}", e)))?;
            self.builder
                .build_call(puts_fn, &[msg.as_pointer_value().into()], "oob_puts")
                .map_err(|e| CodegenError::from(format!("Failed to call puts: {}", e)))?;
        }
        self.builder
            .build_unconditional_branch(exit_bb)
            .map_err(|e| CodegenError::from(format!("Failed to branch to oob exit: {}", e)))?;

        self.builder.position_at_end(exit_bb);
        let exit_fn = self
            .functions
            .get("exit")
            .copied()
            .ok_or(CodegenError::from("exit not declared"))?;
        let exit_code = self.context.i32_type().const_int(1, false);
        self.builder
            .build_call(exit_fn, &[exit_code.into()], "oob_exit")
            .map_err(|e| CodegenError::from(format!("Failed to call exit: {}", e)))?;
        self.builder
            .build_unreachable()
            .map_err(|e| CodegenError::from(format!("Failed to build unreachable: {}", e)))?;

        self.builder.position_at_end(ok_bb);

        Ok(())
    }

    pub(crate) fn make_fat_slice_value(
        &self,
        data_ptr: PointerValue<'ctx>,
        len: inkwell::values::IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let mut slice_ref = self.slice_layout_type().get_undef();
        slice_ref = self
            .builder
            .build_insert_value(slice_ref, data_ptr, 0, "slice_ref_ptr")
            .map_err(|e| CodegenError::from(format!("Failed to insert slice ref ptr: {}", e)))?
            .into_struct_value();
        slice_ref = self
            .builder
            .build_insert_value(slice_ref, len, 1, "slice_ref_len")
            .map_err(|e| CodegenError::from(format!("Failed to insert slice ref len: {}", e)))?
            .into_struct_value();

        Ok(slice_ref.into())
    }

    pub(crate) fn slice_parts_from_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<(PointerValue<'ctx>, inkwell::values::IntValue<'ctx>), CodegenError> {
        let resolved_ty = self.resolve_projection_type(ty);

        match resolved_ty {
            Type::Slice(_) | Type::Str => self.slice_parts_from_struct(value, "slice"),
            Type::Pointer(inner) if matches!(inner.as_ref(), Type::Slice(_) | Type::Str) => {
                self.slice_parts_from_struct(value, "slice")
            }
            Type::Array(inner, len) => {
                let array_ty = self.llvm_type(&Type::Array(inner.clone(), len));
                let temp = self
                    .builder
                    .build_alloca(array_ty, "array_tmp")
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to allocate array temp: {}", e))
                    })?;
                self.builder.build_store(temp, value).map_err(|e| {
                    CodegenError::from(format!("Failed to store array temp: {}", e))
                })?;
                let zero = self.context.i64_type().const_int(0, false);
                let data_ptr = unsafe {
                    self.builder
                        .build_gep(array_ty, temp, &[zero, zero], "array_data_ptr")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to get array data ptr: {}", e))
                        })?
                };
                Ok((
                    data_ptr,
                    self.context.i64_type().const_int(len as u64, false),
                ))
            }
            Type::Reference { inner, .. }
                if matches!(inner.as_ref(), Type::Slice(_) | Type::Str) =>
            {
                self.slice_parts_from_struct(value, "slice_ref")
            }
            Type::Reference { inner, .. } => match inner.as_ref() {
                Type::Array(elem, len) => {
                    let ptr = value.into_pointer_value();
                    let zero = self.context.i64_type().const_int(0, false);
                    let array_ty = self.llvm_type(&Type::Array(elem.clone(), *len));
                    let data_ptr = unsafe {
                        self.builder
                            .build_gep(array_ty, ptr, &[zero, zero], "array_ref_data_ptr")
                            .map_err(|e| {
                                CodegenError::from(format!(
                                    "Failed to get array ref data ptr: {}",
                                    e
                                ))
                            })?
                    };
                    Ok((
                        data_ptr,
                        self.context.i64_type().const_int(*len as u64, false),
                    ))
                }
                _ => Err(CodegenError::from("Expected slice-compatible reference")),
            },
            _ => Err(CodegenError::from("Expected slice-compatible value")),
        }
    }

    fn slice_parts_from_struct(
        &self,
        value: BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<(PointerValue<'ctx>, inkwell::values::IntValue<'ctx>), CodegenError> {
        let slice = value.into_struct_value();
        let data_ptr = self
            .builder
            .build_extract_value(slice, 0, &format!("{}_ptr", name))
            .map_err(|e| CodegenError::from(format!("Failed to extract {} ptr: {}", name, e)))?
            .into_pointer_value();
        let len = self
            .builder
            .build_extract_value(slice, 1, &format!("{}_len", name))
            .map_err(|e| CodegenError::from(format!("Failed to extract {} len: {}", name, e)))?
            .into_int_value();
        Ok((data_ptr, len))
    }
}
