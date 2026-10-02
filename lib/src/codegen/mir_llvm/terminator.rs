use inkwell::basic_block::BasicBlock as LlvmBasicBlock;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum};
use inkwell::AddressSpace;
use inkwell::IntPredicate;

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{
    BasicBlockId, Constant, MirCallable, MirCallableKey, MirFunction, MirIntrinsicId, MirParamAbi,
    MirPassMode, Operand, Place, Terminator,
};
use crate::types::Type;

use super::MirFunctionContext;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn compile_mir_terminator(
        &mut self,
        function: &MirFunction,
        terminator: &Terminator,
        ctx: &MirFunctionContext<'ctx>,
    ) -> Result<(), CodegenError> {
        match terminator {
            Terminator::Unreachable { .. } => self
                .builder
                .build_unreachable()
                .map(|_| ())
                .map_err(|e| CodegenError::from(e.to_string())),
            Terminator::Return | Terminator::ReturnWithOrigin { .. } => {
                self.compile_mir_return(ctx)
            }
            Terminator::Goto(target) => self
                .builder
                .build_unconditional_branch(self.mir_target_block(ctx, *target, "goto")?)
                .map(|_| ())
                .map_err(|e| CodegenError::from(format!("Failed to build MIR goto: {}", e))),
            Terminator::GotoWithOrigin { target, .. } => self
                .builder
                .build_unconditional_branch(self.mir_target_block(ctx, *target, "goto")?)
                .map(|_| ())
                .map_err(|e| CodegenError::from(format!("Failed to build MIR goto: {}", e))),
            Terminator::SwitchInt {
                discr,
                targets,
                otherwise,
            } => {
                let discr = self.compile_mir_operand(function, ctx, discr)?;
                let BasicValueEnum::IntValue(discr) = discr else {
                    return Err(CodegenError::from(
                        "MIR SwitchInt discriminant must lower to an integer",
                    ));
                };

                for (index, (value, target)) in targets.iter().enumerate() {
                    let next_block = self
                        .context
                        .append_basic_block(ctx.function, &format!("switch.next{}", index));
                    let expected = discr.get_type().const_int(*value as u64, true);
                    let condition = self
                        .builder
                        .build_int_compare(IntPredicate::EQ, discr, expected, "switch_eq")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to build MIR switch compare: {}", e))
                        })?;

                    let target_block = self.mir_target_block(ctx, *target, "switch target")?;
                    self.builder
                        .build_conditional_branch(condition, target_block, next_block)
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to build MIR switch branch: {}", e))
                        })?;
                    self.builder.position_at_end(next_block);
                }

                self.builder
                    .build_unconditional_branch(self.mir_target_block(
                        ctx,
                        *otherwise,
                        "switch otherwise",
                    )?)
                    .map(|_| ())
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to build MIR switch default: {}", e))
                    })
            }
            Terminator::SwitchIntWithOrigin {
                discr,
                targets,
                otherwise,
                ..
            } => {
                let discr = self.compile_mir_operand(function, ctx, discr)?;
                let BasicValueEnum::IntValue(discr) = discr else {
                    return Err(CodegenError::from(
                        "MIR SwitchInt discriminant must lower to an integer",
                    ));
                };

                for (index, (value, target)) in targets.iter().enumerate() {
                    let next_block = self
                        .context
                        .append_basic_block(ctx.function, &format!("switch.next{}", index));
                    let expected = discr.get_type().const_int(*value as u64, true);
                    let condition = self
                        .builder
                        .build_int_compare(IntPredicate::EQ, discr, expected, "switch_eq")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to build MIR switch compare: {}", e))
                        })?;
                    let target_block = self.mir_target_block(ctx, *target, "switch target")?;
                    self.builder
                        .build_conditional_branch(condition, target_block, next_block)
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to build MIR switch branch: {}", e))
                        })?;
                    self.builder.position_at_end(next_block);
                }

                self.builder
                    .build_unconditional_branch(self.mir_target_block(
                        ctx,
                        *otherwise,
                        "switch otherwise",
                    )?)
                    .map(|_| ())
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to build MIR switch default: {}", e))
                    })
            }
            Terminator::Drop { place, target } => {
                self.compile_mir_drop_terminator(function, ctx, place, *target)
            }
            Terminator::DropWithOrigin { place, target, .. } => {
                self.compile_mir_drop_terminator(function, ctx, place, *target)
            }
            Terminator::Call {
                func,
                args,
                destination,
                target,
                ..
            } => self.compile_mir_call_terminator(function, ctx, func, args, destination, *target),
        }
    }

    fn compile_mir_drop_terminator(
        &mut self,
        function: &MirFunction,
        ctx: &MirFunctionContext<'ctx>,
        place: &Place,
        target: BasicBlockId,
    ) -> Result<(), CodegenError> {
        let skip_user_drop = function
            .local_decls
            .get(place.local.0)
            .is_some_and(|local| {
                place.projection.is_empty() && local.source == crate::mir::LocalSource::ReturnPlace
            });
        if skip_user_drop {
            return self
                .builder
                .build_unconditional_branch(self.mir_target_block(ctx, target, "drop")?)
                .map(|_| ())
                .map_err(|e| CodegenError::from(format!("Failed to build MIR drop: {}", e)));
        }

        let (_, place_ty) = self.compile_mir_place_local_id(ctx, place)?;
        if !Self::mir_place_is_projected_return_place(function, place)
            && self.mir_drop_without_glue_is_noop(place_ty)
        {
            return self
                .builder
                .build_unconditional_branch(self.mir_target_block(ctx, target, "drop")?)
                .map(|_| ())
                .map_err(|e| CodegenError::from(format!("Failed to build MIR drop: {}", e)));
        }
        let symbol = self
            .resolve_mir_drop_glue_symbol(place_ty)
            .map_err(|error| error.with_note(format!("MIR function '{}'", function.name)))?;
        let callee = self
            .functions
            .get(&symbol)
            .copied()
            .or_else(|| self.module.get_function(&symbol));
        let callee = callee.ok_or_else(|| {
            CodegenError::backend_contract(format!(
                "Missing drop glue symbol '{}' for MIR function '{}'",
                symbol, function.name
            ))
        })?;
        let arg = if let Some(key) = self.drop_glue_callables_by_type.get(&place_ty) {
            let param = self
                .callable_signatures_by_key
                .get(key)
                .and_then(|signature| signature.params.first())
                .cloned()
                .ok_or_else(|| {
                    CodegenError::backend_contract(
                        "Drop glue callable is missing its receiver parameter",
                    )
                })?;
            self.compile_mir_place_as_param(ctx, place, &param)?
        } else {
            BasicMetadataValueEnum::from(self.compile_mir_place_value(ctx, place)?)
        };
        self.builder
            .build_call(callee, &[arg], "mir_drop")
            .map_err(|e| CodegenError::from(format!("Failed to build MIR drop call: {}", e)))?;

        self.builder
            .build_unconditional_branch(self.mir_target_block(ctx, target, "drop")?)
            .map(|_| ())
            .map_err(|e| CodegenError::from(format!("Failed to build MIR drop: {}", e)))
    }

    fn compile_mir_call_terminator(
        &mut self,
        function: &MirFunction,
        ctx: &MirFunctionContext<'ctx>,
        func: &Operand,
        args: &[Operand],
        destination: &Place,
        target: BasicBlockId,
    ) -> Result<(), CodegenError> {
        let (destination_ptr, destination_ty) = self.compile_mir_place_local(ctx, destination)?;
        if let Operand::Constant(Constant::OwnedObjectCall(key)) = func {
            let never = matches!(destination_ty, Type::Never);
            let result = self.compile_owned_object_call(function, ctx, key, args, destination)?;
            if never {
                return self
                    .builder
                    .build_unreachable()
                    .map(|_| ())
                    .map_err(|e| CodegenError::from(e.to_string()));
            }
            if let Some(value) = result {
                self.builder
                    .build_store(destination_ptr, value)
                    .map_err(|e| CodegenError::from(e.to_string()))?;
            }
            return self
                .builder
                .build_unconditional_branch(self.mir_target_block(
                    ctx,
                    target,
                    "owned object call",
                )?)
                .map(|_| ())
                .map_err(|e| CodegenError::from(e.to_string()));
        }
        if let Operand::Constant(Constant::ErasedCall(call)) = func {
            self.compile_erased_call(function, ctx, call, args, destination)?;
            return self
                .builder
                .build_unconditional_branch(self.mir_target_block(ctx, target, "erased call")?)
                .map(|_| ())
                .map_err(|e| CodegenError::from(e.to_string()));
        }
        if let Operand::Constant(Constant::VirtualTarget(selector)) = func {
            let never = self
                .object_schemas
                .get(&selector.object)
                .and_then(|schema| schema.slots.get(selector.slot as usize))
                .is_some_and(|slot| {
                    matches!(
                        self.structural_type_for(slot.signature.ret.semantic_ty),
                        Type::Never
                    )
                });
            let result = self.compile_virtual_call(function, ctx, selector, args)?;
            if never || matches!(destination_ty, Type::Never) {
                return self
                    .builder
                    .build_unreachable()
                    .map(|_| ())
                    .map_err(|e| CodegenError::from(e.to_string()));
            }
            if let Some(value) = result {
                self.builder
                    .build_store(destination_ptr, value)
                    .map_err(|e| CodegenError::from(e.to_string()))?;
            }
            return self
                .builder
                .build_unconditional_branch(self.mir_target_block(ctx, target, "virtual call")?)
                .map(|_| ())
                .map_err(|e| CodegenError::from(e.to_string()));
        }
        let never = if let Operand::Constant(Constant::Callable(callable)) = func {
            self.resolve_mir_callable_signature(callable)
                .is_some_and(|signature| {
                    matches!(
                        self.structural_type_for(signature.ret.semantic_ty),
                        Type::Never
                    )
                })
        } else {
            matches!(self.mir_operand_ty(ctx, func)?, Type::Function { ret, .. } if matches!(ret.as_ref(), Type::Never))
        };
        let result = if let Operand::Constant(Constant::Callable(callable)) = func {
            if let Some(intrinsic) = Self::mir_callable_intrinsic(callable) {
                let (_, destination_ty_id) = self.compile_mir_place_local_id(ctx, destination)?;
                let arg_type_ids = args
                    .iter()
                    .map(|arg| self.mir_operand_ty_id(ctx, arg))
                    .collect::<Result<Vec<_>, _>>()?;
                let type_only_intrinsic = intrinsic.is_type_only()
                    && args
                        .iter()
                        .all(|arg| matches!(arg, Operand::Constant(Constant::TypeId(_))));
                let compiled_args = if type_only_intrinsic {
                    Vec::new()
                } else {
                    args.iter()
                        .map(|arg| self.compile_mir_operand(function, ctx, arg))
                        .collect::<Result<Vec<_>, _>>()?
                };
                let value = self.compile_intrinsic_values_with_type_ids(
                    intrinsic.clone(),
                    &compiled_args,
                    &arg_type_ids,
                    destination_ty_id,
                )?;
                if !matches!(destination_ty, Type::Unit) {
                    let value = value.ok_or_else(|| {
                        CodegenError::from(format!(
                            "MIR intrinsic '{}' returned no value",
                            intrinsic
                        ))
                    })?;
                    let coerced = self.coerce_value(value, self.llvm_type(&destination_ty))?;
                    self.builder
                        .build_store(destination_ptr, coerced)
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to store MIR call result: {}", e))
                        })?;
                }
                self.builder
                    .build_unconditional_branch(self.mir_target_block(ctx, target, "call")?)
                    .map(|_| ())
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to build MIR call branch: {}", e))
                    })?;
                return Ok(());
            } else {
                let symbol = self.resolve_mir_callable_symbol_in_function(callable, function)?;
                let callee = self.functions.get(&symbol).copied().ok_or_else(|| {
                    CodegenError::from(format!(
                        "Unknown MIR callable symbol '{}' in MIR function '{}'",
                        symbol, function.name
                    ))
                })?;
                let signature = self.resolve_mir_callable_signature(callable);
                let compiled_args = self.compile_mir_call_args(
                    function,
                    ctx,
                    args,
                    signature
                        .as_ref()
                        .map(|signature| signature.params.as_slice()),
                )?;
                self.builder
                    .build_call(callee, &compiled_args, "mir_call")
                    .map_err(|e| CodegenError::from(format!("Failed to build MIR call: {}", e)))?
            }
        } else {
            let callable_ty = self.mir_operand_ty(ctx, func)?;
            let Type::Function {
                params: param_types,
                ret: ret_ty,
                ..
            } = callable_ty
            else {
                return Err(CodegenError::layout(format!(
                    "MIR call expected function operand, got {}",
                    self.display_type_for_diagnostic(&callable_ty)
                )));
            };
            let callable_val = self.compile_mir_operand(function, ctx, func)?;
            let expected_param_ids = param_types
                .iter()
                .map(|ty| self.type_context().id_for_type(ty))
                .collect::<Option<Vec<_>>>();
            let expected_params = expected_param_ids.as_ref().map(|ids| {
                ids.iter()
                    .copied()
                    .map(|semantic_ty| MirParamAbi {
                        semantic_ty,
                        pass_mode: MirPassMode::Direct,
                    })
                    .collect::<Vec<_>>()
            });
            let compiled_args =
                self.compile_mir_call_args(function, ctx, args, expected_params.as_deref())?;
            let ptr_ty = self.context.ptr_type(AddressSpace::default());
            let (code_ptr, env_ptr) = if callable_val.is_struct_value() {
                let callable = callable_val.into_struct_value();
                let code_ptr = self
                    .builder
                    .build_extract_value(callable, 0, "callable_code")
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to extract MIR callable code: {}", e))
                    })?
                    .into_pointer_value();
                let env_ptr = self
                    .builder
                    .build_extract_value(callable, 1, "callable_env")
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to extract MIR callable env: {}", e))
                    })?
                    .into_pointer_value();
                (code_ptr, env_ptr)
            } else {
                (callable_val.into_pointer_value(), ptr_ty.const_null())
            };

            let mut all_args = Vec::with_capacity(compiled_args.len() + 1);
            all_args.push(env_ptr.into());
            all_args.extend(compiled_args);
            self.builder
                .build_indirect_call(
                    self.callable_code_type(&param_types, &ret_ty),
                    code_ptr,
                    &all_args,
                    "mir_indirect_call",
                )
                .map_err(|e| {
                    CodegenError::from(format!("Failed to build indirect MIR call: {}", e))
                })?
        };

        if never || matches!(destination_ty, Type::Never) {
            return self
                .builder
                .build_unreachable()
                .map(|_| ())
                .map_err(|e| CodegenError::from(e.to_string()));
        }
        let destination_is_unit = matches!(&destination_ty, Type::Unit)
            || matches!(&destination_ty, Type::Tuple(elements) if elements.is_empty());
        if !destination_is_unit {
            let value = result.try_as_basic_value().left().ok_or_else(|| {
                CodegenError::from("MIR call expected a return value but got void")
            })?;
            let coerced = self.coerce_value(value, self.llvm_type(&destination_ty))?;
            self.builder
                .build_store(destination_ptr, coerced)
                .map_err(|e| {
                    CodegenError::from(format!("Failed to store MIR call result: {}", e))
                })?;
        }

        self.builder
            .build_unconditional_branch(self.mir_target_block(ctx, target, "call")?)
            .map(|_| ())
            .map_err(|e| CodegenError::from(format!("Failed to build MIR call branch: {}", e)))
    }

    pub(super) fn compile_mir_call_args(
        &mut self,
        function: &MirFunction,
        ctx: &MirFunctionContext<'ctx>,
        args: &[Operand],
        expected_params: Option<&[MirParamAbi]>,
    ) -> Result<Vec<BasicMetadataValueEnum<'ctx>>, CodegenError> {
        args.iter()
            .enumerate()
            .map(|(index, arg)| {
                let expected_param = expected_params.and_then(|params| params.get(index));
                let expected_ty =
                    expected_param.map(|param| self.structural_type_for(param.semantic_ty));
                let value = match (arg, expected_ty.as_ref()) {
                    (
                        Operand::Constant(Constant::Callable(callable)),
                        Some(expected_ty @ Type::Function { .. }),
                    ) => self.compile_mir_callable_constant(function, callable, expected_ty)?,
                    _ => {
                        if let Some(ref expected_ty) = expected_ty {
                            if let Some(value) = self.compile_mir_array_ref_to_slice_arg(
                                function,
                                ctx,
                                arg,
                                expected_ty,
                            )? {
                                return Ok(value.into());
                            }
                        }
                        self.compile_mir_operand(function, ctx, arg)?
                    }
                };
                if let Some(param) = expected_param {
                    self.compile_mir_value_as_param(function, ctx, arg, value, param)
                } else {
                    Ok(value.into())
                }
            })
            .collect()
    }

    fn compile_mir_value_as_param(
        &mut self,
        function: &MirFunction,
        ctx: &MirFunctionContext<'ctx>,
        arg: &Operand,
        value: BasicValueEnum<'ctx>,
        param: &MirParamAbi,
    ) -> Result<BasicMetadataValueEnum<'ctx>, CodegenError> {
        match param.pass_mode {
            MirPassMode::Direct | MirPassMode::FatDirect => Ok(value.into()),
            MirPassMode::Pointer => {
                if self.type_id_lowers_to_pointer(param.semantic_ty) {
                    return Ok(value.into());
                }

                match arg {
                    Operand::Copy(place) | Operand::Move(place) => {
                        self.compile_mir_place_as_param(ctx, place, param)
                    }
                    Operand::Constant(_) => {
                        let temp = self
                            .builder
                            .build_alloca(self.llvm_type_id(param.semantic_ty), "mir_pointer_arg")
                            .map_err(|e| {
                                CodegenError::from(format!(
                                    "Failed to allocate MIR pointer ABI argument in '{}': {}",
                                    function.name, e
                                ))
                            })?;
                        let coerced =
                            self.coerce_value(value, self.llvm_type_id(param.semantic_ty))?;
                        self.builder.build_store(temp, coerced).map_err(|e| {
                            CodegenError::from(format!(
                                "Failed to store MIR pointer ABI argument in '{}': {}",
                                function.name, e
                            ))
                        })?;
                        Ok(temp.into())
                    }
                }
            }
        }
    }

    fn compile_mir_place_as_param(
        &mut self,
        ctx: &MirFunctionContext<'ctx>,
        place: &Place,
        param: &MirParamAbi,
    ) -> Result<BasicMetadataValueEnum<'ctx>, CodegenError> {
        match param.pass_mode {
            MirPassMode::Direct | MirPassMode::FatDirect => {
                Ok(self.compile_mir_place_value(ctx, place)?.into())
            }
            MirPassMode::Pointer => {
                if self.type_id_lowers_to_pointer(param.semantic_ty) {
                    Ok(self.compile_mir_place_value(ctx, place)?.into())
                } else {
                    Ok(self.compile_mir_place_address(ctx, place)?.into())
                }
            }
        }
    }

    fn compile_mir_array_ref_to_slice_arg(
        &mut self,
        function: &MirFunction,
        ctx: &MirFunctionContext<'ctx>,
        arg: &Operand,
        expected_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let Type::Slice(expected_elem) = self.resolve_projection_type(expected_ty) else {
            return Ok(None);
        };

        let actual_ty = self.mir_operand_ty(ctx, arg)?;
        let (array_elem, len, array_ptr) = match self.resolve_projection_type(&actual_ty) {
            Type::Reference { inner, .. } => {
                let Type::Array(array_elem, len) = inner.as_ref() else {
                    return Ok(None);
                };
                let array_ptr = self
                    .compile_mir_operand(function, ctx, arg)?
                    .into_pointer_value();
                (array_elem.clone(), *len, array_ptr)
            }
            Type::Array(array_elem, len) => {
                let (Operand::Copy(place) | Operand::Move(place)) = arg else {
                    return Ok(None);
                };
                let array_ptr = self.compile_mir_place_address(ctx, place)?;
                (array_elem, len, array_ptr)
            }
            _ => return Ok(None),
        };

        if array_elem.as_ref() != expected_elem.as_ref() {
            return Ok(None);
        }

        let zero = self.context.i64_type().const_int(0, false);
        let array_ty = self.llvm_type(&Type::Array(array_elem, len));
        let data_ptr = unsafe {
            self.builder
                .build_gep(array_ty, array_ptr, &[zero, zero], "mir_array_slice_arg")
                .map_err(|e| CodegenError::from(format!("Failed MIR array slice arg gep: {}", e)))?
        };

        self.make_fat_slice_value(
            data_ptr,
            self.context.i64_type().const_int(len as u64, false),
        )
        .map(Some)
    }

    fn mir_target_block(
        &self,
        ctx: &MirFunctionContext<'ctx>,
        target: BasicBlockId,
        terminator: &str,
    ) -> Result<LlvmBasicBlock<'ctx>, CodegenError> {
        ctx.blocks.get(target.0).copied().ok_or_else(|| {
            CodegenError::from(format!(
                "invalid MIR basic block target {} for {} terminator; function has {} blocks",
                target.0,
                terminator,
                ctx.blocks.len()
            ))
        })
    }

    fn compile_mir_return(&mut self, ctx: &MirFunctionContext<'ctx>) -> Result<(), CodegenError> {
        if ctx
            .locals
            .first()
            .and_then(|local| local.as_ref())
            .is_some_and(|(_, ty)| matches!(self.structural_type_for(*ty), Type::Never))
        {
            return self
                .builder
                .build_unreachable()
                .map(|_| ())
                .map_err(|e| CodegenError::from(e.to_string()));
        }
        let return_type = ctx.function.get_type().get_return_type();
        if let Some(return_type) = return_type {
            let Some(Some((return_place, return_place_ty))) = ctx.locals.get(0) else {
                return Err(CodegenError::backend_contract(
                    "MIR return local is missing",
                ));
            };
            let value = self
                .builder
                .build_load(
                    self.llvm_type_id(*return_place_ty),
                    *return_place,
                    "return_place",
                )
                .map_err(|e| {
                    CodegenError::from(format!("Failed to load MIR return place: {}", e))
                })?;
            let value = self.coerce_value(value, return_type)?;
            self.builder
                .build_return(Some(&value))
                .map(|_| ())
                .map_err(|e| CodegenError::from(format!("Failed to build MIR return: {}", e)))
        } else {
            self.builder
                .build_return(None)
                .map(|_| ())
                .map_err(|e| CodegenError::from(format!("Failed to build MIR return: {}", e)))
        }
    }

    fn mir_callable_intrinsic(callable: &MirCallable) -> Option<MirIntrinsicId> {
        match callable {
            MirCallable::Resolved(MirCallableKey::Intrinsic(intrinsic)) => Some(intrinsic.clone()),
            _ => None,
        }
    }
}
