use inkwell::module::Linkage;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum};
use inkwell::AddressSpace;

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{MirCallableSignature, MirParamAbi, MirPassMode};
use crate::types::Type;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn materialize_named_callable(
        &mut self,
        name: &str,
        ty: &Type,
        signature: Option<&MirCallableSignature>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let func = self
            .functions
            .get(name)
            .copied()
            .ok_or_else(|| CodegenError::from(format!("Unknown function: {}", name)))?;

        let (param_types, ret_type) = match ty {
            Type::Function { params, ret, .. } => (params.clone(), ret.as_ref().clone()),
            _ => {
                return Err(CodegenError::from(format!(
                    "Expected function type for '{}', got {}",
                    name, ty
                )))
            }
        };

        let signature = signature.cloned();
        if let Some(signature) = &signature {
            if signature.params.len() != param_types.len() {
                return Err(CodegenError::from(format!(
                    "MIR callable '{}' signature has {} params but function type has {} params",
                    name,
                    signature.params.len(),
                    param_types.len()
                )));
            }
        }

        let wrapper_key = (
            name.to_string(),
            self.intern_structural_type(ty),
            signature.clone(),
        );

        let wrapper = if let Some(wrapper) = self.function_value_wrappers.get(&wrapper_key).copied()
        {
            wrapper
        } else {
            let wrapper_name = format!(
                "__callable_wrap_{}_{}",
                Self::sanitize_symbol(name),
                self.function_value_wrappers.len()
            );
            let wrapper_fn = self.module.add_function(
                &wrapper_name,
                self.callable_code_type(&param_types, &ret_type),
                None,
            );
            wrapper_fn.set_linkage(Linkage::Internal);

            let saved_function = self.current_function;
            let saved_block = self.builder.get_insert_block();

            self.current_function = Some(wrapper_fn);
            let entry = self.context.append_basic_block(wrapper_fn, "entry");
            self.builder.position_at_end(entry);

            let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> =
                Vec::with_capacity(param_types.len());
            for (idx, _) in param_types.iter().enumerate() {
                let arg = wrapper_fn
                    .get_nth_param((idx + 1) as u32)
                    .ok_or(CodegenError::from("Wrapper argument not found"))?;
                if let Some(param_abi) = signature.as_ref().and_then(|sig| sig.params.get(idx)) {
                    call_args.push(self.compile_named_callable_wrapper_arg(name, arg, param_abi)?);
                } else {
                    call_args.push(arg.into());
                }
            }

            let result = self
                .builder
                .build_call(func, &call_args, "named_callable")
                .map_err(|e| CodegenError::from(format!("Failed to build wrapper call: {}", e)))?;

            match ret_type {
                Type::Unit => {
                    self.builder.build_return(None).map_err(|e| {
                        CodegenError::from(format!("Failed to return wrapper unit: {}", e))
                    })?;
                }
                _ => {
                    let ret = result
                        .try_as_basic_value()
                        .left()
                        .ok_or(CodegenError::from(
                            "Wrapper expected a return value but got void",
                        ))?;
                    let ret = self.coerce_value(ret, self.llvm_type(&ret_type))?;
                    self.builder.build_return(Some(&ret)).map_err(|e| {
                        CodegenError::from(format!("Failed to return wrapper value: {}", e))
                    })?;
                }
            }

            self.current_function = saved_function;
            if let Some(block) = saved_block {
                self.builder.position_at_end(block);
            }

            self.function_value_wrappers.insert(wrapper_key, wrapper_fn);
            wrapper_fn
        };

        let null_env = self.context.ptr_type(AddressSpace::default()).const_null();
        Ok(Some(self.build_callable_value(
            wrapper.as_global_value().as_pointer_value(),
            null_env,
        )?))
    }

    fn compile_named_callable_wrapper_arg(
        &mut self,
        name: &str,
        arg: BasicValueEnum<'ctx>,
        param: &MirParamAbi,
    ) -> Result<BasicMetadataValueEnum<'ctx>, CodegenError> {
        match param.pass_mode {
            MirPassMode::Direct | MirPassMode::FatDirect => Ok(arg.into()),
            MirPassMode::Pointer => {
                if self.type_id_lowers_to_pointer(param.semantic_ty) {
                    return Ok(arg.into());
                }

                let temp = self
                    .builder
                    .build_alloca(self.llvm_type_id(param.semantic_ty), "mir_pointer_arg")
                    .map_err(|e| {
                        CodegenError::from(format!(
                            "Failed to allocate MIR pointer ABI wrapper argument for '{}': {}",
                            name, e
                        ))
                    })?;
                let coerced = self.coerce_value(arg, self.llvm_type_id(param.semantic_ty))?;
                self.builder.build_store(temp, coerced).map_err(|e| {
                    CodegenError::from(format!(
                        "Failed to store MIR pointer ABI wrapper argument for '{}': {}",
                        name, e
                    ))
                })?;
                Ok(temp.into())
            }
        }
    }
}
