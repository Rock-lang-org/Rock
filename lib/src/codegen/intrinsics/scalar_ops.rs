use inkwell::values::BasicValueEnum;
use inkwell::{FloatPredicate, IntPredicate};

use crate::codegen::{CodeGen, CodegenError};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_int_intrinsic<F>(
        &mut self,
        compiled_args: &[BasicValueEnum<'ctx>],
        label: &str,
        build: F,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError>
    where
        F: FnOnce(
            &mut Self,
            inkwell::values::IntValue<'ctx>,
            inkwell::values::IntValue<'ctx>,
        )
            -> Result<inkwell::values::IntValue<'ctx>, inkwell::builder::BuilderError>,
    {
        let lhs = compiled_args[0].into_int_value();
        let rhs = compiled_args[1].into_int_value();
        let (lhs, rhs) = self.coerce_int_widths(lhs, rhs)?;
        Ok(Some(
            build(self, lhs, rhs)
                .map_err(|e| CodegenError::from(format!("Failed to build {label}: {e}")))?
                .into(),
        ))
    }

    pub(super) fn compile_uint_intrinsic<F>(
        &mut self,
        compiled_args: &[BasicValueEnum<'ctx>],
        label: &str,
        build: F,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError>
    where
        F: FnOnce(
            &mut Self,
            inkwell::values::IntValue<'ctx>,
            inkwell::values::IntValue<'ctx>,
        )
            -> Result<inkwell::values::IntValue<'ctx>, inkwell::builder::BuilderError>,
    {
        let lhs = compiled_args[0].into_int_value();
        let rhs = compiled_args[1].into_int_value();
        let (lhs, rhs) = self.coerce_uint_widths(lhs, rhs)?;
        Ok(Some(
            build(self, lhs, rhs)
                .map_err(|e| CodegenError::from(format!("Failed to build {label}: {e}")))?
                .into(),
        ))
    }

    pub(super) fn compile_int_compare_intrinsic(
        &mut self,
        compiled_args: &[BasicValueEnum<'ctx>],
        predicate: IntPredicate,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let lhs = compiled_args[0].into_int_value();
        let rhs = compiled_args[1].into_int_value();
        let (lhs, rhs) = self.coerce_int_widths(lhs, rhs)?;
        Ok(Some(
            self.builder
                .build_int_compare(predicate, lhs, rhs, "cmp")
                .map_err(|e| CodegenError::from(format!("Failed to build cmp: {e}")))?
                .into(),
        ))
    }

    pub(super) fn compile_uint_compare_intrinsic(
        &mut self,
        compiled_args: &[BasicValueEnum<'ctx>],
        predicate: IntPredicate,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let lhs = compiled_args[0].into_int_value();
        let rhs = compiled_args[1].into_int_value();
        let (lhs, rhs) = self.coerce_uint_widths(lhs, rhs)?;
        Ok(Some(
            self.builder
                .build_int_compare(predicate, lhs, rhs, "cmp")
                .map_err(|e| CodegenError::from(format!("Failed to build cmp: {e}")))?
                .into(),
        ))
    }

    pub(super) fn compile_float_intrinsic<F>(
        &mut self,
        compiled_args: &[BasicValueEnum<'ctx>],
        label: &str,
        build: F,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError>
    where
        F: FnOnce(
            &mut Self,
            inkwell::values::FloatValue<'ctx>,
            inkwell::values::FloatValue<'ctx>,
        )
            -> Result<inkwell::values::FloatValue<'ctx>, inkwell::builder::BuilderError>,
    {
        let lhs = compiled_args[0].into_float_value();
        let rhs = compiled_args[1].into_float_value();
        Ok(Some(
            build(self, lhs, rhs)
                .map_err(|e| CodegenError::from(format!("Failed to build {label}: {e}")))?
                .into(),
        ))
    }

    pub(super) fn compile_float_compare_intrinsic(
        &mut self,
        compiled_args: &[BasicValueEnum<'ctx>],
        predicate: FloatPredicate,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let lhs = compiled_args[0].into_float_value();
        let rhs = compiled_args[1].into_float_value();
        Ok(Some(
            self.builder
                .build_float_compare(predicate, lhs, rhs, "cmp")
                .map_err(|e| CodegenError::from(format!("Failed to build cmp: {e}")))?
                .into(),
        ))
    }
}
