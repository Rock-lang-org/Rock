//! Production MIR runtime helpers shared by MIR-to-LLVM lowering.

use inkwell::module::Linkage;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, PointerValue};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate};

use crate::mir::{
    MirBinOp, MirCallableKind, MirCallableSignature, MirParamAbi, MirPassMode, MirProgram,
    MirUnaryOp,
};
use crate::types::Type;

use super::{CodeGen, CodegenError};

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => unreachable!("hex digit must be in 0..=15"),
    }
}

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn materialize_program_entrypoint(
        &mut self,
        program: &MirProgram,
    ) -> Result<(), CodegenError> {
        let Some(declaration) = program
            .backend_contract
            .callables
            .values()
            .find(|declaration| {
                declaration.llvm_symbol == "main"
                    && matches!(declaration.kind, MirCallableKind::LocalBody { .. })
            })
        else {
            return Ok(());
        };

        if !declaration.signature.params.is_empty() {
            return Err(CodegenError::from(
                "Rock entry function 'main' must not accept parameters",
            ));
        }

        let user_main_symbol = self
            .callable_symbols_by_key
            .get(&declaration.key)
            .ok_or_else(|| CodegenError::from("Rock entry function has no emitted symbol"))?
            .clone();
        let user_main = self
            .module
            .get_function(&user_main_symbol)
            .ok_or_else(|| CodegenError::from("Rock entry function was not declared"))?;

        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        let argc_storage = self
            .module
            .add_global(i64_ty, None, "__rock_runtime_argc_storage");
        argc_storage.set_initializer(&i64_ty.const_zero());
        argc_storage.set_linkage(Linkage::Internal);

        let argv_storage = self
            .module
            .add_global(ptr_ty, None, "__rock_runtime_argv_storage");
        argv_storage.set_initializer(&ptr_ty.const_null());
        argv_storage.set_linkage(Linkage::Internal);

        let arg_count = if let Some(function) = self.module.get_function("__rock_runtime_arg_count")
        {
            function
        } else {
            self.module
                .add_function("__rock_runtime_arg_count", i64_ty.fn_type(&[], false), None)
        };
        let arg_count_entry = self.context.append_basic_block(arg_count, "entry");
        self.builder.position_at_end(arg_count_entry);
        let count = self
            .builder
            .build_load(i64_ty, argc_storage.as_pointer_value(), "program_arg_count")
            .map_err(|error| {
                CodegenError::from(format!("Failed to load program argument count: {error}"))
            })?;
        self.builder.build_return(Some(&count)).map_err(|error| {
            CodegenError::from(format!("Failed to return program argument count: {error}"))
        })?;

        let arg_at = if let Some(function) = self.module.get_function("__rock_runtime_arg_at") {
            function
        } else {
            self.module.add_function(
                "__rock_runtime_arg_at",
                ptr_ty.fn_type(&[i64_ty.into()], false),
                None,
            )
        };
        let arg_at_entry = self.context.append_basic_block(arg_at, "entry");
        let arg_at_valid = self.context.append_basic_block(arg_at, "valid");
        let arg_at_invalid = self.context.append_basic_block(arg_at, "invalid");
        self.builder.position_at_end(arg_at_entry);
        let index = arg_at
            .get_nth_param(0)
            .ok_or_else(|| CodegenError::from("Program argument accessor is missing its index"))?
            .into_int_value();
        let count = self
            .builder
            .build_load(i64_ty, argc_storage.as_pointer_value(), "program_arg_count")
            .map_err(|error| {
                CodegenError::from(format!("Failed to load program argument count: {error}"))
            })?
            .into_int_value();
        let non_negative = self
            .builder
            .build_int_compare(
                IntPredicate::SGE,
                index,
                i64_ty.const_zero(),
                "arg_non_negative",
            )
            .map_err(|error| {
                CodegenError::from(format!("Failed to check program argument index: {error}"))
            })?;
        let below_count = self
            .builder
            .build_int_compare(IntPredicate::SLT, index, count, "arg_below_count")
            .map_err(|error| {
                CodegenError::from(format!("Failed to check program argument index: {error}"))
            })?;
        let valid = self
            .builder
            .build_and(non_negative, below_count, "arg_index_valid")
            .map_err(|error| {
                CodegenError::from(format!("Failed to combine argument bounds checks: {error}"))
            })?;
        self.builder
            .build_conditional_branch(valid, arg_at_valid, arg_at_invalid)
            .map_err(|error| {
                CodegenError::from(format!("Failed to branch on argument bounds: {error}"))
            })?;

        self.builder.position_at_end(arg_at_valid);
        let argv = self
            .builder
            .build_load(ptr_ty, argv_storage.as_pointer_value(), "program_argv")
            .map_err(|error| {
                CodegenError::from(format!("Failed to load program argument vector: {error}"))
            })?
            .into_pointer_value();
        let slot = unsafe {
            self.builder
                .build_gep(ptr_ty, argv, &[index], "program_arg_slot")
        }
        .map_err(|error| {
            CodegenError::from(format!("Failed to address program argument: {error}"))
        })?;
        let argument = self
            .builder
            .build_load(ptr_ty, slot, "program_arg")
            .map_err(|error| {
                CodegenError::from(format!("Failed to load program argument: {error}"))
            })?;
        self.builder
            .build_return(Some(&argument))
            .map_err(|error| {
                CodegenError::from(format!("Failed to return program argument: {error}"))
            })?;

        self.builder.position_at_end(arg_at_invalid);
        let empty = self
            .builder
            .build_global_string_ptr("", "empty_program_arg")
            .map_err(|error| {
                CodegenError::from(format!("Failed to build empty program argument: {error}"))
            })?;
        self.builder
            .build_return(Some(&empty.as_pointer_value()))
            .map_err(|error| {
                CodegenError::from(format!("Failed to return empty program argument: {error}"))
            })?;

        let entrypoint = self.module.add_function(
            "main",
            i32_ty.fn_type(&[i32_ty.into(), ptr_ty.into()], false),
            None,
        );
        let entry = self.context.append_basic_block(entrypoint, "entry");
        self.builder.position_at_end(entry);
        let argc = entrypoint
            .get_nth_param(0)
            .ok_or_else(|| CodegenError::from("Generated entry point is missing argc"))?
            .into_int_value();
        let argv = entrypoint
            .get_nth_param(1)
            .ok_or_else(|| CodegenError::from("Generated entry point is missing argv"))?
            .into_pointer_value();
        let argc = self
            .builder
            .build_int_s_extend(argc, i64_ty, "argc_i64")
            .map_err(|error| {
                CodegenError::from(format!("Failed to widen program argument count: {error}"))
            })?;
        self.builder
            .build_store(argc_storage.as_pointer_value(), argc)
            .map_err(|error| {
                CodegenError::from(format!("Failed to store program argument count: {error}"))
            })?;
        self.builder
            .build_store(argv_storage.as_pointer_value(), argv)
            .map_err(|error| {
                CodegenError::from(format!("Failed to store program argument vector: {error}"))
            })?;

        let call = self
            .builder
            .build_call(user_main, &[], "rock_main")
            .map_err(|error| {
                CodegenError::from(format!("Failed to call Rock entry function: {error}"))
            })?;
        let exit_code = match call.try_as_basic_value().left() {
            None => i32_ty.const_zero(),
            Some(BasicValueEnum::IntValue(value)) => self
                .coerce_value(value.into(), i32_ty.into())?
                .into_int_value(),
            Some(_) => {
                return Err(CodegenError::from(
                    "Rock entry function 'main' must return an integer or ()",
                ));
            }
        };
        self.builder
            .build_return(Some(&exit_code))
            .map_err(|error| {
                CodegenError::from(format!(
                    "Failed to return from generated entry point: {error}"
                ))
            })?;

        Ok(())
    }

    pub(crate) fn mangle_function_symbol(name: &str) -> String {
        if name == "main" {
            return name.to_string();
        }

        let mut mangled = String::with_capacity(name.len() * 2);
        for (index, segment) in name.split("::").enumerate() {
            if index > 0 {
                mangled.push('_');
            }
            mangled.push('s');
            mangled.push_str(&segment.len().to_string());
            mangled.push('_');
            for byte in segment.as_bytes() {
                mangled.push(hex_digit(byte >> 4));
                mangled.push(hex_digit(byte & 0x0f));
            }
        }
        mangled
    }

    pub(crate) fn unique_rock_function_symbol_name(&self, desired: &str) -> String {
        if self.module.get_function(desired).is_none() {
            return desired.to_string();
        }

        let mut index = 0;
        loop {
            let candidate = if index == 0 {
                format!("__rock_{}", desired)
            } else {
                format!("__rock_{}_{}", desired, index)
            };
            if self.module.get_function(&candidate).is_none() {
                return candidate;
            }
            index += 1;
        }
    }

    pub(crate) fn sanitize_symbol(name: &str) -> String {
        name.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    }

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

    fn compile_str_comparison(
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
