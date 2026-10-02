use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{
    MirBackendContract, MirCallable, MirCallableKey, MirCallableKind, MirObjectAdapter,
    MirPassMode, MirPayloadDropChildren, MirPayloadDropPlan,
};
use crate::types::Type;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    fn adapter_target(&self, key: &MirCallableKey) -> Result<FunctionValue<'ctx>, CodegenError> {
        let symbol = self.resolve_mir_callable_symbol(&MirCallable::Resolved(key.clone()))?;
        self.functions
            .get(&symbol)
            .copied()
            .or_else(|| self.module.get_function(&symbol))
            .ok_or_else(|| {
                CodegenError::backend_contract("adapter target has no declared link symbol")
            })
    }

    pub(crate) fn emit_object_adapters(
        &mut self,
        contract: &MirBackendContract,
    ) -> Result<(), CodegenError> {
        for declaration in contract.callables.values() {
            let MirCallableKind::ObjectAdapter(adapter) = &declaration.kind else {
                continue;
            };
            let function = self.adapter_target(&declaration.key)?;
            let block = self.context.append_basic_block(function, "object.adapter");
            self.builder.position_at_end(block);
            let receiver = function
                .get_first_param()
                .ok_or_else(|| CodegenError::backend_contract("adapter receiver missing"))?
                .into_pointer_value();
            let mut environment_to_release = None;
            let call = match adapter {
                MirObjectAdapter::PayloadDrop(plan) => {
                    self.emit_payload_drop_plan(contract, function, receiver, plan)?;
                    self.builder
                        .build_return(None)
                        .map_err(|e| CodegenError::from(e.to_string()))?;
                    continue;
                }
                MirObjectAdapter::ConsumingMethod { concrete, target } => {
                    let by_address = contract
                        .callable(target)
                        .and_then(|target| target.signature.params.first())
                        .is_some_and(|param| {
                            param.pass_mode == MirPassMode::Pointer
                                && !self.type_id_lowers_to_pointer(*concrete)
                        });
                    let value = if by_address {
                        receiver.into()
                    } else {
                        self.builder
                            .build_load(
                                self.llvm_type_id(*concrete),
                                receiver,
                                "object.consume.payload",
                            )
                            .map_err(|e| CodegenError::from(e.to_string()))?
                    };
                    let mut args: Vec<BasicMetadataValueEnum> = vec![value.into()];
                    args.extend(
                        function
                            .get_param_iter()
                            .skip(1)
                            .map(BasicMetadataValueEnum::from),
                    );
                    self.builder
                        .build_call(self.adapter_target(target)?, &args, "object.consume.call")
                        .map_err(|e| CodegenError::from(e.to_string()))?
                }
                MirObjectAdapter::NativeCallable {
                    concrete,
                    release_environment,
                    ..
                } => {
                    let Type::Function { params, ret, .. } = self.structural_type_for(*concrete)
                    else {
                        return Err(CodegenError::backend_contract(
                            "native adapter payload is not callable",
                        ));
                    };
                    let callable = self
                        .builder
                        .build_load(self.llvm_type_id(*concrete), receiver, "object.callable")
                        .map_err(|e| CodegenError::from(e.to_string()))?
                        .into_struct_value();
                    let code = self
                        .builder
                        .build_extract_value(callable, 0, "object.callable.code")
                        .map_err(|e| CodegenError::from(e.to_string()))?
                        .into_pointer_value();
                    let env = self
                        .builder
                        .build_extract_value(callable, 1, "object.callable.environment")
                        .map_err(|e| CodegenError::from(e.to_string()))?;
                    if *release_environment {
                        environment_to_release = Some(env.into_pointer_value());
                    }
                    let mut args: Vec<BasicMetadataValueEnum> = vec![env.into()];
                    if !params.is_empty() {
                        let pack = function.get_nth_param(1).ok_or_else(|| {
                            CodegenError::backend_contract("native argument pack missing")
                        })?;
                        if params.len() == 1 {
                            args.push(pack.into());
                        } else {
                            for index in 0..params.len() {
                                args.push(
                                    self.builder
                                        .build_extract_value(
                                            pack.into_struct_value(),
                                            index as u32,
                                            "object.callable.argument",
                                        )
                                        .map_err(|e| CodegenError::from(e.to_string()))?
                                        .into(),
                                );
                            }
                        }
                    }
                    self.builder
                        .build_indirect_call(
                            self.callable_code_type(&params, &ret),
                            code,
                            &args,
                            "object.callable.invoke",
                        )
                        .map_err(|e| CodegenError::from(e.to_string()))?
                }
            };
            let ret = self.structural_type_for(declaration.signature.ret.semantic_ty);
            if !matches!(ret, Type::Never) {
                if let Some(environment) = environment_to_release {
                    self.release_object_closure_environment(environment)?;
                }
            }
            if matches!(ret, Type::Never) {
                self.builder
                    .build_unreachable()
                    .map_err(|e| CodegenError::from(e.to_string()))?;
            } else if let Some(value) = call.try_as_basic_value().left() {
                self.builder
                    .build_return(Some(&value))
                    .map_err(|e| CodegenError::from(e.to_string()))?;
            } else {
                self.builder
                    .build_return(None)
                    .map_err(|e| CodegenError::from(e.to_string()))?;
            }
        }
        Ok(())
    }

    pub(super) fn emit_payload_drop_plan(
        &mut self,
        contract: &MirBackendContract,
        function: FunctionValue<'ctx>,
        address: PointerValue<'ctx>,
        plan: &MirPayloadDropPlan,
    ) -> Result<(), CodegenError> {
        if !plan.has_drop() {
            return Ok(());
        }
        if let Some(key) = &plan.user_drop {
            let declaration = contract
                .callable(key)
                .ok_or_else(|| CodegenError::backend_contract("payload user destructor missing"))?;
            let param = declaration.signature.params.first().ok_or_else(|| {
                CodegenError::backend_contract("payload destructor receiver missing")
            })?;
            let receiver: BasicValueEnum = if param.pass_mode == MirPassMode::Pointer {
                address.into()
            } else {
                self.builder
                    .build_load(self.llvm_type_id(plan.ty), address, "payload.drop.value")
                    .map_err(|e| CodegenError::from(e.to_string()))?
            };
            self.builder
                .build_call(
                    self.adapter_target(key)?,
                    &[receiver.into()],
                    "payload.user.drop",
                )
                .map_err(|e| CodegenError::from(e.to_string()))?;
        }
        match &plan.children {
            MirPayloadDropChildren::None => {}
            MirPayloadDropChildren::Fields(fields) => {
                for (index, field) in fields.iter().enumerate().rev() {
                    if !field.has_drop() {
                        continue;
                    }
                    let field_ptr = self
                        .builder
                        .build_struct_gep(
                            self.llvm_type_id(plan.ty).into_struct_type(),
                            address,
                            index as u32,
                            "payload.drop.field",
                        )
                        .map_err(|e| CodegenError::from(e.to_string()))?;
                    self.emit_payload_drop_plan(contract, function, field_ptr, field)?;
                }
            }
            MirPayloadDropChildren::Array { len, element } => {
                if !element.has_drop() {
                    return Ok(());
                }
                for index in (0..*len).rev() {
                    let i64 = self.context.i64_type();
                    let pointer = unsafe {
                        self.builder.build_gep(
                            self.llvm_type_id(plan.ty),
                            address,
                            &[i64.const_zero(), i64.const_int(index as u64, false)],
                            "payload.drop.element",
                        )
                    }
                    .map_err(|e| CodegenError::from(e.to_string()))?;
                    self.emit_payload_drop_plan(contract, function, pointer, element)?;
                }
            }
            MirPayloadDropChildren::Variants(variants) => {
                if !variants.iter().any(MirPayloadDropPlan::has_drop) {
                    return Ok(());
                }
                let layout = self.llvm_type_id(plan.ty).into_struct_type();
                let discr_ptr = self
                    .builder
                    .build_struct_gep(layout, address, 0, "payload.drop.tag")
                    .map_err(|e| CodegenError::from(e.to_string()))?;
                let discr = self
                    .builder
                    .build_load(
                        self.context.i32_type(),
                        discr_ptr,
                        "payload.drop.discriminant",
                    )
                    .map_err(|e| CodegenError::from(e.to_string()))?
                    .into_int_value();
                let done = self
                    .context
                    .append_basic_block(function, "payload.drop.done");
                let invalid = self
                    .context
                    .append_basic_block(function, "payload.drop.invalid");
                let blocks = variants
                    .iter()
                    .enumerate()
                    .map(|(index, _)| {
                        (
                            self.context.i32_type().const_int(index as u64, false),
                            self.context
                                .append_basic_block(function, "payload.drop.variant"),
                        )
                    })
                    .collect::<Vec<_>>();
                self.builder
                    .build_switch(discr, invalid, &blocks)
                    .map_err(|e| CodegenError::from(e.to_string()))?;
                self.builder.position_at_end(invalid);
                self.builder
                    .build_unreachable()
                    .map_err(|e| CodegenError::from(e.to_string()))?;
                for (index, (variant, (_, block))) in variants.iter().zip(blocks).enumerate() {
                    self.builder.position_at_end(block);
                    let payload = self
                        .builder
                        .build_struct_gep(
                            layout,
                            address,
                            index as u32 + 1,
                            "payload.drop.variant.data",
                        )
                        .map_err(|e| CodegenError::from(e.to_string()))?;
                    self.emit_payload_drop_plan(contract, function, payload, variant)?;
                    self.builder
                        .build_unconditional_branch(done)
                        .map_err(|e| CodegenError::from(e.to_string()))?;
                }
                self.builder.position_at_end(done);
            }
            MirPayloadDropChildren::ClosureEnvironment { fields, release } => {
                let value = self
                    .builder
                    .build_load(self.llvm_type_id(plan.ty), address, "payload.closure")
                    .map_err(|e| CodegenError::from(e.to_string()))?
                    .into_struct_value();
                let environment = self
                    .builder
                    .build_extract_value(value, 1, "payload.closure.environment")
                    .map_err(|e| CodegenError::from(e.to_string()))?
                    .into_pointer_value();
                let layout = self.context.struct_type(
                    &fields
                        .iter()
                        .map(|field| self.llvm_type_id(field.ty))
                        .collect::<Vec<_>>(),
                    false,
                );
                for (index, field) in fields.iter().enumerate().rev() {
                    if !field.has_drop() {
                        continue;
                    }
                    let address = self
                        .builder
                        .build_struct_gep(
                            layout,
                            environment,
                            index as u32,
                            "payload.closure.capture",
                        )
                        .map_err(|e| CodegenError::from(e.to_string()))?;
                    self.emit_payload_drop_plan(contract, function, address, field)?;
                }
                if *release {
                    self.release_object_closure_environment(environment)?;
                }
            }
        }
        Ok(())
    }

    fn release_object_closure_environment(
        &self,
        environment: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let free = self.functions.get("free").copied().ok_or_else(|| {
            CodegenError::backend_contract("closure environment destruction requires HeapFree")
        })?;
        self.builder
            .build_call(free, &[environment.into()], "payload.closure.release")
            .map(|_| ())
            .map_err(|e| CodegenError::from(e.to_string()))
    }
}
