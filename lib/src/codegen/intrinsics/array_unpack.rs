use inkwell::values::BasicValueEnum;

use crate::codegen::{CodeGen, CodegenError};
use crate::types::Type;

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_array_unpack(
        &mut self,
        values: &[BasicValueEnum<'ctx>],
        types: &[Type],
        result: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let ([value], [Type::Array(element, length)], Type::Tuple(fields)) =
            (values, types, result)
        else {
            return Err(CodegenError::from(
                "ArrayUnpack requires one array and a tuple result",
            ));
        };
        if length.concrete() != Some(fields.len() as u64)
            || fields.iter().any(|field| field != element.as_ref())
        {
            return Err(CodegenError::from(
                "ArrayUnpack result must contain every array element exactly once",
            ));
        }
        let mut tuple = self.llvm_type(result)?.into_struct_type().get_undef();
        for index in 0..fields.len() {
            let field = self
                .builder
                .build_extract_value(value.into_array_value(), index as u32, "unpack_element")
                .map_err(|error| {
                    CodegenError::from(format!("ArrayUnpack extraction failed: {error}"))
                })?;
            tuple = self
                .builder
                .build_insert_value(tuple, field, index as u32, "unpack_tuple")
                .map_err(|error| {
                    CodegenError::from(format!("ArrayUnpack insertion failed: {error}"))
                })?
                .into_struct_value();
        }
        Ok(tuple.into())
    }
}
