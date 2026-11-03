use inkwell::module::Linkage;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{MirCallableKind, MirProgram};

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
}
