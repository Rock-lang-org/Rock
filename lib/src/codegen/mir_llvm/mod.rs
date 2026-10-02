use inkwell::basic_block::BasicBlock as LlvmBasicBlock;

mod assert;
mod object;
mod object_adapters;
mod erased;
mod operand;
mod place;
mod rvalue;
mod terminator;

use inkwell::values::{FunctionValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

use crate::codegen::{CodeGen, CodegenError, MirClosureCodegenMetadata};
use crate::mir::{
    AggregateKind, Constant, MirCallableKey, MirClosureId, MirFunction, MirFunctionId, MirPassMode,
    MirProgram, Operand, Rvalue, StatementData, StatementKind, Terminator,
};

#[allow(dead_code)]
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

#[allow(dead_code)]
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
            Rvalue::Object(operand, _) | Rvalue::Use(operand) => {
                self.validate_mir_operand_type_ids(program, operand)
            }
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
            | Terminator::Unreachable { .. }
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
                .callable_symbols_by_key
                .get(&closure_key)
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

            self.mir_function_symbols
                .insert(function.id.clone(), symbol.clone());
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

    fn mir_closure_symbol(parent_symbol: &str, closure_id: &MirClosureId) -> String {
        format!(
            "__mir_closure_{}_{}_{}",
            Self::sanitize_symbol(parent_symbol),
            Self::mir_function_id_symbol_key(&closure_id.owner),
            closure_id.local_index
        )
    }

    fn mir_function_id_symbol_key(id: &MirFunctionId) -> String {
        match id {
            MirFunctionId::Function(id) => format!("fn_{}_{}", id.crate_id.0, id.local.0),
            MirFunctionId::Extern(id) => format!("extern_{}_{}", id.crate_id.0, id.local.0),
            MirFunctionId::Instance(id) => format!("instance_{}", id.0),
            MirFunctionId::Closure(id) => format!(
                "closure_{}_{}",
                Self::mir_function_id_symbol_key(&id.owner),
                id.local_index
            ),
        }
    }

    fn compile_mir_function(&mut self, mir_function: &MirFunction) -> Result<(), CodegenError> {
        let symbol = self
            .mir_function_symbols
            .get(&mir_function.id)
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
                    .build_alloca(self.llvm_type_id(local.ty), &name)
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to create MIR local: {}", e))
                    })?;
                locals.push(Some((pointer, local.ty)));
            }

            let signature = self
                .mir_function_symbols
                .get(&mir_function.id)
                .and_then(|symbol| self.callable_signatures_by_symbol.get(symbol))
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
                let coerced = if param_abi.is_some_and(|param| {
                    param.pass_mode == MirPassMode::Pointer && !self.type_id_lowers_to_pointer(ty)
                }) {
                    self.builder
                        .build_load(
                            self.llvm_type_id(ty),
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
                    self.coerce_value(param, self.llvm_type_id(ty))?
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
        let env_ty = self.context.struct_type(
            &mir_function
                .closure_captures
                .iter()
                .map(|capture| {
                    let Some(Some((_, ty))) = context.locals.get(capture.local.0) else {
                        return Err(CodegenError::internal(format!(
                            "MIR closure capture local {} is missing",
                            capture.local.0
                        )));
                    };
                    Ok(self.llvm_type_id(*ty))
                })
                .collect::<Result<Vec<_>, CodegenError>>()?,
            false,
        );
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
                    index as u32,
                    &format!("mir_closure_capture_{}", index),
                )
                .map_err(|e| {
                    CodegenError::from(format!("Failed to access MIR closure capture: {}", e))
                })?;
            let value = self
                .builder
                .build_load(
                    self.llvm_type_id(*local_ty),
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
        if context
            .locals
            .first()
            .and_then(|local| local.as_ref())
            .is_some_and(|(_, ty)| {
                matches!(self.structural_type_for(*ty), crate::types::Type::Never)
            })
        {
            return Ok(false);
        }
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
                let coerced = self.coerce_value(value, self.llvm_type_id(ty))?;
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
        if *len != operands.len() {
            return Ok(false);
        }

        let element_llvm_ty = self.llvm_type(element_ty);
        let element = self.compile_mir_operand(function, context, &operands[0])?;
        let element = self.coerce_value(element, element_llvm_ty)?;
        let array_llvm_ty = self.llvm_type(&array_ty);
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
                index_ty.const_int(*len as u64, false),
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
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use inkwell::context::Context;

    use crate::ids::{AssocTypeId, CrateId, DefId, InstanceId, LocalDefId, TypeId, VariantId};
    use crate::mir::{
        AggregateKind, BasicBlock, Constant, Local, LocalDecl, MirAssert, MirAssertKind,
        MirBackendContract, MirBinOp, MirCallable, MirCallableDecl, MirCallableKey,
        MirCallableKind, MirCallableSignature, MirClosure, MirClosureCapture,
        MirClosureCaptureKind, MirClosureId, MirEnumVariantLayout, MirFunction, MirFunctionId,
        MirLinkage, MirNominalLayout, MirPassMode, MirProgram, MirProjectionKey, MirRuntimeHelper,
        MirUnaryOp, MirVariantLayoutFields, Mutability, Operand, Place, Projection, Rvalue,
        StatementData, Terminator,
    };
    use crate::type_context::TypeContext;
    use crate::types::{AssociatedTypeKey, Type};

    use super::super::{CodeGen, CodegenError};

    fn root_place(local: usize) -> Place {
        Place {
            local: Local(local),
            projection: vec![],
        }
    }

    fn type_id(type_context: &RefCell<TypeContext>, ty: Type) -> crate::ids::TypeId {
        type_context.borrow_mut().intern_type(&ty)
    }

    fn drop_glue_contract(
        dropped_ty: crate::ids::TypeId,
        key: MirCallableKey,
        source_def_id: DefId,
        symbol: &str,
        unit_ty: crate::ids::TypeId,
    ) -> MirBackendContract {
        let mut contract = MirBackendContract::default();
        contract.callables.insert(
            key.clone(),
            MirCallableDecl {
                key: key.clone(),
                source_def_id: Some(source_def_id),
                kind: MirCallableKind::ObjectProvided,
                llvm_symbol: symbol.to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(
                    &[dropped_ty],
                    unit_ty,
                    MirPassMode::Direct,
                ),
            },
        );
        contract.drop_glue.insert(dropped_ty, key);
        contract
    }

    fn undeclarable_drop_glue_contract(
        dropped_ty: TypeId,
        key: MirCallableKey,
        symbol: &str,
        unit_ty: TypeId,
    ) -> MirBackendContract {
        let mut contract = MirBackendContract::default();
        contract.callables.insert(
            key.clone(),
            MirCallableDecl {
                key: key.clone(),
                source_def_id: None,
                kind: MirCallableKind::RuntimeHelper(MirRuntimeHelper::DropGlue),
                llvm_symbol: symbol.to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(
                    &[dropped_ty],
                    unit_ty,
                    MirPassMode::Direct,
                ),
            },
        );
        contract.drop_glue.insert(dropped_ty, key);
        contract
    }

    fn local_body_callable_key(function_id: &MirFunctionId) -> MirCallableKey {
        match function_id {
            MirFunctionId::Function(id) => MirCallableKey::Function(*id),
            MirFunctionId::Extern(id) => MirCallableKey::Extern(*id),
            MirFunctionId::Instance(id) => MirCallableKey::Instance(*id),
            MirFunctionId::Closure(id) => {
                MirCallableKey::Closure(MirFunctionId::Closure(id.clone()))
            }
        }
    }

    fn insert_local_body_contract(
        contract: &mut MirBackendContract,
        function_id: MirFunctionId,
        symbol: &str,
        params: &[TypeId],
        ret_ty: TypeId,
    ) {
        insert_local_body_contract_with_signature(
            contract,
            function_id,
            symbol,
            MirCallableSignature::from_type_ids(params, ret_ty, MirPassMode::Direct),
        );
    }

    fn insert_local_body_contract_with_signature(
        contract: &mut MirBackendContract,
        function_id: MirFunctionId,
        symbol: &str,
        signature: MirCallableSignature,
    ) {
        let key = local_body_callable_key(&function_id);
        contract.callables.insert(
            key.clone(),
            MirCallableDecl {
                key: key.clone(),
                source_def_id: None,
                kind: MirCallableKind::LocalBody {
                    function_id: function_id.clone(),
                },
                llvm_symbol: symbol.to_string(),
                linkage: MirLinkage::External,
                signature,
            },
        );
        contract.function_bodies.insert(function_id, key);
    }

    fn local_body_contract_with_params(
        function_id: MirFunctionId,
        symbol: &str,
        params: &[TypeId],
        ret_ty: TypeId,
    ) -> MirBackendContract {
        let mut contract = MirBackendContract::default();
        insert_local_body_contract(&mut contract, function_id, symbol, params, ret_ty);
        contract
    }

    fn local_body_contract(
        function_id: MirFunctionId,
        symbol: &str,
        ret_ty: TypeId,
    ) -> MirBackendContract {
        local_body_contract_with_params(function_id, symbol, &[], ret_ty)
    }

    fn local_body_contract_with_return_abi(
        function_id: MirFunctionId,
        symbol: &str,
        ret_ty: TypeId,
        ret_abi_ty: TypeId,
    ) -> MirBackendContract {
        let mut signature = MirCallableSignature::from_type_ids(&[], ret_ty, MirPassMode::Direct);
        signature.ret.abi_ty = ret_abi_ty;
        let mut contract = MirBackendContract::default();
        insert_local_body_contract_with_signature(&mut contract, function_id, symbol, signature);
        contract
    }

    fn insert_struct_layout_contract(
        contract: &mut MirBackendContract,
        id: DefId,
        fields: Vec<(String, TypeId)>,
    ) {
        contract.nominal_layouts.insert(
            id,
            MirNominalLayout::Struct {
                id,
                fields,
                generic_params: Vec::new(),
            },
        );
    }

    fn insert_enum_layout_contract(
        contract: &mut MirBackendContract,
        id: DefId,
        variants: Vec<MirEnumVariantLayout>,
    ) {
        contract.nominal_layouts.insert(
            id,
            MirNominalLayout::Enum {
                id,
                variants,
                generic_params: Vec::new(),
            },
        );
    }

    fn insert_object_callable_contract(
        contract: &mut MirBackendContract,
        key: MirCallableKey,
        symbol: &str,
        params: &[TypeId],
        ret_ty: TypeId,
    ) {
        insert_object_callable_contract_with_signature(
            contract,
            key,
            symbol,
            MirCallableSignature::from_type_ids(params, ret_ty, MirPassMode::Direct),
        );
    }

    fn insert_object_callable_contract_with_signature(
        contract: &mut MirBackendContract,
        key: MirCallableKey,
        symbol: &str,
        signature: MirCallableSignature,
    ) {
        contract.callables.insert(
            key.clone(),
            MirCallableDecl {
                key,
                source_def_id: None,
                kind: MirCallableKind::ObjectProvided,
                llvm_symbol: symbol.to_string(),
                linkage: MirLinkage::External,
                signature,
            },
        );
    }
    fn projection_type_with_output(
        type_context: &RefCell<TypeContext>,
        base_ty: Type,
        trait_id: DefId,
        assoc_type_id: AssocTypeId,
        trait_args: Vec<Type>,
        output_ty: Type,
    ) -> (Type, MirProjectionKey, crate::ids::TypeId) {
        let base = type_id(type_context, base_ty.clone());
        let trait_arg_ids = trait_args
            .iter()
            .cloned()
            .map(|arg| type_id(type_context, arg))
            .collect();
        let output = type_id(type_context, output_ty);
        let key = MirProjectionKey {
            base,
            trait_id,
            assoc_type_id,
            trait_args: trait_arg_ids,
        };
        let projection = Type::Projection {
            ty: Box::new(base_ty),
            trait_id,
            assoc_type: AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args,
        };

        (projection, key, output)
    }

    fn assert_valid_mir_fixture(program: &MirProgram) {
        let report = crate::mir::agreement::check_mir_runtime_agreement(program);
        assert!(
            report.is_clean(),
            "invalid MIR program fixture: {report:#?}"
        );
    }

    fn compile_valid_mir_program(
        codegen: &mut CodeGen<'_>,
        program: &MirProgram,
    ) -> Result<(), CodegenError> {
        assert_valid_mir_fixture(program);
        codegen.compile_program_from_mir(program)
    }

    struct MirProgramFixtureBuilder {
        functions: BTreeMap<MirFunctionId, MirFunction>,
        type_context: TypeContext,
        backend_contract: MirBackendContract,
    }

    impl MirProgramFixtureBuilder {
        fn new(type_context: TypeContext, backend_contract: MirBackendContract) -> Self {
            Self {
                functions: BTreeMap::new(),
                type_context,
                backend_contract,
            }
        }

        fn with_local_body(
            mut self,
            function: MirFunction,
            symbol: &str,
            params: &[TypeId],
        ) -> Self {
            let function_id = function.id.clone();
            let callable_key = local_body_callable_key(&function_id);
            assert!(
                !self.backend_contract.callables.contains_key(&callable_key)
                    && !self
                        .backend_contract
                        .function_bodies
                        .contains_key(&function_id),
                "duplicate MIR local-body contract fixture"
            );
            insert_local_body_contract(
                &mut self.backend_contract,
                function_id.clone(),
                symbol,
                params,
                function.ret_type,
            );
            assert!(
                self.functions.insert(function_id, function).is_none(),
                "duplicate MIR function fixture"
            );
            self
        }

        fn build(self) -> MirProgram {
            let program = MirProgram {
                functions: self.functions,
                type_context: self.type_context,
                backend_contract: self.backend_contract,
            };
            assert_valid_mir_fixture(&program);
            program
        }
    }

    fn compile_test_function(
        codegen: &mut CodeGen<'_>,
        type_context: &RefCell<TypeContext>,
        name: &str,
        ret_ty: Type,
        params: Vec<Type>,
        local_decls: Vec<LocalDecl>,
        statements: Vec<StatementData>,
    ) {
        compile_test_function_result(
            codegen,
            type_context,
            name,
            ret_ty,
            params,
            local_decls,
            statements,
        )
        .unwrap();
    }

    fn compile_test_function_result(
        codegen: &mut CodeGen<'_>,
        type_context: &RefCell<TypeContext>,
        name: &str,
        ret_ty: Type,
        params: Vec<Type>,
        local_decls: Vec<LocalDecl>,
        statements: Vec<StatementData>,
    ) -> Result<(), CodegenError> {
        compile_test_function_result_with_contract(
            codegen,
            type_context,
            name,
            ret_ty,
            params,
            local_decls,
            statements,
            MirBackendContract::default(),
        )
    }

    fn compile_test_function_with_contract(
        codegen: &mut CodeGen<'_>,
        type_context: &RefCell<TypeContext>,
        name: &str,
        ret_ty: Type,
        params: Vec<Type>,
        local_decls: Vec<LocalDecl>,
        statements: Vec<StatementData>,
        backend_contract: MirBackendContract,
    ) {
        compile_test_function_result_with_contract(
            codegen,
            type_context,
            name,
            ret_ty,
            params,
            local_decls,
            statements,
            backend_contract,
        )
        .unwrap();
    }

    fn compile_test_function_result_with_contract(
        codegen: &mut CodeGen<'_>,
        type_context: &RefCell<TypeContext>,
        name: &str,
        ret_ty: Type,
        params: Vec<Type>,
        local_decls: Vec<LocalDecl>,
        statements: Vec<StatementData>,
        backend_contract: MirBackendContract,
    ) -> Result<(), CodegenError> {
        let function_id = MirFunctionId::Instance(InstanceId(0));
        let param_tys = params
            .iter()
            .cloned()
            .map(|param| type_id(type_context, param))
            .collect::<Vec<_>>();
        let ret_type = type_id(type_context, ret_ty.clone());
        let function = MirFunction {
            id: function_id.clone(),
            name: name.to_string(),
            basic_blocks: vec![BasicBlock {
                statements,
                terminator: Some(Terminator::Return),
            }],
            local_decls,
            closure_captures: Vec::new(),
            arg_count: params.len(),
            ret_type,
            ownership: Default::default(),
        };
        let mir = MirProgramFixtureBuilder::new(type_context.borrow().clone(), backend_contract)
            .with_local_body(function, name, &param_tys)
            .build();

        codegen.compile_program_from_mir(&mir)
    }

    fn local_decl(type_context: &RefCell<TypeContext>, ty: Type, name: &str) -> LocalDecl {
        LocalDecl {
            ty: type_id(type_context, ty),
            mutability: Mutability::Mut,
            name: Some(name.to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }
    }

    fn bounds_check_contract() -> MirBackendContract {
        let mut contract = MirBackendContract::default();
        contract
            .runtime_requirements
            .insert(crate::mir::MirRuntimeHelper::BoundsCheck);
        contract
    }

    fn assert_ir_order(ir: &str, snippets: &[&str]) {
        let mut start = 0;
        for snippet in snippets {
            let found = ir[start..].find(snippet).unwrap_or_else(|| {
                let remaining: String = ir[start..].chars().take(1200).collect();
                panic!(
                    "expected IR to contain {:?} after byte {}\nremaining IR:\n{}",
                    snippet, start, remaining
                )
            });
            start += found + snippet.len();
        }
    }

    fn assert_ir_order_ignoring_aggregate_gep_index_widths(ir: &str, snippets: &[&str]) {
        // LLVM may print constant aggregate GEP indices as target pointer-sized.
        let normalized_ir = ir
            .lines()
            .map(|line| {
                if line.contains("getelementptr inbounds {") {
                    normalize_i64_constant_gep_indices(line)
                } else if line.contains("getelementptr inbounds nuw {") {
                    normalize_i64_constant_gep_indices(
                        &line.replace("getelementptr inbounds nuw {", "getelementptr inbounds {"),
                    )
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_ir_order(&normalized_ir, snippets);
    }

    fn normalize_i64_constant_gep_indices(line: &str) -> String {
        let mut normalized = String::with_capacity(line.len());
        let mut rest = line;

        while let Some(index) = rest.find(", i64 ") {
            normalized.push_str(&rest[..index]);
            let after_prefix = &rest[index + ", i64 ".len()..];
            let digits_len: usize = after_prefix
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .map(char::len_utf8)
                .sum();

            if digits_len == 0 {
                normalized.push_str(", i64 ");
            } else {
                normalized.push_str(", i32 ");
                normalized.push_str(&after_prefix[..digits_len]);
            }
            rest = &after_prefix[digits_len..];
        }

        normalized.push_str(rest);
        normalized
    }

    #[test]
    fn assert_ir_order_ignores_target_dependent_aggregate_gep_index_widths() {
        let ir = "%field = getelementptr inbounds nuw { i64, i64 }, ptr %value, i64 0, i64 1\n\
                  %loaded = load i64, ptr %field";

        assert_ir_order_ignoring_aggregate_gep_index_widths(
            ir,
            &[
                "%field = getelementptr inbounds { i64, i64 }, ptr %value, i32 0, i32 1",
                "%loaded = load i64, ptr %field",
            ],
        );
    }

    #[test]
    fn mir_reference_promotion_ignores_local_display_name_presence() {
        let named_ir = reference_promotion_ir(Some("scalar"), crate::mir::LocalSource::UserBinding);
        let unnamed_ir = reference_promotion_ir(None, crate::mir::LocalSource::UserBinding);

        assert!(!named_ir.contains("mir_ref_tmp_alloc"));
        assert!(!unnamed_ir.contains("mir_ref_tmp_alloc"));
    }

    #[test]
    fn mir_ref_to_temporary_does_not_emit_heap_promotion() {
        let ir = reference_promotion_ir(Some("scalar"), crate::mir::LocalSource::Temporary);

        assert!(
            !ir.contains("mir_ref_tmp_alloc"),
            "reference to MIR temporary should use the place address directly:\n{ir}"
        );
    }

    fn reference_promotion_ir(
        source_name: Option<&str>,
        source: crate::mir::LocalSource,
    ) -> String {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "reference_promotion");
        let type_context = RefCell::new(TypeContext::new());
        let scalar_ref_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };

        compile_test_function(
            &mut codegen,
            &type_context,
            "reference_promotion",
            Type::Unit,
            vec![],
            vec![
                local_decl(&type_context, Type::Unit, "return_place"),
                local_decl_with_name_and_source(&type_context, Type::I64, source_name, source),
                local_decl(&type_context, scalar_ref_ty.clone(), "scalar_ref"),
            ],
            vec![
                StatementData::assign(
                    root_place(1),
                    Rvalue::Use(Operand::Constant(Constant::Int(7))),
                    None,
                ),
                StatementData::assign(
                    root_place(2),
                    Rvalue::Ref(Mutability::Not, root_place(1)),
                    None,
                ),
            ],
        );

        codegen.get_ir()
    }

    fn local_decl_with_name_and_source(
        type_context: &RefCell<TypeContext>,
        ty: Type,
        name: Option<&str>,
        source: crate::mir::LocalSource,
    ) -> LocalDecl {
        LocalDecl {
            ty: type_id(type_context, ty),
            mutability: Mutability::Mut,
            name: name.map(str::to_string),
            span: None,
            source,
        }
    }

    fn field_place(local: usize, index: usize) -> Place {
        Place {
            local: Local(local),
            projection: vec![Projection::Field {
                index,
                identity: None,
            }],
        }
    }

    fn index_place(local: usize, index_local: usize) -> Place {
        Place {
            local: Local(local),
            projection: vec![Projection::Index(Local(index_local))],
        }
    }

    fn downcast_field_place(local: usize, variant_id: VariantId, index: usize) -> Place {
        Place {
            local: Local(local),
            projection: vec![
                Projection::Downcast(variant_id),
                Projection::Field {
                    index,
                    identity: None,
                },
            ],
        }
    }

    #[test]
    fn mir_closure_symbols_include_canonical_owner_identity() {
        let left = MirClosureId {
            owner: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(50))),
            local_index: 0,
        };
        let right = MirClosureId {
            owner: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(51))),
            local_index: 0,
        };

        let left_symbol = CodeGen::mir_closure_symbol("a.b", &left);
        let right_symbol = CodeGen::mir_closure_symbol("a_b", &right);

        assert_ne!(left_symbol, right_symbol);
    }

    #[test]
    fn mir_codegen_lowers_closure_values_from_mir_bodies() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let parent_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(40)));
        let callable_ty = Type::function(vec![Type::I64], Type::I64);
        let callable_type_id = type_id(&type_context, callable_ty.clone());
        let i64_type = type_id(&type_context, Type::I64);
        let closure_id = MirClosureId {
            owner: parent_id.clone(),
            local_index: 0,
        };
        let closure_function_id = MirFunctionId::Closure(Box::new(closure_id.clone()));
        let closure_symbol = CodeGen::mir_closure_symbol("make_closure", &closure_id);
        let parent_capture = MirClosureCapture {
            name: "captured".to_string(),
            local: Local(1),
            kind: MirClosureCaptureKind::ByValue,
            span: None,
        };
        let closure_capture = MirClosureCapture {
            name: "captured".to_string(),
            local: Local(2),
            kind: MirClosureCaptureKind::ByValue,
            span: None,
        };

        let parent = MirFunction {
            id: parent_id.clone(),
            name: "make_closure".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::assign(
                        root_place(1),
                        Rvalue::Use(Operand::Constant(Constant::Int(41))),
                        None,
                    ),
                    StatementData::assign(
                        root_place(0),
                        Rvalue::Closure(MirClosure {
                            id: closure_id.clone(),
                            display_name: "make_closure.closure0".to_string(),
                            captures: vec![parent_capture],
                        }),
                        None,
                    ),
                ],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                local_decl(&type_context, callable_ty.clone(), "return_place"),
                local_decl(&type_context, Type::I64, "captured"),
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: callable_type_id,
            ownership: Default::default(),
        };
        let closure = MirFunction {
            id: closure_function_id.clone(),
            name: "make_closure.closure0".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Copy(root_place(2))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, Type::I64, "arg"),
                local_decl(&type_context, Type::I64, "captured"),
            ],
            closure_captures: vec![closure_capture],
            arg_count: 1,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let mut backend_contract =
            local_body_contract(parent_id.clone(), "make_closure", callable_type_id);
        insert_local_body_contract(
            &mut backend_contract,
            closure_function_id.clone(),
            &closure_symbol,
            &[i64_type],
            i64_type,
        );
        backend_contract
            .runtime_requirements
            .insert(crate::mir::MirRuntimeHelper::HeapAlloc);
        let mir = MirProgram {
            functions: BTreeMap::from([
                (parent_id.clone(), parent),
                (closure_function_id, closure),
            ]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("__mir_closure"));
        assert!(ir.contains("callable_code"));
        assert!(ir.contains("callable_env"));
        assert!(ir.contains("mir_closure_env_alloc"));
        assert!(ir.contains("mir_closure_capture"));
        assert!(!ir.contains("__lambda_"));
    }

    #[test]
    fn mir_codegen_accepts_resolved_closure_callable_constants() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let parent_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(41)));
        let callable_ty = Type::function(vec![], Type::I64);
        let callable_type_id = type_id(&type_context, callable_ty.clone());
        let i64_type = type_id(&type_context, Type::I64);
        let closure_id = MirClosureId {
            owner: parent_id.clone(),
            local_index: 0,
        };
        let closure_function_id = MirFunctionId::Closure(Box::new(closure_id.clone()));
        let closure_symbol = CodeGen::mir_closure_symbol("bad_closure_constant", &closure_id);
        let parent = MirFunction {
            id: parent_id.clone(),
            name: "bad_closure_constant".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(Constant::Callable(
                        MirCallable::Resolved(MirCallableKey::Closure(closure_function_id.clone())),
                    ))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![local_decl(
                &type_context,
                callable_ty.clone(),
                "return_place",
            )],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: callable_type_id,
            ownership: Default::default(),
        };
        let closure = MirFunction {
            id: closure_function_id.clone(),
            name: "bad_closure_constant.closure0".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(Constant::Int(7))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![local_decl(&type_context, Type::I64, "return_place")],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let mut backend_contract =
            local_body_contract(parent_id.clone(), "bad_closure_constant", callable_type_id);
        insert_local_body_contract(
            &mut backend_contract,
            closure_function_id.clone(),
            &closure_symbol,
            &[],
            i64_type,
        );
        let mir = MirProgram {
            functions: BTreeMap::from([
                (parent_id.clone(), parent),
                (closure_function_id, closure),
            ]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        codegen.compile_program_from_mir(&mir).unwrap();
    }

    #[test]
    fn mir_codegen_rejects_closure_signature_mismatch() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let parent_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(42)));
        let expected_callable_ty = Type::function(vec![Type::I32], Type::I64);
        let expected_callable_type_id = type_id(&type_context, expected_callable_ty.clone());
        let i64_type = type_id(&type_context, Type::I64);
        let closure_id = MirClosureId {
            owner: parent_id.clone(),
            local_index: 0,
        };
        let closure_function_id = MirFunctionId::Closure(Box::new(closure_id.clone()));
        let closure_symbol = CodeGen::mir_closure_symbol("bad_closure_signature", &closure_id);
        let parent = MirFunction {
            id: parent_id.clone(),
            name: "bad_closure_signature".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Closure(MirClosure {
                        id: closure_id,
                        display_name: "bad_closure_signature.closure0".to_string(),
                        captures: Vec::new(),
                    }),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![local_decl(
                &type_context,
                expected_callable_ty.clone(),
                "return_place",
            )],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: expected_callable_type_id,
            ownership: Default::default(),
        };
        let closure = MirFunction {
            id: closure_function_id.clone(),
            name: "bad_closure_signature.closure0".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(Constant::Int(7))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, Type::I64, "arg"),
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let mut backend_contract = local_body_contract(
            parent_id.clone(),
            "bad_closure_signature",
            expected_callable_type_id,
        );
        insert_local_body_contract(
            &mut backend_contract,
            closure_function_id.clone(),
            &closure_symbol,
            &[i64_type],
            i64_type,
        );
        let mir = MirProgram {
            functions: BTreeMap::from([
                (parent_id.clone(), parent),
                (closure_function_id, closure),
            ]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        let error = codegen.compile_program_from_mir(&mir).unwrap_err();
        assert!(error.message.contains("signature"));
        assert!(!codegen.get_ir().contains("callable_code"));
    }

    #[test]
    fn mir_codegen_rejects_closure_capture_layout_mismatch() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let parent_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(43)));
        let callable_ty = Type::function(vec![], Type::I64);
        let callable_type_id = type_id(&type_context, callable_ty.clone());
        let i64_type = type_id(&type_context, Type::I64);
        let closure_id = MirClosureId {
            owner: parent_id.clone(),
            local_index: 0,
        };
        let closure_function_id = MirFunctionId::Closure(Box::new(closure_id.clone()));
        let closure_symbol = CodeGen::mir_closure_symbol("bad_closure_capture_layout", &closure_id);
        let parent_capture = MirClosureCapture {
            name: "captured".to_string(),
            local: Local(1),
            kind: MirClosureCaptureKind::ByValue,
            span: None,
        };
        let closure_capture = MirClosureCapture {
            name: "captured".to_string(),
            local: Local(1),
            kind: MirClosureCaptureKind::ByValue,
            span: None,
        };
        let parent = MirFunction {
            id: parent_id.clone(),
            name: "bad_closure_capture_layout".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::assign(
                        root_place(1),
                        Rvalue::Use(Operand::Constant(Constant::Int(11))),
                        None,
                    ),
                    StatementData::assign(
                        root_place(0),
                        Rvalue::Closure(MirClosure {
                            id: closure_id,
                            display_name: "bad_closure_capture_layout.closure0".to_string(),
                            captures: vec![parent_capture],
                        }),
                        None,
                    ),
                ],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                local_decl(&type_context, callable_ty.clone(), "return_place"),
                local_decl(&type_context, Type::I64, "captured"),
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: callable_type_id,
            ownership: Default::default(),
        };
        let closure = MirFunction {
            id: closure_function_id.clone(),
            name: "bad_closure_capture_layout.closure0".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(Constant::Int(7))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, Type::I32, "captured"),
            ],
            closure_captures: vec![closure_capture],
            arg_count: 0,
            ret_type: i64_type,
            ownership: Default::default(),
        };
        let mut backend_contract = local_body_contract(
            parent_id.clone(),
            "bad_closure_capture_layout",
            callable_type_id,
        );
        insert_local_body_contract(
            &mut backend_contract,
            closure_function_id.clone(),
            &closure_symbol,
            &[],
            i64_type,
        );
        backend_contract
            .runtime_requirements
            .insert(MirRuntimeHelper::HeapAlloc);
        let mir = MirProgram {
            functions: BTreeMap::from([
                (parent_id.clone(), parent),
                (closure_function_id, closure),
            ]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        let error = codegen.compile_program_from_mir(&mir).unwrap_err();
        assert!(error.message.contains("capture layout"));
        assert!(!codegen.get_ir().contains("mir_closure_env_alloc"));
    }

    #[test]
    fn mir_codegen_reports_bad_aggregate_operand_counts() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        let too_few_tuple = compile_test_function_result(
            &mut codegen,
            &type_context,
            "bad_tuple_aggregate",
            Type::Tuple(vec![Type::I64, Type::I64]),
            vec![],
            vec![local_decl(
                &type_context,
                Type::Tuple(vec![Type::I64, Type::I64]),
                "return_place",
            )],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Aggregate(
                    AggregateKind::Tuple,
                    vec![Operand::Constant(Constant::Int(1))],
                ),
                None,
            )],
        );
        assert!(too_few_tuple.is_err());
        assert!(too_few_tuple.unwrap_err().message.contains("operand count"));

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let too_many_array = catch_unwind(AssertUnwindSafe(|| {
            compile_test_function_result(
                &mut codegen,
                &type_context,
                "bad_array_aggregate",
                Type::Array(Box::new(Type::I64), 1),
                vec![],
                vec![local_decl(
                    &type_context,
                    Type::Array(Box::new(Type::I64), 1),
                    "return_place",
                )],
                vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Aggregate(
                        AggregateKind::Array,
                        vec![
                            Operand::Constant(Constant::Int(1)),
                            Operand::Constant(Constant::Int(2)),
                        ],
                    ),
                    None,
                )],
            )
        }));
        assert!(too_many_array.is_ok());
        let too_many_array = too_many_array.unwrap();
        assert!(too_many_array.is_err());
        assert!(too_many_array
            .unwrap_err()
            .message
            .contains("operand count"));

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let struct_id = DefId::new(CrateId(0), LocalDefId(30));
        let struct_ty = Type::Struct {
            id: struct_id,
            args: vec![],
        };
        let mut backend_contract = MirBackendContract::default();
        insert_struct_layout_contract(
            &mut backend_contract,
            struct_id,
            vec![("value".to_string(), type_id(&type_context, Type::I64))],
        );
        let too_few_struct = compile_test_function_result_with_contract(
            &mut codegen,
            &type_context,
            "bad_struct_aggregate",
            struct_ty.clone(),
            vec![],
            vec![local_decl(&type_context, struct_ty, "return_place")],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Aggregate(
                    AggregateKind::Struct {
                        id: struct_id,
                        display_name: "OneField".to_string(),
                    },
                    vec![],
                ),
                None,
            )],
            backend_contract,
        );
        assert!(too_few_struct.is_err());
        assert!(too_few_struct
            .unwrap_err()
            .message
            .contains("operand count"));

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let enum_id = DefId::new(CrateId(0), LocalDefId(31));
        let enum_ty = Type::Enum {
            id: enum_id,
            args: vec![],
        };
        let mut backend_contract = MirBackendContract::default();
        insert_enum_layout_contract(
            &mut backend_contract,
            enum_id,
            vec![
                MirEnumVariantLayout {
                    name: "None".to_string(),
                    fields: MirVariantLayoutFields::Unit,
                },
                MirEnumVariantLayout {
                    name: "Some".to_string(),
                    fields: MirVariantLayoutFields::Positional(vec![type_id(
                        &type_context,
                        Type::I64,
                    )]),
                },
            ],
        );
        let too_many_enum_payload = compile_test_function_result_with_contract(
            &mut codegen,
            &type_context,
            "bad_enum_aggregate",
            enum_ty.clone(),
            vec![],
            vec![local_decl(&type_context, enum_ty, "return_place")],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Aggregate(
                    AggregateKind::EnumVariant {
                        enum_id,
                        variant_id: VariantId(1),
                        enum_name: "MaybeInt".to_string(),
                        variant_name: "Some".to_string(),
                    },
                    vec![
                        Operand::Constant(Constant::Int(1)),
                        Operand::Constant(Constant::Int(2)),
                    ],
                ),
                None,
            )],
            backend_contract,
        );
        assert!(too_many_enum_payload.is_err());
        assert!(too_many_enum_payload
            .unwrap_err()
            .message
            .contains("operand count"));
    }

    #[test]
    fn mir_codegen_reports_non_integer_index_projection() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let result = catch_unwind(AssertUnwindSafe(|| {
            compile_test_function_result(
                &mut codegen,
                &type_context,
                "bad_index_projection",
                Type::I64,
                vec![],
                vec![
                    local_decl(&type_context, Type::I64, "return_place"),
                    local_decl(
                        &type_context,
                        Type::Array(Box::new(Type::I64), 2),
                        "array_value",
                    ),
                    local_decl(&type_context, Type::F64, "index"),
                ],
                vec![
                    StatementData::assign(
                        root_place(1),
                        Rvalue::Aggregate(
                            AggregateKind::Array,
                            vec![
                                Operand::Constant(Constant::Int(1)),
                                Operand::Constant(Constant::Int(2)),
                            ],
                        ),
                        None,
                    ),
                    StatementData::assign(
                        root_place(2),
                        Rvalue::Use(Operand::Constant(Constant::Float(0.0))),
                        None,
                    ),
                    StatementData::assign(
                        root_place(0),
                        Rvalue::Use(Operand::Copy(index_place(1, 2))),
                        None,
                    ),
                ],
            )
        }));

        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("index"));
    }

    #[test]
    fn mir_codegen_lowers_single_named_enum_payload_as_tuple() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let enum_id = DefId::new(CrateId(0), LocalDefId(32));
        let enum_ty = Type::Enum {
            id: enum_id,
            args: vec![],
        };

        let mut backend_contract = MirBackendContract::default();
        insert_enum_layout_contract(
            &mut backend_contract,
            enum_id,
            vec![
                MirEnumVariantLayout {
                    name: "None".to_string(),
                    fields: MirVariantLayoutFields::Unit,
                },
                MirEnumVariantLayout {
                    name: "Some".to_string(),
                    fields: MirVariantLayoutFields::Named(vec![(
                        "value".to_string(),
                        type_id(&type_context, Type::I64),
                    )]),
                },
            ],
        );

        compile_test_function_with_contract(
            &mut codegen,
            &type_context,
            "single_named_enum_payload",
            enum_ty.clone(),
            vec![],
            vec![local_decl(&type_context, enum_ty, "return_place")],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Aggregate(
                    AggregateKind::EnumVariant {
                        enum_id,
                        variant_id: VariantId(1),
                        enum_name: "NamedMaybe".to_string(),
                        variant_name: "Some".to_string(),
                    },
                    vec![Operand::Constant(Constant::Int(77))],
                ),
                None,
            )],
            backend_contract,
        );

        let ir = codegen.get_ir();
        assert_ir_order(
            &ir,
            &[
                "store { i32, i64, { i64 } } { i32 1, i64 undef, { i64 } { i64 77 } }, ptr %return_place",
                "ret { i32, i64, { i64 } }",
            ],
        );
    }

    #[test]
    fn mir_codegen_lowers_aggregates_and_projections() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let struct_id = DefId::new(CrateId(0), LocalDefId(20));
        let enum_id = DefId::new(CrateId(0), LocalDefId(21));
        let single_enum_id = DefId::new(CrateId(0), LocalDefId(22));
        let option_variant = VariantId(1);
        let tuple_ty = Type::Tuple(vec![Type::I64, Type::I64]);
        let array_ty = Type::Array(Box::new(Type::I64), 2);
        let struct_ty = Type::Struct {
            id: struct_id,
            args: vec![],
        };
        let enum_ty = Type::Enum {
            id: enum_id,
            args: vec![],
        };
        let single_enum_ty = Type::Enum {
            id: single_enum_id,
            args: vec![],
        };

        let mut backend_contract = MirBackendContract::default();
        insert_struct_layout_contract(
            &mut backend_contract,
            struct_id,
            vec![
                ("left".to_string(), type_id(&type_context, Type::I64)),
                ("right".to_string(), type_id(&type_context, Type::I64)),
            ],
        );
        insert_enum_layout_contract(
            &mut backend_contract,
            enum_id,
            vec![
                MirEnumVariantLayout {
                    name: "None".to_string(),
                    fields: MirVariantLayoutFields::Unit,
                },
                MirEnumVariantLayout {
                    name: "Some".to_string(),
                    fields: MirVariantLayoutFields::Positional(vec![
                        type_id(&type_context, Type::I64),
                        type_id(&type_context, Type::I64),
                    ]),
                },
            ],
        );
        insert_enum_layout_contract(
            &mut backend_contract,
            single_enum_id,
            vec![
                MirEnumVariantLayout {
                    name: "None".to_string(),
                    fields: MirVariantLayoutFields::Unit,
                },
                MirEnumVariantLayout {
                    name: "Some".to_string(),
                    fields: MirVariantLayoutFields::Positional(vec![type_id(
                        &type_context,
                        Type::I64,
                    )]),
                },
            ],
        );

        compile_test_function_with_contract(
            &mut codegen,
            &type_context,
            "aggregates_and_projections",
            Type::I64,
            vec![],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, tuple_ty.clone(), "tuple_value"),
                local_decl(&type_context, struct_ty.clone(), "struct_value"),
                local_decl(&type_context, enum_ty.clone(), "enum_value"),
                local_decl(&type_context, Type::I64, "projected"),
                local_decl(&type_context, Type::I32, "discriminant"),
                local_decl(&type_context, array_ty.clone(), "array_value"),
                local_decl(&type_context, Type::I64, "index"),
                local_decl(&type_context, single_enum_ty.clone(), "single_enum_value"),
                local_decl(&type_context, Type::I64, "single_projected"),
            ],
            vec![
                StatementData::assign(
                    root_place(1),
                    Rvalue::Aggregate(
                        AggregateKind::Tuple,
                        vec![
                            Operand::Constant(Constant::Int(11)),
                            Operand::Constant(Constant::Int(22)),
                        ],
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(2),
                    Rvalue::Aggregate(
                        AggregateKind::Struct {
                            id: struct_id,
                            display_name: "Pair".to_string(),
                        },
                        vec![
                            Operand::Constant(Constant::Int(33)),
                            Operand::Copy(field_place(1, 1)),
                        ],
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(3),
                    Rvalue::Aggregate(
                        AggregateKind::EnumVariant {
                            enum_id,
                            variant_id: option_variant,
                            enum_name: "MaybePair".to_string(),
                            variant_name: "Some".to_string(),
                        },
                        vec![
                            Operand::Constant(Constant::Int(44)),
                            Operand::Copy(field_place(2, 1)),
                        ],
                    ),
                    None,
                ),
                StatementData::assign(root_place(5), Rvalue::Discriminant(root_place(3)), None),
                StatementData::assign(
                    root_place(6),
                    Rvalue::Aggregate(
                        AggregateKind::Array,
                        vec![
                            Operand::Copy(downcast_field_place(3, option_variant, 1)),
                            Operand::Constant(Constant::Int(55)),
                        ],
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(7),
                    Rvalue::Use(Operand::Constant(Constant::Int(0))),
                    None,
                ),
                StatementData::assign(
                    root_place(4),
                    Rvalue::Use(Operand::Copy(index_place(6, 7))),
                    None,
                ),
                StatementData::assign(
                    root_place(8),
                    Rvalue::Aggregate(
                        AggregateKind::EnumVariant {
                            enum_id: single_enum_id,
                            variant_id: option_variant,
                            enum_name: "MaybeInt".to_string(),
                            variant_name: "Some".to_string(),
                        },
                        vec![Operand::Constant(Constant::Int(66))],
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(9),
                    Rvalue::Use(Operand::Copy(downcast_field_place(8, option_variant, 0))),
                    None,
                ),
                StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Copy(root_place(4))),
                    None,
                ),
            ],
            backend_contract,
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("insertvalue"));
        assert!(ir.contains("extractvalue"));
        assert!(ir.contains("getelementptr"));
        assert!(ir.contains("i32 1"));
        assert!(ir.contains("insertvalue { i32, i64, { i64, i64 } }"));
        assert!(ir.contains("extractvalue { i32, i64, { i64, i64 } }"));
        assert_ir_order_ignoring_aggregate_gep_index_widths(
            &ir,
            &[
                "store { i64, i64 } { i64 11, i64 22 }, ptr %tuple_value",
                "%mir_field = getelementptr inbounds { i64, i64 }, ptr %tuple_value, i32 0, i32 1",
                "%mir_place = load i64, ptr %mir_field",
            ],
        );
        assert_ir_order_ignoring_aggregate_gep_index_widths(
            &ir,
            &[
                "%struct_field = insertvalue { i64, i64 } { i64 33, i64 undef }, i64 %mir_place, 1",
                "store { i64, i64 } %struct_field, ptr %struct_value",
                "%mir_field1 = getelementptr inbounds { i64, i64 }, ptr %struct_value, i32 0, i32 1",
                "%mir_place2 = load i64, ptr %mir_field1",
            ],
        );
        assert_ir_order_ignoring_aggregate_gep_index_widths(
            &ir,
            &[
                "%payload_field = insertvalue { i64, i64 } { i64 44, i64 undef }, i64 %mir_place2, 1",
                "%payload = insertvalue { i32, i64, { i64, i64 } } { i32 1, i64 undef, { i64, i64 } undef }, { i64, i64 } %payload_field, 2",
                "store { i32, i64, { i64, i64 } } %payload, ptr %enum_value",
            ],
        );
        assert_ir_order_ignoring_aggregate_gep_index_widths(
            &ir,
            &[
                "%mir_place3 = load { i32, i64, { i64, i64 } }, ptr %enum_value",
                "%discriminant4 = extractvalue { i32, i64, { i64, i64 } } %mir_place3, 0",
                "store i32 %discriminant4, ptr %discriminant",
            ],
        );
        assert_ir_order_ignoring_aggregate_gep_index_widths(
            &ir,
            &[
                "%mir_variant_payload = getelementptr inbounds { i32, i64, { i64, i64 } }, ptr %enum_value, i32 0, i32 2",
                "%mir_field5 = getelementptr inbounds { i64, i64 }, ptr %mir_variant_payload, i32 0, i32 1",
                "%mir_place6 = load i64, ptr %mir_field5",
            ],
        );
        assert_ir_order_ignoring_aggregate_gep_index_widths(
            &ir,
            &[
                "%array_elem = insertvalue [2 x i64] undef, i64 %mir_place6, 0",
                "%array_elem7 = insertvalue [2 x i64] %array_elem, i64 55, 1",
                "store [2 x i64] %array_elem7, ptr %array_value",
                "%mir_index_ptr = getelementptr [2 x i64], ptr %array_value, i64 0, i64 %mir_index",
                "%mir_place8 = load i64, ptr %mir_index_ptr",
            ],
        );
        assert_ir_order_ignoring_aggregate_gep_index_widths(
            &ir,
            &[
                "store { i32, i64, i64 } { i32 1, i64 undef, i64 66 }, ptr %single_enum_value",
                "%mir_variant_payload9 = getelementptr inbounds { i32, i64, i64 }, ptr %single_enum_value, i32 0, i32 2",
                "%mir_place10 = load i64, ptr %mir_variant_payload9",
                "store i64 %mir_place10, ptr %single_projected",
            ],
        );
    }

    #[test]
    fn mir_codegen_lowers_repeated_array_as_fill_loop() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let array_ty = Type::Array(Box::new(Type::I64), 256);

        compile_test_function_with_contract(
            &mut codegen,
            &type_context,
            "repeat_array",
            Type::I64,
            vec![],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, Type::I64, "element"),
                local_decl(&type_context, array_ty, "array"),
            ],
            vec![
                StatementData::assign(
                    root_place(1),
                    Rvalue::Use(Operand::Constant(Constant::Int(7))),
                    None,
                ),
                StatementData::assign(
                    root_place(2),
                    Rvalue::Aggregate(
                        AggregateKind::Array,
                        vec![Operand::Copy(root_place(1)); 256],
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(Constant::Int(0))),
                    None,
                ),
            ],
            MirBackendContract::default(),
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("array_repeat_loop"));
        assert!(ir.contains("array_repeat_element"));
        assert!(!ir.contains("insertvalue [256 x i64]"));
    }

    #[test]
    fn mir_codegen_lowers_control_flow() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(0));
        let ret_type = type_id(&type_context, Type::I64);
        let function = MirFunction {
            id: function_id.clone(),
            name: "control_flow".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Goto(crate::mir::BasicBlockId(1))),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::SwitchInt {
                        discr: Operand::Copy(root_place(1)),
                        targets: vec![
                            (0, crate::mir::BasicBlockId(2)),
                            (1, crate::mir::BasicBlockId(3)),
                        ],
                        otherwise: crate::mir::BasicBlockId(4),
                    }),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Drop {
                        place: root_place(0),
                        target: crate::mir::BasicBlockId(4),
                    }),
                },
                BasicBlock {
                    statements: vec![StatementData::assign(
                        root_place(0),
                        Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(7))),
                        None,
                    )],
                    terminator: Some(Terminator::Return),
                },
                BasicBlock {
                    statements: vec![StatementData::assign(
                        root_place(0),
                        Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(0))),
                        None,
                    )],
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: type_id(&type_context, Type::I64),
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                local_decl(&type_context, Type::I64, "drop_place"),
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        };
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract: local_body_contract(function_id, "control_flow", ret_type),
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("switch_eq"));
        assert!(ir.contains("br i1"));
        assert!(ir.contains("br label"));
        assert_ir_order(&ir, &["entry:", "br label %bb1", "bb1:"]);
        assert_ir_order(
            &ir,
            &[
                "bb1:",
                "switch_eq = icmp eq",
                "br i1",
                "label %bb2, label %switch.next0",
                "switch.next0:",
                "switch_eq1 = icmp eq",
                "br i1",
                "label %bb3, label %switch.next1",
                "switch.next1:",
                "br label %bb4",
            ],
        );
        assert_ir_order(&ir, &["bb2:", "br label %bb4", "bb3:"]);
        assert!(!ir.contains("drop_place_hir"));
    }

    #[test]
    fn mir_codegen_calls_drop_impl_before_drop_target() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(20));
        let struct_id = DefId::new(CrateId(0), LocalDefId(80));
        let drop_method_id = DefId::new(CrateId(0), LocalDefId(83));
        let box_ty = Type::Struct {
            id: struct_id,
            args: Vec::new(),
        };
        let box_type_id = type_id(&type_context, box_ty.clone());
        let unit_type = type_id(&type_context, Type::Unit);
        let ret_type = type_id(&type_context, Type::I64);
        let field_type = type_id(&type_context, Type::I64);
        let function = MirFunction {
            id: function_id.clone(),
            name: "drop_box".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![StatementData::assign(
                        root_place(1),
                        Rvalue::Aggregate(
                            AggregateKind::Struct {
                                id: struct_id,
                                display_name: "Box".to_string(),
                            },
                            vec![Operand::Constant(Constant::Int(7))],
                        ),
                        None,
                    )],
                    terminator: Some(Terminator::Drop {
                        place: root_place(1),
                        target: crate::mir::BasicBlockId(1),
                    }),
                },
                BasicBlock {
                    statements: vec![StatementData::assign(
                        root_place(0),
                        Rvalue::Use(Operand::Constant(Constant::Int(0))),
                        None,
                    )],
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, box_ty, "box_value"),
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        };
        let mut backend_contract = drop_glue_contract(
            box_type_id,
            MirCallableKey::Instance(InstanceId(120)),
            drop_method_id,
            "Box_drop",
            unit_type,
        );
        insert_local_body_contract(
            &mut backend_contract,
            function_id.clone(),
            "drop_box",
            &[],
            ret_type,
        );
        insert_struct_layout_contract(
            &mut backend_contract,
            struct_id,
            vec![("value".to_string(), field_type)],
        );
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert_ir_order(&ir, &["call void @Box_drop", "br label %bb1"]);
    }

    #[test]
    fn mir_codegen_rejects_drop_glue_contract_codegen_will_not_declare() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(22));
        let struct_id = DefId::new(CrateId(0), LocalDefId(87));
        let box_ty = Type::Struct {
            id: struct_id,
            args: Vec::new(),
        };
        let box_type_id = type_id(&type_context, box_ty.clone());
        let unit_type = type_id(&type_context, Type::Unit);
        let ret_type = type_id(&type_context, Type::I64);
        let field_type = type_id(&type_context, Type::I64);

        let function = MirFunction {
            id: function_id.clone(),
            name: "drop_box_missing_glue".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![StatementData::assign(
                        root_place(1),
                        Rvalue::Aggregate(
                            AggregateKind::Struct {
                                id: struct_id,
                                display_name: "Box".to_string(),
                            },
                            vec![Operand::Constant(Constant::Int(7))],
                        ),
                        None,
                    )],
                    terminator: Some(Terminator::Drop {
                        place: root_place(1),
                        target: crate::mir::BasicBlockId(1),
                    }),
                },
                BasicBlock {
                    statements: vec![StatementData::assign(
                        root_place(0),
                        Rvalue::Use(Operand::Constant(Constant::Int(0))),
                        None,
                    )],
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, box_ty, "box_value"),
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        };
        let mut backend_contract = undeclarable_drop_glue_contract(
            box_type_id,
            MirCallableKey::Instance(InstanceId(122)),
            "Box_drop",
            unit_type,
        );
        insert_local_body_contract(
            &mut backend_contract,
            function_id.clone(),
            "drop_box_missing_glue",
            &[],
            ret_type,
        );
        backend_contract.nominal_layouts.insert(
            struct_id,
            MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("value".to_string(), field_type)],
                generic_params: Vec::new(),
            },
        );
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        let error = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            error.message.contains("Invalid MIR backend contract")
                && error.message.contains("DropGlueCallableKindMismatch"),
            "unexpected error: {}",
            error.message
        );
    }

    #[test]
    fn mir_drop_without_required_glue_is_codegen_error() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(23));
        let struct_id = DefId::new(CrateId(0), LocalDefId(88));
        let box_ty = Type::Struct {
            id: struct_id,
            args: Vec::new(),
        };
        let i64_ty = type_id(&type_context, Type::I64);
        let function = MirFunction {
            id: function_id.clone(),
            name: "drop_box_without_glue".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![StatementData::assign(
                        root_place(1),
                        Rvalue::Aggregate(
                            AggregateKind::Struct {
                                id: struct_id,
                                display_name: "Box".to_string(),
                            },
                            vec![Operand::Constant(Constant::Int(7))],
                        ),
                        None,
                    )],
                    terminator: Some(Terminator::Drop {
                        place: root_place(1),
                        target: crate::mir::BasicBlockId(1),
                    }),
                },
                BasicBlock {
                    statements: vec![StatementData::assign(
                        root_place(0),
                        Rvalue::Use(Operand::Constant(Constant::Int(0))),
                        None,
                    )],
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, box_ty, "box_value"),
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: type_id(&type_context, Type::I64),
            ownership: Default::default(),
        };
        let mut backend_contract = local_body_contract(
            function_id.clone(),
            "drop_box_without_glue",
            type_id(&type_context, Type::I64),
        );
        backend_contract.nominal_layouts.insert(
            struct_id,
            MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("value".to_string(), i64_ty)],
                generic_params: Vec::new(),
            },
        );
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        let error = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            error
                .message
                .contains("Missing drop glue contract for MIR drop")
                && error.message.contains("drop_box_without_glue"),
            "unexpected error: {}",
            error.message
        );
    }

    #[test]
    fn projected_return_place_drop_without_required_glue_is_codegen_error() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(24));
        let struct_id = DefId::new(CrateId(0), LocalDefId(89));
        let owner_ty = Type::Struct {
            id: struct_id,
            args: Vec::new(),
        };
        let i64_ty = type_id(&type_context, Type::I64);
        let owner_type_id = type_id(&type_context, owner_ty.clone());
        let function = MirFunction {
            id: function_id.clone(),
            name: "drop_return_field_without_glue".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Drop {
                        place: field_place(0, 0),
                        target: crate::mir::BasicBlockId(1),
                    }),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![LocalDecl {
                ty: owner_type_id,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: owner_type_id,
            ownership: Default::default(),
        };
        let mut backend_contract = local_body_contract(
            function_id.clone(),
            "drop_return_field_without_glue",
            owner_type_id,
        );
        backend_contract.nominal_layouts.insert(
            struct_id,
            MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("value".to_string(), i64_ty)],
                generic_params: Vec::new(),
            },
        );
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        let error = codegen.compile_program_from_mir(&mir).unwrap_err();

        assert!(
            error
                .message
                .contains("Missing drop glue contract for MIR drop")
                && error.message.contains("drop_return_field_without_glue"),
            "unexpected error: {}",
            error.message
        );
    }

    #[test]
    fn mir_codegen_does_not_call_drop_impl_for_return_place() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(21));
        let struct_id = DefId::new(CrateId(0), LocalDefId(83));
        let drop_method_id = DefId::new(CrateId(0), LocalDefId(86));
        let box_ty = Type::Struct {
            id: struct_id,
            args: Vec::new(),
        };
        let return_ty = type_id(&type_context, box_ty.clone());
        let unit_type = type_id(&type_context, Type::Unit);
        let field_type = type_id(&type_context, Type::I64);
        let function = MirFunction {
            id: function_id.clone(),
            name: "return_box".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Drop {
                        place: root_place(0),
                        target: crate::mir::BasicBlockId(1),
                    }),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![LocalDecl {
                ty: return_ty,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: return_ty,
            ownership: Default::default(),
        };
        let mut backend_contract = drop_glue_contract(
            return_ty,
            MirCallableKey::Instance(InstanceId(121)),
            drop_method_id,
            "Box_drop",
            unit_type,
        );
        insert_local_body_contract(
            &mut backend_contract,
            function_id.clone(),
            "return_box",
            &[],
            return_ty,
        );
        insert_struct_layout_contract(
            &mut backend_contract,
            struct_id,
            vec![("value".to_string(), field_type)],
        );
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert!(!ir.contains("call void @Box_drop"));
    }

    #[test]
    fn mir_codegen_reports_invalid_terminator_target() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(0));
        let ret_type = type_id(&type_context, Type::I64);
        let function = MirFunction {
            id: function_id.clone(),
            name: "bad_target".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Goto(crate::mir::BasicBlockId(99))),
            }],
            local_decls: vec![local_decl(&type_context, Type::I64, "return_place")],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        };
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract: local_body_contract(function_id, "bad_target", ret_type),
        };

        let error = codegen.compile_program_from_mir(&mir).unwrap_err();
        assert!(error.message.contains("invalid MIR basic block target"));
    }

    #[test]
    fn mir_codegen_lowers_canonical_calls() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let callee_instance = InstanceId(7);
        let callee_key = MirCallableKey::Instance(callee_instance);
        let callee_symbol = "canonical_callee";
        let callable_ty = Type::function(vec![Type::I64], Type::I64);
        let i64_id = type_id(&type_context, Type::I64);

        let function_id = MirFunctionId::Instance(InstanceId(0));
        let function = MirFunction {
            id: function_id.clone(),
            name: "canonical_calls".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![StatementData::assign(
                        root_place(1),
                        Rvalue::Use(Operand::Constant(Constant::Callable(
                            MirCallable::Resolved(callee_key.clone()),
                        ))),
                        None,
                    )],
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                            callee_key.clone(),
                        ))),
                        args: vec![Operand::Constant(Constant::Int(41))],
                        destination: root_place(0),
                        target: crate::mir::BasicBlockId(1),
                        span: None,
                    }),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, callable_ty, "callable_value"),
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_id,
            ownership: Default::default(),
        };
        let mut backend_contract =
            local_body_contract(function_id.clone(), "canonical_calls", i64_id);
        insert_object_callable_contract(
            &mut backend_contract,
            callee_key.clone(),
            callee_symbol,
            &[i64_id],
            i64_id,
        );
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("__callable_wrap_canonical_callee_0"));
        assert!(ir.contains("call i64 @canonical_callee(i64 41)"));
        assert!(ir.contains("store i64 %mir_call, ptr %return_place"));

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let higher_order_instance = InstanceId(8);
        let callback_instance = InstanceId(9);
        let higher_order_key = MirCallableKey::Instance(higher_order_instance);
        let callback_key = MirCallableKey::Instance(callback_instance);
        let higher_order_symbol = "higher_order_callee";
        let callback_symbol = "callback_callee";
        let callback_ty = Type::function(vec![Type::I64], Type::I64);
        let i64_id = type_id(&type_context, Type::I64);
        let callback_type_id = type_id(&type_context, callback_ty.clone());

        let function_id = MirFunctionId::Instance(InstanceId(0));
        let function = MirFunction {
            id: function_id.clone(),
            name: "canonical_callable_arg".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                            higher_order_key.clone(),
                        ))),
                        args: vec![Operand::Constant(Constant::Callable(
                            MirCallable::Resolved(callback_key.clone()),
                        ))],
                        destination: root_place(0),
                        target: crate::mir::BasicBlockId(1),
                        span: None,
                    }),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![local_decl(&type_context, Type::I64, "return_place")],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_id,
            ownership: Default::default(),
        };
        let mut backend_contract =
            local_body_contract(function_id.clone(), "canonical_callable_arg", i64_id);
        insert_object_callable_contract(
            &mut backend_contract,
            higher_order_key.clone(),
            higher_order_symbol,
            &[callback_type_id],
            i64_id,
        );
        insert_object_callable_contract(
            &mut backend_contract,
            callback_key.clone(),
            callback_symbol,
            &[i64_id],
            i64_id,
        );
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("__callable_wrap_callback_callee_0"));
        assert!(ir.contains("call i64 @higher_order_callee"));

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let callback_instance = InstanceId(10);
        let callback_key = MirCallableKey::Instance(callback_instance);
        let callback_symbol = "pointer_abi_callback";
        let callback_ty = Type::function(vec![Type::I64], Type::I64);
        let i64_id = type_id(&type_context, Type::I64);

        let callback_signature = MirCallableSignature {
            params: vec![crate::mir::MirParamAbi {
                semantic_ty: i64_id,
                pass_mode: MirPassMode::Pointer,
            }],
            ret: crate::mir::MirReturnAbi {
                semantic_ty: i64_id,
                abi_ty: i64_id,
            },
        };

        let function_id = MirFunctionId::Instance(InstanceId(0));
        let function = MirFunction {
            id: function_id.clone(),
            name: "pointer_abi_callable_value".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(1),
                    Rvalue::Use(Operand::Constant(Constant::Callable(
                        MirCallable::Resolved(callback_key.clone()),
                    ))),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, callback_ty, "callable_value"),
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: i64_id,
            ownership: Default::default(),
        };
        let mut backend_contract =
            local_body_contract(function_id.clone(), "pointer_abi_callable_value", i64_id);
        insert_object_callable_contract_with_signature(
            &mut backend_contract,
            callback_key.clone(),
            callback_symbol,
            callback_signature,
        );
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("__callable_wrap_pointer_abi_callback_0"));
        assert!(ir.contains("call i64 @pointer_abi_callback(ptr %mir_pointer_arg"));

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(0));
        let missing_instance = InstanceId(99);
        let ret_type = type_id(&type_context, Type::I64);
        let function = MirFunction {
            id: function_id.clone(),
            name: "unknown_canonical_call".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Instance(missing_instance),
                    ))),
                    args: vec![],
                    destination: root_place(0),
                    target: crate::mir::BasicBlockId(0),
                    span: None,
                }),
            }],
            local_decls: vec![local_decl(&type_context, Type::I64, "return_place")],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        };
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract: local_body_contract(function_id, "unknown_canonical_call", ret_type),
        };

        let error = codegen.compile_program_from_mir(&mir).unwrap_err();
        assert!(error.message.contains("ResolvedCallableWithoutContract"));
        assert!(error.message.contains("Instance(InstanceId(99))"));
    }

    #[test]
    fn mir_codegen_uses_contract_return_abi_for_main() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(0));
        let ret_type = type_id(&type_context, Type::I64);
        let function = MirFunction {
            id: function_id.clone(),
            name: "main".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![LocalDecl {
                ty: ret_type,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        };
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract: local_body_contract(function_id, "main", ret_type),
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("define internal i64 @__rock_main"));
        assert!(ir.contains("define i32 @main(i32"));
        assert!(ir.contains("call i64 @__rock_main()"));
        assert!(ir.contains("define i64 @__rock_runtime_arg_count()"));
        assert!(ir.contains("define ptr @__rock_runtime_arg_at(i64"));
    }

    #[test]
    fn mir_codegen_declares_and_verifies_empty_return_function() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(0));
        let ret_type = type_id(&type_context, Type::I64);
        let ret_abi_type = type_id(&type_context, Type::I32);
        let function = MirFunction {
            id: function_id.clone(),
            name: "main".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![LocalDecl {
                ty: type_id(&type_context, Type::I64),
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        };
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract: local_body_contract_with_return_abi(
                function_id,
                "main",
                ret_type,
                ret_abi_type,
            ),
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("define internal i32 @__rock_main"));
        assert!(ir.contains("define i32 @main(i32"));
        assert!(ir.contains("call i32 @__rock_main()"));
    }

    #[test]
    fn mir_codegen_returns_constant_assignment_to_return_place() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(0));
        let ret_type = type_id(&type_context, Type::I64);
        let ret_abi_type = type_id(&type_context, Type::I32);
        let function = MirFunction {
            id: function_id.clone(),
            name: "main".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(42),
                    )),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![LocalDecl {
                ty: type_id(&type_context, Type::I64),
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        };
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract: local_body_contract_with_return_abi(
                function_id,
                "main",
                ret_type,
                ret_abi_type,
            ),
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("ret i32 42"));
        assert!(!ir.contains("store i64 42"));
    }

    #[test]
    fn mir_codegen_binds_parameters_to_argument_locals() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(0));
        let i64_id = type_id(&type_context, Type::I64);
        let function = MirFunction {
            id: function_id.clone(),
            name: "identity".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Copy(crate::mir::Place {
                        local: crate::mir::Local(1),
                        projection: vec![],
                    })),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: i64_id,
                    mutability: Mutability::Not,
                    name: Some("arg".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 1,
            ret_type: i64_id,
            ownership: Default::default(),
        };
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract: local_body_contract_with_params(
                function_id,
                "identity",
                &[i64_id],
                i64_id,
            ),
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("define i64 @identity(i64"));
        assert!(ir.contains("store i64 %0, ptr %arg"));
        assert!(ir.contains("load i64, ptr %arg"));
        assert!(ir.contains("store i64 %mir_place, ptr %return_place"));
    }

    #[test]
    fn mir_codegen_sanitizes_invalid_local_display_names() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(0));
        let ret_type = type_id(&type_context, Type::I64);
        let function = MirFunction {
            id: function_id.clone(),
            name: "bad_local".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![LocalDecl {
                ty: ret_type,
                mutability: Mutability::Mut,
                name: Some("bad\0local".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            }],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        };
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract: local_body_contract(function_id, "bad_local", ret_type),
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();
        assert!(codegen.current_function.is_none());
        assert!(codegen.get_ir().contains("%local0 = alloca i64"));
    }

    #[test]
    fn mir_codegen_lowers_binary_integer_add_rvalue() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        compile_test_function(
            &mut codegen,
            &type_context,
            "add_values",
            Type::I64,
            vec![Type::I64, Type::I64],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, Type::I64, "lhs"),
                local_decl(&type_context, Type::I64, "rhs"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::BinaryOp(
                    MirBinOp::Add,
                    Operand::Copy(root_place(1)),
                    Operand::Copy(root_place(2)),
                ),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("add i64"));
        assert!(ir.contains("ret i64"));
    }

    #[test]
    fn mir_codegen_lowers_unsigned_less_than_with_unsigned_predicate() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        compile_test_function(
            &mut codegen,
            &type_context,
            "u64_less_than",
            Type::Bool,
            vec![Type::U64, Type::U64],
            vec![
                local_decl(&type_context, Type::Bool, "return_place"),
                local_decl(&type_context, Type::U64, "lhs"),
                local_decl(&type_context, Type::U64, "rhs"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::BinaryOp(
                    MirBinOp::Lt,
                    Operand::Copy(root_place(1)),
                    Operand::Copy(root_place(2)),
                ),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("icmp ult"));
        assert!(!ir.contains("icmp slt"));
    }

    #[test]
    fn mir_codegen_lowers_unsigned_right_shift_logically() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        compile_test_function(
            &mut codegen,
            &type_context,
            "u64_shift_right",
            Type::U64,
            vec![Type::U64, Type::U64],
            vec![
                local_decl(&type_context, Type::U64, "return_place"),
                local_decl(&type_context, Type::U64, "lhs"),
                local_decl(&type_context, Type::U64, "rhs"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::BinaryOp(
                    MirBinOp::Shr,
                    Operand::Copy(root_place(1)),
                    Operand::Copy(root_place(2)),
                ),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("lshr"));
        assert!(!ir.contains("ashr"));
    }

    #[test]
    fn mir_codegen_zero_extends_u8_less_than_integer_constant() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        compile_test_function(
            &mut codegen,
            &type_context,
            "u8_less_than_constant",
            Type::Bool,
            vec![Type::U8],
            vec![
                local_decl(&type_context, Type::Bool, "return_place"),
                local_decl(&type_context, Type::U8, "lhs"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::BinaryOp(
                    MirBinOp::Lt,
                    Operand::Copy(root_place(1)),
                    Operand::Constant(crate::mir::Constant::Int(255)),
                ),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("icmp ult"));
        assert!(ir.contains("zext"));
        assert!(!ir.contains("sext"));
        assert!(!ir.contains("icmp slt"));
    }

    #[test]
    fn mir_codegen_zero_extends_u8_right_shift_integer_constant() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        compile_test_function(
            &mut codegen,
            &type_context,
            "u8_shift_right_constant",
            Type::U8,
            vec![Type::U8],
            vec![
                local_decl(&type_context, Type::U8, "return_place"),
                local_decl(&type_context, Type::U8, "lhs"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::BinaryOp(
                    MirBinOp::Shr,
                    Operand::Copy(root_place(1)),
                    Operand::Constant(crate::mir::Constant::Int(1)),
                ),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("lshr"));
        assert!(ir.contains("zext"));
        assert!(!ir.contains("sext"));
        assert!(!ir.contains("ashr"));
    }

    #[test]
    fn mir_codegen_infers_unsigned_type_for_left_integer_constant_less_than() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        compile_test_function(
            &mut codegen,
            &type_context,
            "constant_less_than_u8",
            Type::Bool,
            vec![Type::U8],
            vec![
                local_decl(&type_context, Type::Bool, "return_place"),
                local_decl(&type_context, Type::U8, "rhs"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::BinaryOp(
                    MirBinOp::Lt,
                    Operand::Constant(crate::mir::Constant::Int(0)),
                    Operand::Copy(root_place(1)),
                ),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("icmp ult"));
        assert!(ir.contains("zext"));
        assert!(!ir.contains("icmp slt"));
        assert!(!ir.contains("sext"));
    }

    #[test]
    fn mir_codegen_infers_unsigned_type_for_left_integer_constant_right_shift() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        compile_test_function(
            &mut codegen,
            &type_context,
            "constant_shift_right_u8",
            Type::U64,
            vec![Type::U8],
            vec![
                local_decl(&type_context, Type::U64, "return_place"),
                local_decl(&type_context, Type::U8, "rhs"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::BinaryOp(
                    MirBinOp::Shr,
                    Operand::Constant(crate::mir::Constant::Int(128)),
                    Operand::Copy(root_place(1)),
                ),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("lshr"));
        assert!(ir.contains("zext"));
        assert!(!ir.contains("ashr"));
        assert!(!ir.contains("sext"));
    }

    #[test]
    fn mir_codegen_lowers_unary_integer_neg_rvalue() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        compile_test_function(
            &mut codegen,
            &type_context,
            "neg_value",
            Type::I64,
            vec![Type::I64],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, Type::I64, "value"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::UnaryOp(MirUnaryOp::Neg, Operand::Copy(root_place(1))),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("sub i64 0"));
        assert!(ir.contains("ret i64"));
    }

    #[test]
    fn mir_codegen_lowers_integer_trunc_cast_rvalue() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        compile_test_function(
            &mut codegen,
            &type_context,
            "trunc_value",
            Type::I32,
            vec![Type::I64],
            vec![
                local_decl(&type_context, Type::I32, "return_place"),
                local_decl(&type_context, Type::I64, "value"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Cast(
                    Operand::Copy(root_place(1)),
                    type_id(&type_context, Type::I32),
                ),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("trunc i64"));
        assert!(ir.contains("ret i32"));
    }

    #[test]
    fn mir_codegen_lowers_char_to_u8_cast_rvalue() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        compile_test_function(
            &mut codegen,
            &type_context,
            "char_to_u8",
            Type::U8,
            vec![Type::Char],
            vec![
                local_decl(&type_context, Type::U8, "return_place"),
                local_decl(&type_context, Type::Char, "ch"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Cast(
                    Operand::Copy(root_place(1)),
                    type_id(&type_context, Type::U8),
                ),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("ret i8"));
    }

    #[test]
    fn mir_codegen_lowers_string_constant_rvalue() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        compile_test_function(
            &mut codegen,
            &type_context,
            "string_constant",
            Type::Str,
            vec![],
            vec![local_decl(&type_context, Type::Str, "return_place")],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Constant(Constant::String("abc".to_string()))),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("@str"));
        assert!(ir.contains("ret { ptr, i64 }"));
    }

    #[test]
    fn mir_codegen_lowers_pointer_deref_place() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let ptr_ty = Type::Pointer(Box::new(Type::U8));

        compile_test_function(
            &mut codegen,
            &type_context,
            "load_deref",
            Type::U8,
            vec![ptr_ty.clone()],
            vec![
                local_decl(&type_context, Type::U8, "return_place"),
                local_decl(&type_context, ptr_ty, "ptr"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Copy(Place {
                    local: Local(1),
                    projection: vec![Projection::Deref],
                })),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("load i8, ptr"));
        assert!(ir.contains("ret i8"));
    }

    #[test]
    fn mir_codegen_lowers_raw_pointer_index_place_without_bounds_check() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let ptr_ty = Type::Pointer(Box::new(Type::I64));

        compile_test_function(
            &mut codegen,
            &type_context,
            "load_ptr_index",
            Type::I64,
            vec![ptr_ty.clone(), Type::I64],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, ptr_ty, "ptr"),
                local_decl(&type_context, Type::I64, "index"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Use(Operand::Copy(Place {
                    local: Local(1),
                    projection: vec![Projection::Index(Local(2))],
                })),
                None,
            )],
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("getelementptr i64, ptr"));
        assert!(!ir.contains("arr_oob"));
    }

    #[test]
    fn mir_codegen_normalizes_projected_array_place_before_index() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let trait_id = DefId::new(CrateId(0), LocalDefId(940));
        let base_struct_id = DefId::new(CrateId(0), LocalDefId(941));
        let assoc_type_id = AssocTypeId(0);
        let (projected_array_ty, projection_key, output_id) = projection_type_with_output(
            &type_context,
            Type::Struct {
                id: base_struct_id,
                args: Vec::new(),
            },
            trait_id,
            assoc_type_id,
            vec![Type::Bool],
            Type::Array(Box::new(Type::I64), 2),
        );
        let function_id = MirFunctionId::Instance(InstanceId(0));
        let ret_type = type_id(&type_context, Type::I64);
        let function = MirFunction {
            id: function_id.clone(),
            name: "projected_array_index".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![
                    StatementData::assign(
                        root_place(2),
                        Rvalue::Use(Operand::Constant(Constant::Int(1))),
                        None,
                    ),
                    StatementData::assign(
                        root_place(0),
                        Rvalue::Use(Operand::Copy(index_place(1, 2))),
                        None,
                    ),
                ],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, projected_array_ty, "projected_array"),
                local_decl(&type_context, Type::I64, "index"),
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        };
        let mut backend_contract =
            local_body_contract(function_id.clone(), "projected_array_index", ret_type);
        backend_contract.projection_traits.insert(trait_id);
        backend_contract
            .projection_outputs
            .insert(projection_key, output_id);
        insert_struct_layout_contract(&mut backend_contract, base_struct_id, Vec::new());
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id, function)]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();

        let ir = codegen.get_ir();
        assert!(ir.contains("getelementptr [2 x i64], ptr %projected_array"));
    }

    #[test]
    fn mir_contract_validation_normalizes_projected_field_type_before_index() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(930));
        let struct_id = DefId::new(CrateId(0), LocalDefId(942));
        let trait_id = DefId::new(CrateId(0), LocalDefId(943));
        let projection_base_id = DefId::new(CrateId(0), LocalDefId(944));
        let assoc_type_id = AssocTypeId(0);
        let unit = type_id(&type_context, Type::Unit);
        let (projected_array_ty, projection_key, output_id) = projection_type_with_output(
            &type_context,
            Type::Struct {
                id: projection_base_id,
                args: Vec::new(),
            },
            trait_id,
            assoc_type_id,
            Vec::new(),
            Type::Array(Box::new(Type::I64), 2),
        );
        let projected_array_id = type_id(&type_context, projected_array_ty);
        let owner_ty = Type::Struct {
            id: struct_id,
            args: Vec::new(),
        };
        let owner_type_id = type_id(&type_context, owner_ty.clone());
        let index_ty = type_id(&type_context, Type::I64);
        let function = MirFunction {
            id: function_id.clone(),
            name: "drop_projected_field_index".to_string(),
            basic_blocks: vec![
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Drop {
                        place: Place {
                            local: Local(1),
                            projection: vec![
                                Projection::Field {
                                    index: 0,
                                    identity: None,
                                },
                                Projection::Index(Local(2)),
                            ],
                        },
                        target: crate::mir::BasicBlockId(1),
                    }),
                },
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Some(Terminator::Return),
                },
            ],
            local_decls: vec![
                LocalDecl {
                    ty: unit,
                    mutability: Mutability::Mut,
                    name: Some("return_place".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::ReturnPlace,
                },
                LocalDecl {
                    ty: owner_type_id,
                    mutability: Mutability::Mut,
                    name: Some("owner".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
                LocalDecl {
                    ty: index_ty,
                    mutability: Mutability::Mut,
                    name: Some("index".to_string()),
                    span: None,
                    source: crate::mir::LocalSource::UserBinding,
                },
            ],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type: unit,
            ownership: Default::default(),
        };
        let callable_key = MirCallableKey::Instance(InstanceId(930));
        let mut backend_contract = MirBackendContract::default();
        backend_contract.projection_traits.insert(trait_id);
        backend_contract
            .projection_outputs
            .insert(projection_key, output_id);
        insert_struct_layout_contract(&mut backend_contract, projection_base_id, Vec::new());
        backend_contract.nominal_layouts.insert(
            struct_id,
            MirNominalLayout::Struct {
                id: struct_id,
                fields: vec![("items".to_string(), projected_array_id)],
                generic_params: Vec::new(),
            },
        );
        backend_contract.callables.insert(
            callable_key.clone(),
            MirCallableDecl {
                key: callable_key.clone(),
                source_def_id: None,
                kind: MirCallableKind::LocalBody {
                    function_id: function_id.clone(),
                },
                llvm_symbol: "drop_projected_field_index".to_string(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(&[], unit, MirPassMode::Direct),
            },
        );
        backend_contract
            .function_bodies
            .insert(function_id.clone(), callable_key);
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id, function)]),
            type_context: type_context.borrow().clone(),
            backend_contract,
        };

        compile_valid_mir_program(&mut codegen, &mir).unwrap();
    }

    #[test]
    fn mir_codegen_lowers_references_and_checks() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let array_ty = Type::Array(Box::new(Type::I64), 3);
        let scalar_ref_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };
        let array_ref_ty = Type::Reference {
            mutable: false,
            inner: Box::new(array_ty.clone()),
        };
        let slice_ref_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Slice(Box::new(Type::I64))),
        };
        let str_ref_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        };
        let raw_ptr_ty = Type::Pointer(Box::new(Type::I64));
        let raw_slice_ptr_ty = Type::Pointer(Box::new(Type::Slice(Box::new(Type::I64))));

        compile_test_function_with_contract(
            &mut codegen,
            &type_context,
            "refs_and_checks",
            Type::I64,
            vec![
                slice_ref_ty.clone(),
                raw_slice_ptr_ty.clone(),
                str_ref_ty.clone(),
            ],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, slice_ref_ty.clone(), "slice_param"),
                local_decl(&type_context, raw_slice_ptr_ty.clone(), "raw_slice_param"),
                local_decl(&type_context, str_ref_ty.clone(), "str_param"),
                local_decl(&type_context, Type::I64, "scalar"),
                local_decl(&type_context, scalar_ref_ty.clone(), "scalar_ref"),
                local_decl(&type_context, raw_ptr_ty.clone(), "raw_ptr"),
                local_decl(&type_context, Type::I64, "ptr_bits"),
                local_decl(&type_context, array_ty.clone(), "array_value"),
                local_decl(&type_context, slice_ref_ty.clone(), "slice_ref"),
                local_decl(&type_context, array_ref_ty.clone(), "array_ref"),
                local_decl(&type_context, slice_ref_ty.clone(), "cast_slice_ref"),
                local_decl(&type_context, Type::F64, "float_value"),
                local_decl(&type_context, Type::I64, "int_value"),
                local_decl(&type_context, raw_ptr_ty.clone(), "int_ptr"),
                local_decl(&type_context, raw_slice_ptr_ty.clone(), "raw_slice_cast"),
            ],
            vec![
                StatementData::assign(
                    root_place(4),
                    Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(9))),
                    None,
                ),
                StatementData::assign(
                    root_place(5),
                    Rvalue::Ref(Mutability::Not, root_place(4)),
                    None,
                ),
                StatementData::assign(
                    root_place(6),
                    Rvalue::Cast(
                        Operand::Copy(root_place(5)),
                        type_id(&type_context, raw_ptr_ty.clone()),
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(7),
                    Rvalue::Cast(
                        Operand::Copy(root_place(6)),
                        type_id(&type_context, Type::I64),
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(8),
                    Rvalue::Aggregate(
                        AggregateKind::Array,
                        vec![
                            Operand::Constant(crate::mir::Constant::Int(1)),
                            Operand::Constant(crate::mir::Constant::Int(2)),
                            Operand::Constant(crate::mir::Constant::Int(3)),
                        ],
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(9),
                    Rvalue::Ref(Mutability::Not, root_place(8)),
                    None,
                ),
                StatementData::assign(
                    root_place(10),
                    Rvalue::Ref(Mutability::Not, root_place(8)),
                    None,
                ),
                StatementData::assign(
                    root_place(11),
                    Rvalue::Cast(
                        Operand::Copy(root_place(10)),
                        type_id(&type_context, slice_ref_ty.clone()),
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(12),
                    Rvalue::Cast(
                        Operand::Copy(root_place(4)),
                        type_id(&type_context, Type::F64),
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(13),
                    Rvalue::Cast(
                        Operand::Copy(root_place(12)),
                        type_id(&type_context, Type::I64),
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(14),
                    Rvalue::Cast(
                        Operand::Copy(root_place(13)),
                        type_id(&type_context, raw_ptr_ty.clone()),
                    ),
                    None,
                ),
                StatementData::assign(
                    root_place(15),
                    Rvalue::Cast(
                        Operand::Copy(root_place(11)),
                        type_id(&type_context, raw_slice_ptr_ty.clone()),
                    ),
                    None,
                ),
                StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![
                            Operand::Copy(root_place(8)),
                            Operand::Constant(crate::mir::Constant::Int(2)),
                        ],
                    },
                    None,
                ),
                StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![
                            Operand::Copy(root_place(9)),
                            Operand::Constant(crate::mir::Constant::Int(2)),
                        ],
                    },
                    None,
                ),
                StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![
                            Operand::Copy(root_place(11)),
                            Operand::Constant(crate::mir::Constant::Int(2)),
                        ],
                    },
                    None,
                ),
                StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![
                            Operand::Copy(root_place(1)),
                            Operand::Constant(crate::mir::Constant::Int(0)),
                        ],
                    },
                    None,
                ),
                StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![
                            Operand::Copy(root_place(2)),
                            Operand::Constant(crate::mir::Constant::Int(0)),
                        ],
                    },
                    None,
                ),
                StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![
                            Operand::Copy(root_place(3)),
                            Operand::Constant(crate::mir::Constant::Int(0)),
                        ],
                    },
                    None,
                ),
                StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Copy(root_place(7))),
                    None,
                ),
            ],
            bounds_check_contract(),
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("store ptr %scalar, ptr %scalar_ref"));
        assert!(ir.contains("ptrtoint ptr"));
        assert!(ir.contains("sitofp i64"));
        assert!(ir.contains("fptosi double"));
        assert!(ir.contains("inttoptr i64"));
        assert!(ir.contains("array_ref_slice_ptr"));
        assert!(ir.contains("array_ref_data_ptr"));
        assert!(ir.contains("raw_slice_cast"));
        assert!(ir.contains("slice_ref_ptr"));
        assert!(ir.contains("slice_ref_len"));
        assert!(ir.contains("i64 3"));
        assert!(ir.contains("arr_oob"));
        assert!(ir.contains("call void @exit(i32 1)"));
    }

    #[test]
    fn mir_codegen_lowers_enum_discriminant_rvalue() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let enum_id = DefId::new(CrateId(0), LocalDefId(10));
        let enum_ty = Type::Enum {
            id: enum_id,
            args: vec![],
        };
        let mut backend_contract = MirBackendContract::default();
        insert_enum_layout_contract(
            &mut backend_contract,
            enum_id,
            vec![
                MirEnumVariantLayout {
                    name: "None".to_string(),
                    fields: MirVariantLayoutFields::Unit,
                },
                MirEnumVariantLayout {
                    name: "Some".to_string(),
                    fields: MirVariantLayoutFields::Positional(vec![type_id(
                        &type_context,
                        Type::I64,
                    )]),
                },
            ],
        );

        compile_test_function_with_contract(
            &mut codegen,
            &type_context,
            "enum_discriminant",
            Type::I64,
            vec![enum_ty.clone()],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, enum_ty, "value"),
            ],
            vec![StatementData::assign(
                root_place(0),
                Rvalue::Discriminant(root_place(1)),
                None,
            )],
            backend_contract,
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("extractvalue"));
        assert!(ir.contains("discriminant"));
    }

    #[test]
    fn mir_codegen_rejects_unsupported_cast_rvalue() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let function_id = MirFunctionId::Instance(InstanceId(0));
        let ret_type = type_id(&type_context, Type::I64);
        let function = MirFunction {
            id: function_id.clone(),
            name: "bad_cast".to_string(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Cast(Operand::Constant(crate::mir::Constant::Unit), ret_type),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
            local_decls: vec![local_decl(&type_context, Type::I64, "return_place")],
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: Default::default(),
        };
        let mir = MirProgram {
            functions: BTreeMap::from([(function_id.clone(), function)]),
            type_context: type_context.borrow().clone(),
            backend_contract: local_body_contract(function_id, "bad_cast", ret_type),
        };

        let error = codegen.compile_program_from_mir(&mir).unwrap_err();
        assert!(error.message.contains("Unsupported MIR cast"));
    }

    #[test]
    fn mir_codegen_rejects_invalid_fat_pointer_casts() {
        let slice_ref_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Slice(Box::new(Type::I64))),
        };
        let raw_slice_ptr_ty = Type::Pointer(Box::new(Type::Slice(Box::new(Type::I64))));
        let raw_str_ptr_ty = Type::Pointer(Box::new(Type::Str));
        let raw_i64_ptr_ty = Type::Pointer(Box::new(Type::I64));

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let fat_ref_to_thin_ptr = catch_unwind(AssertUnwindSafe(|| {
            compile_test_function_result(
                &mut codegen,
                &type_context,
                "fat_ref_to_thin_ptr",
                raw_i64_ptr_ty.clone(),
                vec![slice_ref_ty.clone()],
                vec![
                    local_decl(&type_context, raw_i64_ptr_ty.clone(), "return_place"),
                    local_decl(&type_context, slice_ref_ty.clone(), "slice_ref"),
                ],
                vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Cast(
                        Operand::Copy(root_place(1)),
                        type_id(&type_context, raw_i64_ptr_ty.clone()),
                    ),
                    None,
                )],
            )
        }));
        assert!(fat_ref_to_thin_ptr.is_ok());
        let fat_ref_to_thin_ptr = fat_ref_to_thin_ptr.unwrap().unwrap_err();
        assert!(fat_ref_to_thin_ptr
            .message
            .contains("Cannot cast fat MIR pointer"));

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let fat_raw_ptr_to_int = catch_unwind(AssertUnwindSafe(|| {
            compile_test_function_result(
                &mut codegen,
                &type_context,
                "fat_raw_ptr_to_int",
                Type::I64,
                vec![raw_slice_ptr_ty.clone()],
                vec![
                    local_decl(&type_context, Type::I64, "return_place"),
                    local_decl(&type_context, raw_slice_ptr_ty.clone(), "raw_slice_ptr"),
                ],
                vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Cast(
                        Operand::Copy(root_place(1)),
                        type_id(&type_context, Type::I64),
                    ),
                    None,
                )],
            )
        }));
        assert!(fat_raw_ptr_to_int.is_ok());
        let fat_raw_ptr_to_int = fat_raw_ptr_to_int.unwrap().unwrap_err();
        assert!(fat_raw_ptr_to_int
            .message
            .contains("Cannot cast fat MIR pointer"));

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let int_to_fat_raw_ptr = catch_unwind(AssertUnwindSafe(|| {
            compile_test_function_result(
                &mut codegen,
                &type_context,
                "int_to_fat_raw_ptr",
                raw_str_ptr_ty.clone(),
                vec![Type::I64],
                vec![
                    local_decl(&type_context, raw_str_ptr_ty.clone(), "return_place"),
                    local_decl(&type_context, Type::I64, "bits"),
                ],
                vec![StatementData::assign(
                    root_place(0),
                    Rvalue::Cast(
                        Operand::Copy(root_place(1)),
                        type_id(&type_context, raw_str_ptr_ty.clone()),
                    ),
                    None,
                )],
            )
        }));
        assert!(int_to_fat_raw_ptr.is_ok());
        let int_to_fat_raw_ptr = int_to_fat_raw_ptr.unwrap().unwrap_err();
        assert!(int_to_fat_raw_ptr
            .message
            .contains("Cannot cast integer to fat MIR pointer"));
    }

    #[test]
    fn mir_codegen_lowers_bounds_check_assertion() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());

        compile_test_function_with_contract(
            &mut codegen,
            &type_context,
            "check_bounds",
            Type::I64,
            vec![],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, Type::Array(Box::new(Type::I64), 4), "arr"),
            ],
            vec![
                StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![
                            Operand::Copy(root_place(1)),
                            Operand::Constant(crate::mir::Constant::Int(2)),
                        ],
                    },
                    None,
                ),
                StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(0))),
                    None,
                ),
            ],
            bounds_check_contract(),
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("arr_oob"));
        assert!(ir.contains("oob_exit"));
    }

    #[test]
    fn mir_codegen_lowers_slice_bounds_check_assertion() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let slice_ty = Type::Slice(Box::new(Type::I64));

        compile_test_function_with_contract(
            &mut codegen,
            &type_context,
            "check_slice_bounds",
            Type::I64,
            vec![slice_ty.clone()],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, slice_ty, "slice"),
            ],
            vec![
                StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![
                            Operand::Copy(root_place(1)),
                            Operand::Constant(crate::mir::Constant::Int(0)),
                        ],
                    },
                    None,
                ),
                StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(0))),
                    None,
                ),
            ],
            bounds_check_contract(),
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("slice_len"));
        assert!(ir.contains("arr_oob"));
        assert!(ir.contains("oob_exit"));
    }

    #[test]
    fn mir_codegen_lowers_slice_reference_bounds_check_assertion() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let slice_ref_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Slice(Box::new(Type::I64))),
        };

        compile_test_function_with_contract(
            &mut codegen,
            &type_context,
            "check_slice_ref_bounds",
            Type::I64,
            vec![slice_ref_ty.clone()],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, slice_ref_ty, "slice_ref"),
            ],
            vec![
                StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![
                            Operand::Copy(root_place(1)),
                            Operand::Constant(crate::mir::Constant::Int(0)),
                        ],
                    },
                    None,
                ),
                StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(0))),
                    None,
                ),
            ],
            bounds_check_contract(),
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("slice_ref_len"));
        assert!(ir.contains("arr_oob"));
        assert!(ir.contains("oob_exit"));
    }

    #[test]
    fn mir_codegen_lowers_string_reference_bounds_check_assertion() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let type_context = RefCell::new(TypeContext::new());
        let str_ref_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        };

        compile_test_function_with_contract(
            &mut codegen,
            &type_context,
            "check_str_ref_bounds",
            Type::I64,
            vec![str_ref_ty.clone()],
            vec![
                local_decl(&type_context, Type::I64, "return_place"),
                local_decl(&type_context, str_ref_ty, "str_ref"),
            ],
            vec![
                StatementData::assert(
                    MirAssert {
                        kind: MirAssertKind::BoundsCheck,
                        operands: vec![
                            Operand::Copy(root_place(1)),
                            Operand::Constant(crate::mir::Constant::Int(0)),
                        ],
                    },
                    None,
                ),
                StatementData::assign(
                    root_place(0),
                    Rvalue::Use(Operand::Constant(crate::mir::Constant::Int(0))),
                    None,
                ),
            ],
            bounds_check_contract(),
        );

        let ir = codegen.get_ir();
        assert!(ir.contains("slice_ref_len"));
        assert!(ir.contains("arr_oob"));
        assert!(ir.contains("oob_exit"));
    }
}
