use inkwell::basic_block::BasicBlock as LlvmBasicBlock;

mod assert;
mod operand;
mod place;
mod rvalue;
mod terminator;

use inkwell::types::BasicTypeEnum;
use inkwell::values::{FunctionValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

use crate::codegen::{CodeGen, CodegenError, MirClosureCodegenMetadata};
use crate::mir::{
    AggregateKind, Constant, MirCallableKey, MirFunction, MirFunctionId, MirPassMode, MirProgram,
    Operand, Rvalue, StatementData, StatementKind, Terminator,
};

pub(crate) struct MirFunctionContext<'ctx> {
    function: FunctionValue<'ctx>,
    locals: Vec<Option<(PointerValue<'ctx>, crate::ids::TypeId)>>,
    blocks: Vec<LlvmBasicBlock<'ctx>>,
}

pub(crate) fn compile_mir_program(
    codegen: &mut CodeGen<'_>,
    program: &MirProgram,
) -> Result<(), CodegenError> {
    codegen.set_type_context(program.type_context.clone());
    codegen.prepare_mir_program_declarations(program)?;
    codegen.compile_mir_program_bodies(program)
}

impl<'ctx> CodeGen<'ctx> {
    fn compile_mir_program_bodies(&mut self, program: &MirProgram) -> Result<(), CodegenError> {
        self.ensure_mir_type_context(program)
            .map_err(CodegenError::internalize)?;
        self.declare_mir_closure_functions(program)
            .map_err(CodegenError::internalize)?;

        for (_, function) in program.functions() {
            self.compile_mir_function(function)?;
        }

        Ok(())
    }

    fn ensure_mir_type_context(&mut self, program: &MirProgram) -> Result<(), CodegenError> {
        if self.type_context.is_none() {
            self.set_type_context(program.type_context.clone());
            return Ok(());
        }

        for (_, function) in program.functions() {
            self.validate_mir_type_id(program, function.ret_type, "return type")?;
            for local in &function.local_decls {
                self.validate_mir_type_id(program, local.ty, "local type")?;
            }
            for block in &function.basic_blocks {
                for statement in &block.statements {
                    self.validate_mir_statement_type_ids(program, statement)?;
                }
                if let Some(terminator) = &block.terminator {
                    self.validate_mir_terminator_type_ids(program, terminator)?;
                }
            }
        }

        Ok(())
    }

    fn validate_mir_type_id(
        &self,
        program: &MirProgram,
        id: crate::ids::TypeId,
        label: &str,
    ) -> Result<(), CodegenError> {
        let expected = program.type_context.type_for(id);
        if self.type_context().id_for_type(&expected) == Some(id) {
            return Ok(());
        }

        Err(CodegenError::from(format!(
            "MIR {} TypeId {:?} resolves to {} in MIR context but not in codegen context",
            label, id, expected
        )))
    }

    fn validate_mir_statement_type_ids(
        &self,
        program: &MirProgram,
        statement: &StatementData,
    ) -> Result<(), CodegenError> {
        match &statement.kind {
            StatementKind::Assign(_, rvalue) => self.validate_mir_rvalue_type_ids(program, rvalue),
            StatementKind::Assert(assertion) => {
                for operand in &assertion.operands {
                    self.validate_mir_operand_type_ids(program, operand)?;
                }
                Ok(())
            }
            StatementKind::StorageLive(_) | StatementKind::StorageDead(_) => Ok(()),
        }
    }

    fn validate_mir_rvalue_type_ids(
        &self,
        program: &MirProgram,
        rvalue: &Rvalue,
    ) -> Result<(), CodegenError> {
        match rvalue {
            Rvalue::Use(operand) => self.validate_mir_operand_type_ids(program, operand),
            Rvalue::Ref(_, _) => Ok(()),
            Rvalue::Cast(operand, target_ty) => {
                self.validate_mir_operand_type_ids(program, operand)?;
                self.validate_mir_type_id(program, *target_ty, "cast target")
            }
            Rvalue::Closure(_) | Rvalue::Discriminant(_) => Ok(()),
            Rvalue::BinaryOp(_, left, right) => {
                self.validate_mir_operand_type_ids(program, left)?;
                self.validate_mir_operand_type_ids(program, right)
            }
            Rvalue::UnaryOp(_, operand) => self.validate_mir_operand_type_ids(program, operand),
            Rvalue::Aggregate(_, operands) => {
                for operand in operands {
                    self.validate_mir_operand_type_ids(program, operand)?;
                }
                Ok(())
            }
        }
    }

    fn validate_mir_terminator_type_ids(
        &self,
        program: &MirProgram,
        terminator: &Terminator,
    ) -> Result<(), CodegenError> {
        match terminator {
            Terminator::Return
            | Terminator::ReturnWithOrigin { .. }
            | Terminator::Goto(_)
            | Terminator::GotoWithOrigin { .. }
            | Terminator::Drop { .. }
            | Terminator::DropWithOrigin { .. } => Ok(()),
            Terminator::SwitchInt { discr, .. } | Terminator::SwitchIntWithOrigin { discr, .. } => {
                self.validate_mir_operand_type_ids(program, discr)
            }
            Terminator::Call { func, args, .. } => {
                self.validate_mir_operand_type_ids(program, func)?;
                for arg in args {
                    self.validate_mir_operand_type_ids(program, arg)?;
                }
                Ok(())
            }
        }
    }

    fn validate_mir_operand_type_ids(
        &self,
        program: &MirProgram,
        operand: &Operand,
    ) -> Result<(), CodegenError> {
        match operand {
            Operand::Copy(_) | Operand::Move(_) => Ok(()),
            Operand::Constant(Constant::Callable(callable)) => {
                self.validate_mir_callable_type_ids(program, callable)
            }
            Operand::Constant(_) => Ok(()),
        }
    }

    fn validate_mir_callable_type_ids(
        &self,
        _program: &MirProgram,
        _callable: &crate::mir::MirCallable,
    ) -> Result<(), CodegenError> {
        Ok(())
    }

    pub(crate) fn declare_mir_closure_functions(
        &mut self,
        program: &MirProgram,
    ) -> Result<(), CodegenError> {
        for (_, function) in program.functions() {
            let MirFunctionId::Closure(_) = &function.id else {
                continue;
            };
            let closure_key = MirCallableKey::Closure(function.id.clone());
            let captures = function
                .closure_captures
                .iter()
                .map(|capture| {
                    function
                        .local_decls
                        .get(capture.local.0)
                        .map(|local| local.ty)
                        .ok_or_else(|| {
                            CodegenError::internal(format!(
                                "MIR closure '{}' capture local {} is missing",
                                function.name, capture.local.0
                            ))
                        })
                })
                .collect::<Result<Vec<_>, CodegenError>>()?;

            let declaration = program
                .backend_contract
                .callable(&closure_key)
                .ok_or_else(|| {
                    CodegenError::internal(format!(
                        "MIR closure '{}' is missing backend contract callable",
                        function.name
                    ))
                })?;
            let symbol = self
                .callables
                .symbol_for_key(&closure_key)
                .cloned()
                .unwrap_or_else(|| declaration.llvm_symbol.clone());
            if !self.functions.contains_key(&symbol) {
                return Err(CodegenError::internal(format!(
                    "MIR closure contract callable '{}' was not declared before codegen",
                    symbol
                )));
            }
            let params = declaration
                .signature
                .params
                .iter()
                .map(|param| param.semantic_ty)
                .collect::<Vec<_>>();
            let ret_type = declaration.signature.ret.semantic_ty;

            self.callables
                .bind_function_symbol(function.id.clone(), symbol.clone());
            self.mir_closure_metadata.insert(
                function.id.clone(),
                MirClosureCodegenMetadata {
                    params,
                    ret: ret_type,
                    captures,
                },
            );
        }

        Ok(())
    }

    fn compile_mir_function(&mut self, mir_function: &MirFunction) -> Result<(), CodegenError> {
        let symbol = self
            .callables
            .function_symbol(&mir_function.id)
            .cloned()
            .ok_or_else(|| {
                CodegenError::internal(format!(
                    "MIR function '{}' was not declared before codegen",
                    mir_function.name
                ))
            })?;
        let function = *self.functions.get(&symbol).ok_or_else(|| {
            CodegenError::internal(format!(
                "LLVM function '{}' not found for MIR function '{}'",
                symbol, mir_function.name
            ))
        })?;

        self.current_function = Some(function);
        let result = (|| {
            if mir_function.basic_blocks.is_empty() {
                return Err(CodegenError::internal(format!(
                    "MIR function '{}' has no basic blocks",
                    mir_function.name
                )));
            }

            let blocks = mir_function
                .basic_blocks
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    let name = if index == 0 {
                        "entry".to_string()
                    } else {
                        format!("bb{}", index)
                    };
                    self.context.append_basic_block(function, &name)
                })
                .collect::<Vec<_>>();

            self.builder.position_at_end(blocks[0]);
            let mut locals = Vec::with_capacity(mir_function.local_decls.len());
            for (index, local) in mir_function.local_decls.iter().enumerate() {
                let name = Self::mir_local_alloca_name(index, local.name.as_deref());
                let pointer = self
                    .builder
                    .build_alloca(self.llvm_type_id(local.ty)?, &name)
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to create MIR local: {}", e))
                    })?;
                locals.push(Some((pointer, local.ty)));
            }

            let signature = self
                .callables
                .signature_for_function(&mir_function.id)
                .cloned();
            let is_closure = matches!(mir_function.id, MirFunctionId::Closure(_));
            for param_index in 0..mir_function.arg_count {
                let local_index = param_index + 1;
                let Some(Some((pointer, ty))) = locals.get(local_index).cloned() else {
                    return Err(CodegenError::internal(format!(
                        "MIR function '{}' argument local {} is missing",
                        mir_function.name, local_index
                    )));
                };
                let llvm_param_index = if is_closure {
                    param_index + 1
                } else {
                    param_index
                };
                let Some(param) = function.get_nth_param(llvm_param_index as u32) else {
                    return Err(CodegenError::internal(format!(
                        "LLVM function for MIR function '{}' is missing parameter {}",
                        mir_function.name, llvm_param_index
                    )));
                };
                let param_abi = signature
                    .as_ref()
                    .and_then(|signature| signature.params.get(param_index));
                let coerced = if param_abi.is_some_and(|param| param.pass_mode == MirPassMode::Pointer)
                    && !self.type_id_lowers_to_pointer(ty)? {
                    self.builder
                        .build_load(
                            self.llvm_type_id(ty)?,
                            param.into_pointer_value(),
                            "mir_pointer_param",
                        )
                        .map_err(|e| {
                            CodegenError::from(format!(
                                "Failed to load MIR pointer ABI argument: {}",
                                e
                            ))
                        })?
                } else {
                    self.coerce_value(param, self.llvm_type_id(ty)?)?
                };
                self.builder.build_store(pointer, coerced).map_err(|e| {
                    CodegenError::from(format!("Failed to store MIR argument: {}", e))
                })?;
            }

            let context = MirFunctionContext {
                function,
                locals,
                blocks,
            };

            if is_closure {
                self.bind_mir_closure_captures(mir_function, &context)?;
            }

            self.compile_mir_function_body(mir_function, &context)
        })();
        self.current_function = None;
        result?;

        if !function.verify(true) {
            let ir = self.module.print_to_string().to_string();
            return Err(CodegenError::from(format!(
                "MIR function '{}' failed verification.\nIR:\n{}",
                mir_function.name, ir
            )));
        }

        Ok(())
    }

    fn mir_local_alloca_name(index: usize, display_name: Option<&str>) -> String {
        match display_name {
            Some(name) if !name.is_empty() && !name.contains('\0') => name.to_string(),
            _ => format!("local{}", index),
        }
    }

    fn bind_mir_closure_captures(
        &mut self,
        mir_function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
    ) -> Result<(), CodegenError> {
        if mir_function.closure_captures.is_empty() {
            return Ok(());
        }

        let env_param = context
            .function
            .get_nth_param(0)
            .ok_or_else(|| {
                CodegenError::internal(format!(
                    "MIR closure '{}' is missing environment parameter",
                    mir_function.name
                ))
            })?
            .into_pointer_value();
        // Field 0 is the environment drop-function header.
        let mut env_fields = vec![BasicTypeEnum::from(
            self.context.ptr_type(AddressSpace::default()),
        )];
        for capture in &mir_function.closure_captures {
            let Some(Some((_, ty))) = context.locals.get(capture.local.0) else {
                return Err(CodegenError::internal(format!(
                    "MIR closure capture local {} is missing",
                    capture.local.0
                )));
            };
            env_fields.push(self.llvm_type_id(*ty)?);
        }
        let env_ty = self.context.struct_type(&env_fields, false);
        let env_ptr = self
            .builder
            .build_pointer_cast(
                env_param,
                self.context.ptr_type(AddressSpace::default()),
                "mir_closure_env",
            )
            .map_err(|e| CodegenError::from(format!("Failed to cast MIR closure env: {}", e)))?;

        for (index, capture) in mir_function.closure_captures.iter().enumerate() {
            let Some(Some((local_ptr, local_ty))) = context.locals.get(capture.local.0) else {
                return Err(CodegenError::internal(format!(
                    "MIR closure capture local {} is missing",
                    capture.local.0
                )));
            };
            let field_ptr = self
                .builder
                .build_struct_gep(
                    env_ty,
                    env_ptr,
                    index as u32 + 1,
                    &format!("mir_closure_capture_{}", index),
                )
                .map_err(|e| {
                    CodegenError::from(format!("Failed to access MIR closure capture: {}", e))
                })?;
            let value = self
                .builder
                .build_load(
                    self.llvm_type_id(*local_ty)?,
                    field_ptr,
                    "mir_closure_capture",
                )
                .map_err(|e| {
                    CodegenError::from(format!("Failed to load MIR closure capture: {}", e))
                })?;
            self.builder.build_store(*local_ptr, value).map_err(|e| {
                CodegenError::from(format!("Failed to store MIR closure capture: {}", e))
            })?;
        }

        Ok(())
    }

    fn compile_mir_function_body(
        &mut self,
        mir_function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
    ) -> Result<(), CodegenError> {
        for (index, block) in mir_function.basic_blocks.iter().enumerate() {
            self.builder.position_at_end(context.blocks[index]);

            let direct_return_origin = block
                .statements
                .first()
                .and_then(|statement| statement.origin().source_span().cloned());
            if self
                .compile_mir_direct_constant_return(block, context)
                .map_err(|error| error.with_operation_span(direct_return_origin))?
            {
                continue;
            }

            for statement in &block.statements {
                self.compile_mir_statement(mir_function, context, statement)
                    .map_err(|error| {
                        error.with_operation_span(statement.origin().source_span().cloned())
                    })?;
            }

            let terminator = block.terminator.as_ref().ok_or_else(|| {
                CodegenError::from(format!(
                    "MIR basic block {} in '{}' has no terminator",
                    index, mir_function.name
                ))
            })?;
            self.compile_mir_terminator(mir_function, terminator, context)
                .map_err(|error| {
                    error.with_operation_span(terminator.origin().source_span().cloned())
                })?;
        }

        Ok(())
    }

    fn compile_mir_direct_constant_return(
        &mut self,
        block: &crate::mir::BasicBlock,
        context: &MirFunctionContext<'ctx>,
    ) -> Result<bool, CodegenError> {
        if !matches!(
            block.terminator,
            Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
        ) {
            return Ok(false);
        }

        let [statement] = block.statements.as_slice() else {
            return Ok(false);
        };
        let StatementKind::Assign(place, Rvalue::Use(Operand::Constant(constant))) =
            &statement.kind
        else {
            return Ok(false);
        };
        if place.local.0 != 0 || !place.projection.is_empty() {
            return Ok(false);
        }
        if matches!(constant, Constant::Callable(_) | Constant::String(_)) {
            return Ok(false);
        }

        let Some(return_type) = context.function.get_type().get_return_type() else {
            self.builder
                .build_return(None)
                .map_err(|e| CodegenError::from(format!("Failed to build MIR return: {}", e)))?;
            return Ok(true);
        };
        let value = self.compile_mir_constant(constant)?;
        let value = self.coerce_value(value, return_type)?;
        self.builder
            .build_return(Some(&value))
            .map_err(|e| CodegenError::from(format!("Failed to build MIR return: {}", e)))?;
        Ok(true)
    }

    fn compile_mir_statement(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        statement: &StatementData,
    ) -> Result<(), CodegenError> {
        match &statement.kind {
            StatementKind::Assign(place, rvalue) => {
                let destination = self.compile_mir_place_address(context, place)?;
                let (_, ty) = self.compile_mir_place_local_id(context, place)?;
                if self.compile_mir_repeat_array_assignment(
                    function,
                    context,
                    destination,
                    ty,
                    rvalue,
                )? {
                    return Ok(());
                }
                let value = match rvalue {
                    Rvalue::Use(Operand::Constant(Constant::Callable(callable))) => {
                        let structural = self.structural_type_for(ty);
                        self.compile_mir_callable_constant(function, callable, &structural)?
                    }
                    _ => {
                        let structural = self.structural_type_for(ty);
                        self.compile_mir_rvalue_with_expected(
                            function,
                            context,
                            rvalue,
                            &structural,
                        )?
                    }
                };
                let coerced = self.coerce_value(value, self.llvm_type_id(ty)?)?;
                self.builder
                    .build_store(destination, coerced)
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to store MIR assignment: {}", e))
                    })?;

                Ok(())
            }
            StatementKind::Assert(assertion) => {
                self.compile_mir_assert(function, context, assertion)
            }
            StatementKind::StorageLive(_) | StatementKind::StorageDead(_) => Ok(()),
        }
    }

    fn compile_mir_repeat_array_assignment(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        destination: PointerValue<'ctx>,
        destination_ty: crate::ids::TypeId,
        rvalue: &Rvalue,
    ) -> Result<bool, CodegenError> {
        let Rvalue::Aggregate(AggregateKind::Array, operands) = rvalue else {
            return Ok(false);
        };
        let Some(Operand::Copy(source)) = operands.first() else {
            return Ok(false);
        };
        if operands.len() < 2
            || !operands
                .iter()
                .all(|operand| matches!(operand, Operand::Copy(place) if place == source))
        {
            return Ok(false);
        }

        let array_ty = self.structural_type_for(destination_ty);
        let crate::types::Type::Array(element_ty, len) = &array_ty else {
            return Ok(false);
        };
        if len.concrete().ok_or_else(|| CodegenError::non_concrete_array_len(len))? as usize != operands.len() {
            return Ok(false);
        }

        let element_llvm_ty = self.llvm_type(element_ty)?;
        let element = self.compile_mir_operand(function, context, &operands[0])?;
        let element = self.coerce_value(element, element_llvm_ty)?;
        let array_llvm_ty = self.llvm_type(&array_ty)?;
        let index_ty = self.context.i64_type();
        let index = self
            .builder
            .build_alloca(index_ty, "array_repeat_index")
            .map_err(|error| {
                CodegenError::from(format!("Failed to allocate array repeat index: {error}"))
            })?;
        self.builder
            .build_store(index, index_ty.const_zero())
            .map_err(|error| {
                CodegenError::from(format!("Failed to initialize array repeat index: {error}"))
            })?;

        let loop_block = self
            .context
            .append_basic_block(context.function, "array_repeat_loop");
        let done_block = self
            .context
            .append_basic_block(context.function, "array_repeat_done");
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| {
                CodegenError::from(format!("Failed to enter array repeat loop: {error}"))
            })?;

        self.builder.position_at_end(loop_block);
        let current_index = self
            .builder
            .build_load(index_ty, index, "array_repeat_current_index")
            .map_err(|error| {
                CodegenError::from(format!("Failed to load array repeat index: {error}"))
            })?
            .into_int_value();
        let zero = index_ty.const_zero();
        let element_ptr = unsafe {
            self.builder.build_gep(
                array_llvm_ty,
                destination,
                &[zero, current_index],
                "array_repeat_element",
            )
        }
        .map_err(|error| {
            CodegenError::from(format!("Failed to address array repeat element: {error}"))
        })?;
        self.builder
            .build_store(element_ptr, element)
            .map_err(|error| {
                CodegenError::from(format!("Failed to store array repeat element: {error}"))
            })?;

        let next_index = self
            .builder
            .build_int_add(
                current_index,
                index_ty.const_int(1, false),
                "array_repeat_next_index",
            )
            .map_err(|error| {
                CodegenError::from(format!("Failed to advance array repeat index: {error}"))
            })?;
        self.builder
            .build_store(index, next_index)
            .map_err(|error| {
                CodegenError::from(format!("Failed to store array repeat index: {error}"))
            })?;
        let should_continue = self
            .builder
            .build_int_compare(
                IntPredicate::ULT,
                next_index,
                index_ty.const_int(len.concrete().ok_or_else(|| CodegenError::non_concrete_array_len(len))?, false),
                "array_repeat_should_continue",
            )
            .map_err(|error| {
                CodegenError::from(format!("Failed to compare array repeat index: {error}"))
            })?;
        self.builder
            .build_conditional_branch(should_continue, loop_block, done_block)
            .map_err(|error| {
                CodegenError::from(format!("Failed to branch in array repeat loop: {error}"))
            })?;
        self.builder.position_at_end(done_block);

        Ok(true)
    }
}

#[cfg(test)]
mod tests;
