use inkwell::values::BasicValueEnum;

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{MirAssert, MirAssertKind, MirFunction};
use crate::types::Type;

use super::MirFunctionContext;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn compile_mir_assert(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        assertion: &MirAssert,
    ) -> Result<(), CodegenError> {
        match assertion.kind {
            MirAssertKind::BoundsCheck => {
                self.compile_mir_bounds_check(function, context, assertion)
            }
        }
    }

    fn compile_mir_bounds_check(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        assertion: &MirAssert,
    ) -> Result<(), CodegenError> {
        let [base, index] = assertion.operands.as_slice() else {
            return Err(CodegenError::from(format!(
                "MIR bounds check expected 2 operands, got {}",
                assertion.operands.len()
            )));
        };

        let base_ty = self.mir_operand_ty(context, base)?;
        let len = match &base_ty {
            Type::Array(_, len) => self.context.i64_type().const_int(len.as_concrete(), false),
            Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::Array(_, _)) => {
                let Type::Array(_, len) = inner.as_ref() else {
                    unreachable!();
                };
                self.context.i64_type().const_int(len.as_concrete(), false)
            }
            Type::Slice(_) | Type::Str | Type::Pointer(_) | Type::Reference { .. } => {
                let value = self.compile_mir_operand(function, context, base)?;
                let (_, len) = self.slice_parts_from_value(value, &base_ty).map_err(|e| {
                    CodegenError::from(format!("Unsupported MIR bounds check base: {}", e))
                })?;
                len
            }
            ty => {
                return Err(CodegenError::layout(format!(
                    "Unsupported MIR bounds check base: {}",
                    self.display_type_for_diagnostic(ty)
                )))
            }
        };
        let index_ty = self.mir_operand_ty(context, index)?;
        let index_value = self.compile_mir_operand(function, context, index)?;
        let BasicValueEnum::IntValue(index) = index_value else {
            return Err(CodegenError::layout(format!(
                "MIR bounds check expected integer index, got {}",
                self.display_type_for_diagnostic(&index_ty)
            )));
        };
        let index = self
            .builder
            .build_int_cast(index, self.context.i64_type(), "bounds_idx")
            .map_err(|e| CodegenError::from(format!("Failed to cast MIR bounds index: {}", e)))?;

        self.emit_index_bounds_check(index, len)
    }
}
