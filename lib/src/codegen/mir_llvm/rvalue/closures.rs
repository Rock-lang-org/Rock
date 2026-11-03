use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{MirClosure, MirFunctionId};
use crate::types::Type;

use super::super::MirFunctionContext;

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_mir_closure_value(
        &mut self,
        context: &MirFunctionContext<'ctx>,
        closure: &MirClosure,
        expected_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let closure_function_id = MirFunctionId::Closure(Box::new(closure.id.clone()));
        self.validate_mir_closure_value(context, closure, &closure_function_id, expected_ty)?;
        let symbol = self
            .mir_function_symbols
            .get(&closure_function_id)
            .cloned()
            .ok_or_else(|| {
                CodegenError::backend_contract(
                    "MIR closure function was not declared before codegen",
                )
            })?;
        let closure_fn = self.functions.get(&symbol).copied().ok_or_else(|| {
            CodegenError::layout(format!(
                "LLVM function '{}' not found for MIR closure",
                symbol
            ))
        })?;

        let mut capture_values = Vec::with_capacity(closure.captures.len());
        for capture in &closure.captures {
            let place = capture.place();
            let (capture_ptr, capture_ty) = self.compile_mir_place_local(context, &place)?;
            match capture.kind {
                crate::mir::MirClosureCaptureKind::ByValue => {
                    let value = self
                        .builder
                        .build_load(
                            self.llvm_type(&capture_ty),
                            capture_ptr,
                            &format!("mir_closure_capture_load_{}", capture_values.len()),
                        )
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to load MIR closure capture: {}", e))
                        })?;
                    capture_values.push((value, capture_ty));
                }
                crate::mir::MirClosureCaptureKind::ByRef
                | crate::mir::MirClosureCaptureKind::ByMutRef => {
                    capture_values.push((
                        capture_ptr.into(),
                        Type::Reference {
                            mutable: capture.kind == crate::mir::MirClosureCaptureKind::ByMutRef,
                            inner: Box::new(capture_ty),
                        },
                    ));
                }
            }
        }

        let env_ptr = if capture_values.is_empty() {
            self.context.ptr_type(AddressSpace::default()).const_null()
        } else {
            let env_ty = self.context.struct_type(
                &capture_values
                    .iter()
                    .map(|(_, ty)| self.llvm_type(ty))
                    .collect::<Vec<_>>(),
                false,
            );
            let env_size = env_ty
                .size_of()
                .ok_or_else(|| CodegenError::from("Cannot compute MIR closure environment size"))?;
            let malloc_fn = self
                .functions
                .get("malloc")
                .copied()
                .ok_or_else(|| CodegenError::from("malloc not declared"))?;
            let raw_env_ptr = self
                .builder
                .build_call(malloc_fn, &[env_size.into()], "mir_closure_env_alloc")
                .map_err(|e| {
                    CodegenError::from(format!("Failed to call malloc for MIR closure env: {}", e))
                })?
                .try_as_basic_value()
                .left()
                .ok_or_else(|| CodegenError::from("malloc returned void"))?
                .into_pointer_value();
            let env_ptr = self
                .builder
                .build_pointer_cast(
                    raw_env_ptr,
                    self.context.ptr_type(AddressSpace::default()),
                    "mir_closure_env_ptr",
                )
                .map_err(|e| {
                    CodegenError::from(format!("Failed to cast MIR closure env ptr: {}", e))
                })?;

            for (index, (value, ty)) in capture_values.iter().enumerate() {
                let field_ptr = self
                    .builder
                    .build_struct_gep(
                        env_ty,
                        env_ptr,
                        index as u32,
                        &format!("mir_closure_capture_store_{}", index),
                    )
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to access MIR closure env: {}", e))
                    })?;
                let coerced = self.coerce_value(*value, self.llvm_type(ty))?;
                self.builder.build_store(field_ptr, coerced).map_err(|e| {
                    CodegenError::from(format!("Failed to store MIR closure env: {}", e))
                })?;
            }

            raw_env_ptr
        };

        let code_slot = self
            .builder
            .build_alloca(
                self.context.ptr_type(AddressSpace::default()),
                "mir_closure_code_slot",
            )
            .map_err(|e| {
                CodegenError::from(format!("Failed to allocate MIR closure code slot: {}", e))
            })?;
        self.builder
            .build_store(code_slot, closure_fn.as_global_value().as_pointer_value())
            .map_err(|e| {
                CodegenError::from(format!("Failed to store MIR closure code ptr: {}", e))
            })?;
        let code_ptr = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                code_slot,
                "callable_code",
            )
            .map_err(|e| CodegenError::from(format!("Failed to load MIR closure code ptr: {}", e)))?
            .into_pointer_value();

        self.build_callable_value(code_ptr, env_ptr)
    }

    fn validate_mir_closure_value(
        &self,
        context: &MirFunctionContext<'ctx>,
        closure: &MirClosure,
        closure_function_id: &MirFunctionId,
        expected_ty: &Type,
    ) -> Result<(), CodegenError> {
        let Type::Function {
            params: expected_params,
            ret: expected_ret,
            ..
        } = expected_ty
        else {
            return Err(CodegenError::layout(format!(
                "MIR closure destination must be a function type, got {}",
                self.display_type_for_diagnostic(expected_ty)
            )));
        };
        let metadata = self
            .mir_closure_metadata
            .get(closure_function_id)
            .ok_or_else(|| {
                CodegenError::backend_contract(
                    "MIR closure metadata was not recorded before codegen",
                )
            })?;

        let metadata_params = metadata
            .params
            .iter()
            .map(|ty| self.structural_type_for(*ty))
            .collect::<Vec<_>>();
        let metadata_ret = self.structural_type_for(metadata.ret);
        if metadata_params != *expected_params || metadata_ret != **expected_ret {
            let expected_ret_display = self.display_type_for_diagnostic(expected_ret);
            let metadata_ret_display = self.display_type_for_diagnostic(&metadata_ret);
            return Err(CodegenError::layout(format!(
                "MIR closure signature mismatch for '{}': expected ({}) -> {}, target ({}) -> {}",
                closure.display_name,
                expected_params
                    .iter()
                    .map(|ty| self.display_type_for_diagnostic(ty))
                    .collect::<Vec<_>>()
                    .join(", "),
                expected_ret_display,
                metadata_params
                    .iter()
                    .map(|ty| self.display_type_for_diagnostic(ty))
                    .collect::<Vec<_>>()
                    .join(", "),
                metadata_ret_display
            )));
        }

        if closure.captures.len() != metadata.captures.len() {
            return Err(CodegenError::layout(format!(
                "MIR closure capture layout mismatch for '{}': value has {} captures, target has {}",
                closure.display_name,
                closure.captures.len(),
                metadata.captures.len()
            )));
        }

        for (index, (capture, target_ty)) in closure
            .captures
            .iter()
            .zip(metadata.captures.iter())
            .enumerate()
        {
            let place = capture.place();
            let (_, value_ty) = self.compile_mir_place_local(context, &place)?;
            let value_ty = match capture.kind {
                crate::mir::MirClosureCaptureKind::ByValue => value_ty,
                crate::mir::MirClosureCaptureKind::ByRef
                | crate::mir::MirClosureCaptureKind::ByMutRef => Type::Reference {
                    mutable: capture.kind == crate::mir::MirClosureCaptureKind::ByMutRef,
                    inner: Box::new(value_ty),
                },
            };
            let target_ty = self.structural_type_for(*target_ty);
            if value_ty != target_ty {
                return Err(CodegenError::layout(format!(
                    "MIR closure capture layout mismatch for '{}': capture {} has type {}, target expects {}",
                    closure.display_name,
                    index,
                    self.display_type_for_diagnostic(&value_ty),
                    self.display_type_for_diagnostic(&target_ty)
                )));
            }
        }

        Ok(())
    }
}
