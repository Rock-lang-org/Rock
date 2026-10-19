use inkwell::values::BasicValueEnum;

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{Constant, MirCallable, MirFunction, Operand};
use crate::types::Type;

use super::MirFunctionContext;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn compile_mir_operand(
        &mut self,
        _function: &MirFunction,
        ctx: &MirFunctionContext<'ctx>,
        operand: &Operand,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => self.compile_mir_place_value(ctx, place),
            Operand::Constant(constant) => self.compile_mir_constant(constant),
        }
    }

    pub(crate) fn compile_mir_constant(
        &self,
        constant: &Constant,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match constant {
            Constant::Int(value) => Ok(self
                .context
                .i64_type()
                .const_int(*value as u64, true)
                .into()),
            Constant::Float(value) => Ok(self.context.f64_type().const_float(*value).into()),
            Constant::Bool(value) => Ok(self
                .context
                .bool_type()
                .const_int(u64::from(*value), false)
                .into()),
            Constant::Char(value) => Ok(self
                .context
                .i32_type()
                .const_int(*value as u64, false)
                .into()),
            Constant::String(value) => {
                let processed = Self::process_escape_sequences(value);
                let byte_len = processed.len() as u64;
                // LLVMBuildGlobalStringPtr takes a C string and truncates at NUL.
                // Build the byte array explicitly to preserve the stored length.
                let bytes = self.context.const_string(processed.as_bytes(), true);
                let str_ptr = self.module.add_global(bytes.get_type(), None, "str");
                str_ptr.set_initializer(&bytes);
                str_ptr.set_constant(true);
                str_ptr.set_linkage(inkwell::module::Linkage::Private);
                let mut str_value = self.slice_layout_type().get_undef();
                str_value = self
                    .builder
                    .build_insert_value(str_value, str_ptr.as_pointer_value(), 0, "mir_str_ptr")
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to insert MIR string ptr: {}", e))
                    })?
                    .into_struct_value();
                str_value = self
                    .builder
                    .build_insert_value(
                        str_value,
                        self.context.i64_type().const_int(byte_len, false),
                        1,
                        "mir_str_len",
                    )
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to insert MIR string len: {}", e))
                    })?
                    .into_struct_value();
                Ok(str_value.into())
            }
            Constant::Unit => Ok(self.context.i64_type().const_int(0, false).into()),
            Constant::Callable(_) => Err(CodegenError::from(
                "MIR callable constant reached scalar lowering without an expected function type",
            )),
            Constant::TypeId(_) => Err(CodegenError::from(
                "MIR TypeId metadata reached runtime scalar lowering",
            )),
        }
    }

    pub(crate) fn compile_mir_callable_constant(
        &mut self,
        function: &MirFunction,
        callable: &MirCallable,
        expected_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let symbol = self.resolve_mir_callable_symbol_in_function(callable, function)?;
        let signature = self
            .resolve_mir_callable_signature(callable)
            .ok_or_else(|| {
                CodegenError::backend_contract(format!(
                    "Missing MIR callable signature for {:?} in MIR function '{}'",
                    callable, function.name
                ))
            })?;
        self.materialize_named_callable(&symbol, expected_ty, Some(&signature))?
            .ok_or_else(|| CodegenError::from("MIR callable materialized no value"))
    }
}
