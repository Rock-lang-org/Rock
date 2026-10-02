use super::MirFunctionContext;
use crate::codegen::{CodeGen, CodegenError};
use crate::mir::*;
use inkwell::types::{BasicType, FunctionType, StructType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, IntValue, PointerValue};
use inkwell::{module::Linkage, AddressSpace};
use std::collections::BTreeMap;

impl<'ctx> CodeGen<'ctx> {
    fn erased_descriptor_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        self.context.struct_type(
            &[
                word.into(),
                word.into(),
                ptr.into(),
                word.into(),
                ptr.into(),
            ],
            false,
        )
    }
    fn erased_function_type(&self, signature: &MirErasedSignature) -> FunctionType<'ctx> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        self.context.void_type().fn_type(
            &vec![
                pointer.into();
                1 + signature.layouts.len()
                    + signature.dictionaries.len()
                    + signature.params.len()
            ],
            false,
        )
    }
    fn erased_error(message: impl Into<String>) -> CodegenError {
        CodegenError::backend_contract(message)
    }
    fn erased_layout(
        &self,
        ty: &MirErasedType,
        layouts: &BTreeMap<MirErasedType, PointerValue<'ctx>>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        if let Some(layout) = layouts.get(ty) {
            return Ok(*layout);
        }
        if let MirErasedType::Concrete(id) = ty {
            return self
                .erased_descriptors
                .get(id)
                .map(|global| global.as_pointer_value())
                .ok_or_else(|| {
                    Self::erased_error("concrete erased value is missing its descriptor")
                });
        }
        Err(Self::erased_error(
            "erased value requires unavailable layout/application evidence",
        ))
    }
    fn erased_descriptor_word(
        &self,
        descriptor: PointerValue<'ctx>,
        field: u32,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let address = self
            .builder
            .build_struct_gep(
                self.erased_descriptor_type(),
                descriptor,
                field,
                "descriptor.field",
            )
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder
            .build_load(self.context.i64_type(), address, "descriptor.word")
            .map(|v| v.into_int_value())
            .map_err(|e| Self::erased_error(e.to_string()))
    }
    fn erased_descriptor_pointer(
        &self,
        descriptor: PointerValue<'ctx>,
        field: u32,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let address = self
            .builder
            .build_struct_gep(
                self.erased_descriptor_type(),
                descriptor,
                field,
                "descriptor.field",
            )
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                address,
                "descriptor.pointer",
            )
            .map(|v| v.into_pointer_value())
            .map_err(|e| Self::erased_error(e.to_string()))
    }
    fn erased_field_address(
        &self,
        value: PointerValue<'ctx>,
        descriptor: PointerValue<'ctx>,
        index: u32,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let offsets = self.erased_descriptor_pointer(descriptor, 4)?;
        let offset_pointer = unsafe {
            self.builder.build_gep(
                self.context.i64_type(),
                offsets,
                &[self.context.i64_type().const_int(index as u64, false)],
                "descriptor.offset.address",
            )
        }
        .map_err(|e| Self::erased_error(e.to_string()))?;
        let offset = self
            .builder
            .build_load(self.context.i64_type(), offset_pointer, "descriptor.offset")
            .map_err(|e| Self::erased_error(e.to_string()))?
            .into_int_value();
        unsafe {
            self.builder
                .build_gep(self.context.i8_type(), value, &[offset], "erased.field")
        }
        .map_err(|e| Self::erased_error(e.to_string()))
    }
    fn allocate_erased_local(
        &self,
        descriptor: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let size = self.erased_descriptor_word(descriptor, 0)?;
        let align = self.erased_descriptor_word(descriptor, 1)?;
        let one = self.context.i64_type().const_int(1, false);
        let padding = self
            .builder
            .build_int_sub(align, one, "erased.alignment.padding")
            .map_err(|e| Self::erased_error(e.to_string()))?;
        let bytes = self
            .builder
            .build_int_add(size, padding, "erased.storage.bytes")
            .map_err(|e| Self::erased_error(e.to_string()))?;
        let raw = self
            .builder
            .build_array_alloca(self.context.i8_type(), bytes, "erased.storage")
            .map_err(|e| Self::erased_error(e.to_string()))?;
        let address = self
            .builder
            .build_ptr_to_int(raw, self.context.i64_type(), "erased.storage.address")
            .map_err(|e| Self::erased_error(e.to_string()))?;
        let padded = self
            .builder
            .build_int_add(address, padding, "erased.storage.padded")
            .map_err(|e| Self::erased_error(e.to_string()))?;
        let mask = self
            .builder
            .build_not(padding, "erased.alignment.mask")
            .map_err(|e| Self::erased_error(e.to_string()))?;
        let aligned = self
            .builder
            .build_and(padded, mask, "erased.storage.aligned")
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder
            .build_int_to_ptr(
                aligned,
                self.context.ptr_type(AddressSpace::default()),
                "erased.local",
            )
            .map_err(|e| Self::erased_error(e.to_string()))
    }
    fn move_erased_bytes(
        &self,
        destination: PointerValue<'ctx>,
        source: PointerValue<'ctx>,
        descriptor: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let size = self.erased_descriptor_word(descriptor, 0)?;
        // Alignment 1 is a conservative intrinsic attribute, not the storage
        // layout: allocation used the descriptor's actual target alignment.
        self.builder
            .build_memmove(destination, 1, source, 1, size)
            .map(|_| ())
            .map_err(|e| Self::erased_error(e.to_string()))
    }
    fn drop_erased_value(
        &self,
        address: PointerValue<'ctx>,
        descriptor: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let drop = self.erased_descriptor_pointer(descriptor, 2)?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.builder
            .build_indirect_call(
                self.context
                    .void_type()
                    .fn_type(&[ptr.into(), ptr.into()], false),
                drop,
                &[address.into(), descriptor.into()],
                "erased.drop",
            )
            .map(|_| ())
            .map_err(|e| Self::erased_error(e.to_string()))
    }

    pub(crate) fn emit_erased_program(
        &mut self,
        contract: &MirBackendContract,
    ) -> Result<(), CodegenError> {
        self.erased_contract = contract.erased.clone();
        let ptr = self.context.ptr_type(AddressSpace::default());
        for (key, declaration) in &contract.erased.functions {
            let function = self.module.add_function(
                &declaration.symbol,
                self.erased_function_type(&declaration.signature),
                None,
            );
            if declaration.linkage == MirLinkage::Internal {
                function.set_linkage(Linkage::Internal);
            }
            self.erased_functions.insert(key.clone(), function);
        }
        for (id, descriptor) in &contract.erased.descriptors {
            let drop = self.module.add_function(
                &format!("rock.descriptor.drop.{}", id.0),
                self.context
                    .void_type()
                    .fn_type(&[ptr.into(), ptr.into()], false),
                Some(Linkage::Internal),
            );
            self.builder
                .position_at_end(self.context.append_basic_block(drop, "entry"));
            self.emit_payload_drop_plan(
                contract,
                drop,
                drop.get_first_param()
                    .ok_or_else(|| Self::erased_error("descriptor destructor parameter missing"))?
                    .into_pointer_value(),
                &descriptor.drop,
            )?;
            self.builder
                .build_return(None)
                .map_err(|e| Self::erased_error(e.to_string()))?;
            let value_type = self.llvm_type_id(*id);
            let word = self.context.i64_type();
            let offsets = descriptor
                .fields
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    unsafe {
                        ptr.const_null().const_gep(
                            value_type,
                            &[
                                self.context.i32_type().const_zero(),
                                self.context.i32_type().const_int(index as u64, false),
                            ],
                        )
                    }
                    .const_to_int(word)
                })
                .collect::<Vec<_>>();
            let offsets_value = word.const_array(&offsets);
            let offsets_global = self.module.add_global(
                offsets_value.get_type(),
                None,
                &format!("rock.descriptor.offsets.{}", id.0),
            );
            offsets_global.set_initializer(&offsets_value);
            offsets_global.set_constant(true);
            offsets_global.set_linkage(Linkage::Private);
            let initializer = self.erased_descriptor_type().const_named_struct(&[
                value_type
                    .size_of()
                    .ok_or_else(|| Self::erased_error("descriptor requires a sized runtime type"))?
                    .into(),
                self.context
                    .struct_type(&[value_type], false)
                    .get_alignment()
                    .into(),
                drop.as_global_value().as_pointer_value().into(),
                word.const_int(descriptor.fields.len() as u64, false).into(),
                offsets_global.as_pointer_value().into(),
            ]);
            let global = self.module.add_global(
                self.erased_descriptor_type(),
                None,
                &format!("rock.descriptor.{}", id.0),
            );
            global.set_initializer(&initializer);
            global.set_constant(true);
            global.set_linkage(Linkage::Private);
            self.erased_descriptors.insert(*id, global);
        }
        self.emit_erased_dictionaries(contract)?;
        for declaration in contract.erased.functions.values() {
            self.emit_erased_body(contract, declaration)?;
        }
        Ok(())
    }

    fn emit_erased_dictionaries(
        &mut self,
        contract: &MirBackendContract,
    ) -> Result<(), CodegenError> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        for (id, dictionary) in &contract.erased.dictionaries {
            let mut entries = Vec::new();
            for (index, member) in dictionary.members.iter().enumerate() {
                let target = contract
                    .callable(&member.target)
                    .ok_or_else(|| Self::erased_error("dictionary entry callable missing"))?;
                let symbol = self
                    .resolve_mir_callable_symbol(&MirCallable::Resolved(member.target.clone()))?;
                let callee = self
                    .functions
                    .get(&symbol)
                    .copied()
                    .or_else(|| self.module.get_function(&symbol))
                    .ok_or_else(|| Self::erased_error("dictionary entry link symbol missing"))?;
                let entry = self.module.add_function(
                    &format!("rock.dictionary.{}.{}", id.0, index),
                    self.context
                        .void_type()
                        .fn_type(&vec![ptr.into(); target.signature.params.len() + 1], false),
                    Some(Linkage::Internal),
                );
                self.builder
                    .position_at_end(self.context.append_basic_block(entry, "entry"));
                let out = entry
                    .get_first_param()
                    .ok_or_else(|| Self::erased_error("dictionary out parameter missing"))?
                    .into_pointer_value();
                let mut args = Vec::new();
                for (parameter, abi) in target.signature.params.iter().enumerate() {
                    let address = entry
                        .get_nth_param(parameter as u32 + 1)
                        .ok_or_else(|| Self::erased_error("dictionary argument missing"))?
                        .into_pointer_value();
                    let receiver_address = parameter == 0
                        && (member.receiver != crate::types::ReceiverMode::Move
                            || matches!(target.kind, MirCallableKind::ObjectAdapter(_)));
                    let by_address = abi.pass_mode == MirPassMode::Pointer
                        && !self.type_id_lowers_to_pointer(abi.semantic_ty);
                    let value: BasicValueEnum = if receiver_address || by_address {
                        address.into()
                    } else {
                        self.builder
                            .build_load(
                                self.llvm_type_id(abi.semantic_ty),
                                address,
                                "dictionary.value",
                            )
                            .map_err(|e| Self::erased_error(e.to_string()))?
                    };
                    args.push(value.into());
                }
                let result = self
                    .builder
                    .build_call(callee, &args, "dictionary.invoke")
                    .map_err(|e| Self::erased_error(e.to_string()))?;
                if let Some(value) = result.try_as_basic_value().left() {
                    self.builder
                        .build_store(out, value)
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                }
                if matches!(
                    self.structural_type_for(target.signature.ret.semantic_ty),
                    crate::types::Type::Never
                ) {
                    self.builder
                        .build_unreachable()
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                } else {
                    self.builder
                        .build_return(None)
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                }
                entries.push(entry.as_global_value().as_pointer_value());
            }
            let value = ptr.const_array(&entries);
            let global = self.module.add_global(
                value.get_type(),
                None,
                &format!("rock.dictionary.{}", id.0),
            );
            global.set_initializer(&value);
            global.set_constant(true);
            global.set_linkage(Linkage::Private);
            self.erased_dictionaries.insert(*id, global);
        }
        Ok(())
    }

    fn erased_place_address(
        &self,
        body: &MirErasedFunction,
        locals: &[PointerValue<'ctx>],
        place: &MirErasedPlace,
        layouts: &BTreeMap<MirErasedType, PointerValue<'ctx>>,
        contract: &MirBackendContract,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let mut address = *locals
            .get(place.local.0 as usize)
            .ok_or_else(|| Self::erased_error("erased local missing"))?;
        let mut prefix = MirErasedPlace::local(place.local);
        for projection in &place.projection {
            let ty = erased_place_type(body, &prefix, contract, self.type_context())
                .map_err(|e| Self::erased_error(format!("{e:?}")))?;
            address = match projection {
                MirErasedProjection::Deref => self
                    .builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        address,
                        "erased.deref",
                    )
                    .map_err(|e| Self::erased_error(e.to_string()))?
                    .into_pointer_value(),
                MirErasedProjection::Field(index) => {
                    self.erased_field_address(address, self.erased_layout(&ty, layouts)?, *index)?
                }
            };
            prefix.projection.push(projection.clone());
        }
        Ok(address)
    }

    fn erased_argument_address(
        &self,
        body: &MirErasedFunction,
        locals: &[PointerValue<'ctx>],
        operand: &MirErasedOperand,
        layouts: &BTreeMap<MirErasedType, PointerValue<'ctx>>,
        contract: &MirBackendContract,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let place = match operand {
            MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
        };
        let address = self.erased_place_address(body, locals, place, layouts, contract)?;
        if matches!(operand, MirErasedOperand::Move(_)) {
            return Ok(address);
        }
        let ty = erased_place_type(body, place, contract, self.type_context())
            .map_err(|e| Self::erased_error(format!("{e:?}")))?;
        let descriptor = self.erased_layout(&ty, layouts)?;
        let copy = self.allocate_erased_local(descriptor)?;
        self.move_erased_bytes(copy, address, descriptor)?;
        Ok(copy)
    }

    fn emit_erased_body(
        &mut self,
        contract: &MirBackendContract,
        body: &MirErasedFunction,
    ) -> Result<(), CodegenError> {
        let Some(code) = &body.body else {
            return Ok(());
        };
        let function = self.erased_functions[&body.key];
        self.builder
            .position_at_end(self.context.append_basic_block(function, "entry"));
        let out = function
            .get_first_param()
            .ok_or_else(|| Self::erased_error("erased out parameter missing"))?
            .into_pointer_value();
        let mut layouts = BTreeMap::new();
        for (index, ty) in body.signature.layouts.iter().enumerate() {
            layouts.insert(
                ty.clone(),
                function
                    .get_nth_param(index as u32 + 1)
                    .ok_or_else(|| Self::erased_error("erased layout argument missing"))?
                    .into_pointer_value(),
            );
        }
        let dictionary_base = 1 + body.signature.layouts.len();
        let args_base = dictionary_base + body.signature.dictionaries.len();
        let mut locals = Vec::new();
        let mut initialized = vec![false; code.locals.len()];
        for (index, ty) in code.locals.iter().enumerate() {
            let address = if index < body.signature.params.len() {
                initialized[index] = true;
                function
                    .get_nth_param((args_base + index) as u32)
                    .ok_or_else(|| Self::erased_error("erased value parameter missing"))?
                    .into_pointer_value()
            } else {
                self.allocate_erased_local(self.erased_layout(ty, &layouts)?)?
            };
            locals.push(address);
        }
        for statement in &code.statements {
            let destination = locals[statement.destination.0 as usize];
            match &statement.value {
                MirErasedRvalue::Use(operand) => {
                    let place = match operand {
                        MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
                    };
                    let source =
                        self.erased_place_address(body, &locals, place, &layouts, contract)?;
                    self.move_erased_bytes(
                        destination,
                        source,
                        self.erased_layout(
                            &code.locals[statement.destination.0 as usize],
                            &layouts,
                        )?,
                    )?;
                    if matches!(operand, MirErasedOperand::Move(_)) {
                        initialized[place.local.0 as usize] = false;
                    }
                }
                MirErasedRvalue::Literal(literal) => {
                    let value: BasicValueEnum = match literal {
                        MirErasedLiteral::Int(value) => self
                            .context
                            .i64_type()
                            .const_int(*value as u64, true)
                            .into(),
                        MirErasedLiteral::Bool(value) => self
                            .context
                            .bool_type()
                            .const_int(u64::from(*value), false)
                            .into(),
                        MirErasedLiteral::Unit => self.context.i64_type().const_zero().into(),
                    };
                    self.builder
                        .build_store(destination, value)
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                }
                MirErasedRvalue::Borrow { place, .. } => {
                    let address =
                        self.erased_place_address(body, &locals, place, &layouts, contract)?;
                    self.builder
                        .build_store(destination, address)
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                }
                MirErasedRvalue::Aggregate(operands) => {
                    let descriptor = self
                        .erased_layout(&code.locals[statement.destination.0 as usize], &layouts)?;
                    for (index, operand) in operands.iter().enumerate() {
                        let place = match operand {
                            MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
                        };
                        let field =
                            self.erased_field_address(destination, descriptor, index as u32)?;
                        let ty = erased_place_type(body, place, contract, self.type_context())
                            .map_err(|e| Self::erased_error(format!("{e:?}")))?;
                        self.move_erased_bytes(
                            field,
                            self.erased_place_address(body, &locals, place, &layouts, contract)?,
                            self.erased_layout(&ty, &layouts)?,
                        )?;
                        if matches!(operand, MirErasedOperand::Move(_)) {
                            initialized[place.local.0 as usize] = false;
                        }
                    }
                }
                MirErasedRvalue::Call {
                    target,
                    type_args,
                    dictionaries,
                    args,
                } => {
                    let callee = contract
                        .erased
                        .functions
                        .get(target)
                        .ok_or_else(|| Self::erased_error("erased callee missing"))?;
                    let mut values: Vec<BasicMetadataValueEnum> = vec![destination.into()];
                    for ty in &callee.signature.layouts {
                        values.push(
                            self.erased_layout(
                                &ty.substitute(type_args)
                                    .map_err(|e| Self::erased_error(format!("{e:?}")))?,
                                &layouts,
                            )?
                            .into(),
                        );
                    }
                    for dictionary in dictionaries {
                        values.push(
                            function
                                .get_nth_param((dictionary_base + *dictionary as usize) as u32)
                                .ok_or_else(|| {
                                    Self::erased_error("erased dictionary argument missing")
                                })?
                                .into(),
                        );
                    }
                    for operand in args {
                        let place = match operand {
                            MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
                        };
                        values.push(
                            self.erased_argument_address(
                                body, &locals, operand, &layouts, contract,
                            )?
                            .into(),
                        );
                        if matches!(operand, MirErasedOperand::Move(_)) {
                            initialized[place.local.0 as usize] = false;
                        }
                    }
                    self.builder
                        .build_call(self.erased_functions[target], &values, "erased.call")
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                }
                MirErasedRvalue::DictionaryCall {
                    dictionary,
                    member,
                    receiver,
                    args,
                } => {
                    let table = function
                        .get_nth_param((dictionary_base + *dictionary as usize) as u32)
                        .ok_or_else(|| Self::erased_error("dictionary parameter missing"))?
                        .into_pointer_value();
                    let pointer = unsafe {
                        self.builder.build_gep(
                            self.context.ptr_type(AddressSpace::default()),
                            table,
                            &[self.context.i64_type().const_int(*member as u64, false)],
                            "dictionary.entry.address",
                        )
                    }
                    .map_err(|e| Self::erased_error(e.to_string()))?;
                    let target = self
                        .builder
                        .build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            pointer,
                            "dictionary.entry",
                        )
                        .map_err(|e| Self::erased_error(e.to_string()))?
                        .into_pointer_value();
                    let mut values: Vec<BasicMetadataValueEnum> = vec![
                        destination.into(),
                        self.erased_place_address(body, &locals, receiver, &layouts, contract)?
                            .into(),
                    ];
                    if body.signature.dictionaries[*dictionary as usize].members[*member as usize]
                        .receiver
                        == crate::types::ReceiverMode::Move
                    {
                        initialized[receiver.local.0 as usize] = false;
                    }
                    for operand in args {
                        let place = match operand {
                            MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
                        };
                        values.push(
                            self.erased_argument_address(
                                body, &locals, operand, &layouts, contract,
                            )?
                            .into(),
                        );
                        if matches!(operand, MirErasedOperand::Move(_)) {
                            initialized[place.local.0 as usize] = false;
                        }
                    }
                    let ptr = self.context.ptr_type(AddressSpace::default());
                    self.builder
                        .build_indirect_call(
                            self.context
                                .void_type()
                                .fn_type(&vec![ptr.into(); values.len()], false),
                            target,
                            &values,
                            "erased.dictionary.call",
                        )
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                }
            }
            initialized[statement.destination.0 as usize] = true;
        }
        let result = match &code.result {
            MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
        };
        self.move_erased_bytes(
            out,
            self.erased_place_address(body, &locals, result, &layouts, contract)?,
            self.erased_layout(&body.signature.ret, &layouts)?,
        )?;
        if matches!(code.result, MirErasedOperand::Move(_)) {
            initialized[result.local.0 as usize] = false;
        }
        for (index, live) in initialized.into_iter().enumerate().rev() {
            if live {
                self.drop_erased_value(
                    locals[index],
                    self.erased_layout(&code.locals[index], &layouts)?,
                )?;
            }
        }
        self.builder
            .build_return(None)
            .map(|_| ())
            .map_err(|e| Self::erased_error(e.to_string()))
    }

    /// Only the validated owned-call path supplies this payload address. The
    /// erased callee takes its initialization; the caller then releases storage
    /// without invoking the payload destructor again.
    pub(super) fn compile_owned_erased_entry(
        &mut self,
        ctx: &MirFunctionContext<'ctx>,
        key: &MirOwnedObjectKey,
        plan: &MirOwnedObjectPlan,
        data: PointerValue<'ctx>,
        metadata: PointerValue<'ctx>,
        args: &[Operand],
        destination: &Place,
    ) -> Result<(), CodegenError> {
        let MirOwnedObjectSlot::Erased { slot, type_args } = &key.slot else {
            return Err(Self::erased_error(
                "owned erased entry requires an erased slot key",
            ));
        };
        let schema = self
            .object_schemas
            .get(&key.object)
            .cloned()
            .ok_or_else(|| Self::erased_error("owned erased object schema missing"))?;
        let signature = &schema
            .erased_slots
            .get(*slot as usize)
            .ok_or_else(|| Self::erased_error("owned erased slot missing"))?
            .signature;
        let types = std::iter::once(None)
            .chain(
                type_args
                    .iter()
                    .map(|id| Some(self.structural_type_for(*id))),
            )
            .collect::<Vec<_>>();
        let (out, _) = self.compile_mir_place_local_id(ctx, destination)?;
        let mut values: Vec<BasicMetadataValueEnum> = vec![out.into()];
        let address = self
            .builder
            .build_struct_gep(
                self.object_table_type(&schema),
                metadata,
                5,
                "owner.erased.descriptor.address",
            )
            .map_err(|e| Self::erased_error(e.to_string()))?;
        let descriptor = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                address,
                "owner.erased.descriptor",
            )
            .map_err(|e| Self::erased_error(e.to_string()))?
            .into_pointer_value();
        for layout in &signature.layouts {
            if *layout == MirErasedType::Parameter(0) {
                values.push(descriptor.into());
                continue;
            }
            let ty = layout
                .instantiate_optional(&types, self.type_context())
                .map_err(|e| Self::erased_error(format!("{e:?}")))?;
            let id = self
                .type_context()
                .id_for_type(&ty)
                .ok_or_else(|| Self::erased_error("owned erased layout type not interned"))?;
            values.push(
                self.erased_descriptors
                    .get(&id)
                    .ok_or_else(|| Self::erased_error("owned erased descriptor missing"))?
                    .as_pointer_value()
                    .into(),
            );
        }
        for dictionary in &plan.dictionaries {
            values.push(
                self.erased_dictionaries
                    .get(dictionary)
                    .ok_or_else(|| Self::erased_error("owned erased dictionary missing"))?
                    .as_pointer_value()
                    .into(),
            );
        }
        values.push(data.into());
        for arg in args {
            let (Operand::Copy(place) | Operand::Move(place)) = arg else {
                return Err(Self::erased_error(
                    "owned erased arguments require addressable values",
                ));
            };
            let (address, ty) = self.compile_mir_place_local_id(ctx, place)?;
            let address = if matches!(arg, Operand::Copy(_)) {
                let copy = self
                    .builder
                    .build_alloca(self.llvm_type_id(ty), "owner.erased.argument.copy")
                    .map_err(|e| Self::erased_error(e.to_string()))?;
                let value = self
                    .builder
                    .build_load(
                        self.llvm_type_id(ty),
                        address,
                        "owner.erased.argument.value",
                    )
                    .map_err(|e| Self::erased_error(e.to_string()))?;
                self.builder
                    .build_store(copy, value)
                    .map_err(|e| Self::erased_error(e.to_string()))?;
                copy
            } else {
                address
            };
            values.push(address.into());
        }
        let entry = self.load_object_entry(metadata, &schema, 6, *slot)?;
        self.builder
            .build_indirect_call(
                self.erased_function_type(signature),
                entry,
                &values,
                "owner.erased.consume",
            )
            .map_err(|e| Self::erased_error(e.to_string()))?;
        Ok(())
    }

    pub(super) fn compile_erased_call(
        &mut self,
        _function: &MirFunction,
        ctx: &MirFunctionContext<'ctx>,
        call: &MirErasedCall,
        args: &[Operand],
        destination: &Place,
    ) -> Result<(), CodegenError> {
        let (signature, virtual_receiver) = match &call.target {
            MirErasedCallTarget::Direct(key) => (
                self.erased_contract
                    .functions
                    .get(key)
                    .ok_or_else(|| Self::erased_error("erased call declaration missing"))?
                    .signature
                    .clone(),
                None,
            ),
            MirErasedCallTarget::Virtual(target) => {
                let schema = self
                    .object_schemas
                    .get(&target.object)
                    .cloned()
                    .ok_or_else(|| Self::erased_error("erased virtual schema missing"))?;
                let slot = schema
                    .erased_slots
                    .get(target.slot as usize)
                    .ok_or_else(|| Self::erased_error("erased virtual slot missing"))?;
                let receiver = args
                    .first()
                    .ok_or_else(|| Self::erased_error("erased virtual receiver missing"))?;
                let value = self.compile_mir_operand(_function, ctx, receiver)?;
                let (data, metadata) = self.object_parts(value)?;
                let descriptor_ptr = self
                    .builder
                    .build_struct_gep(
                        self.object_table_type(&schema),
                        metadata,
                        5,
                        "object.witness.descriptor.address",
                    )
                    .map_err(|e| Self::erased_error(e.to_string()))?;
                let descriptor = self
                    .builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        descriptor_ptr,
                        "object.witness.descriptor",
                    )
                    .map_err(|e| Self::erased_error(e.to_string()))?
                    .into_pointer_value();
                let code = self.load_object_entry(metadata, &schema, 6, target.slot)?;
                (
                    slot.signature.clone(),
                    Some((data, metadata, descriptor, code, slot.result)),
                )
            }
        };
        let types = if virtual_receiver.is_some() {
            std::iter::once(None)
                .chain(
                    call.type_args
                        .iter()
                        .map(|id| Some(self.structural_type_for(*id))),
                )
                .collect::<Vec<_>>()
        } else {
            call.type_args
                .iter()
                .map(|id| Some(self.structural_type_for(*id)))
                .collect()
        };
        let (out, _) = self.compile_mir_place_local_id(ctx, destination)?;
        let mut values: Vec<BasicMetadataValueEnum> = vec![out.into()];
        for ty in &signature.layouts {
            if let Some((_, _, descriptor, _, _)) = virtual_receiver {
                if *ty == MirErasedType::Parameter(0) {
                    values.push(descriptor.into());
                    continue;
                }
            }
            let ty = ty
                .instantiate_optional(&types, self.type_context())
                .map_err(|e| Self::erased_error(format!("{e:?}")))?;
            let id = self
                .type_context()
                .id_for_type(&ty)
                .ok_or_else(|| Self::erased_error("erased call layout type not interned"))?;
            values.push(
                self.erased_descriptors
                    .get(&id)
                    .ok_or_else(|| Self::erased_error("erased call descriptor missing"))?
                    .as_pointer_value()
                    .into(),
            );
        }
        for dictionary in &call.dictionaries {
            values.push(
                self.erased_dictionaries
                    .get(dictionary)
                    .ok_or_else(|| Self::erased_error("erased call dictionary missing"))?
                    .as_pointer_value()
                    .into(),
            );
        }
        for (index, arg) in args.iter().enumerate() {
            if index == 0 {
                if let Some((data, _, _, _, _)) = virtual_receiver {
                    let receiver = self
                        .builder
                        .build_alloca(
                            self.context.ptr_type(AddressSpace::default()),
                            "erased.receiver.reference",
                        )
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                    self.builder
                        .build_store(receiver, data)
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                    values.push(receiver.into());
                    continue;
                }
            }
            let (Operand::Copy(place) | Operand::Move(place)) = arg else {
                return Err(Self::erased_error(
                    "erased call arguments require initialized addressable MIR values",
                ));
            };
            let (address, ty) = self.compile_mir_place_local_id(ctx, place)?;
            let address = if matches!(arg, Operand::Copy(_)) {
                let copy = self
                    .builder
                    .build_alloca(self.llvm_type_id(ty), "erased.argument.copy")
                    .map_err(|e| Self::erased_error(e.to_string()))?;
                let value = self
                    .builder
                    .build_load(self.llvm_type_id(ty), address, "erased.argument.value")
                    .map_err(|e| Self::erased_error(e.to_string()))?;
                self.builder
                    .build_store(copy, value)
                    .map_err(|e| Self::erased_error(e.to_string()))?;
                copy
            } else {
                address
            };
            values.push(address.into());
        }
        if let Some((_, metadata, _, code, result)) = virtual_receiver {
            self.builder
                .build_indirect_call(
                    self.erased_function_type(&signature),
                    code,
                    &values,
                    "erased.virtual.invoke",
                )
                .map_err(|e| Self::erased_error(e.to_string()))?;
            if matches!(result, MirObjectResult::BorrowedSelf { .. }) {
                let address = self
                    .builder
                    .build_struct_gep(
                        self.object_handle_type(),
                        out,
                        1,
                        "erased.self.result.metadata",
                    )
                    .map_err(|e| Self::erased_error(e.to_string()))?;
                self.builder
                    .build_store(address, metadata)
                    .map_err(|e| Self::erased_error(e.to_string()))?;
            }
        } else {
            let MirErasedCallTarget::Direct(key) = &call.target else {
                unreachable!()
            };
            self.builder
                .build_call(self.erased_functions[key], &values, "erased.invoke")
                .map_err(|e| Self::erased_error(e.to_string()))?;
        }
        Ok(())
    }
}
