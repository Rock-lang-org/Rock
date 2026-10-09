use super::MirFunctionContext;
use crate::codegen::{CodeGen, CodegenError};
use crate::mir::*;
use inkwell::types::{BasicType, FunctionType, StructType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, IntValue, PointerValue};
use inkwell::{module::Linkage, AddressSpace};
use std::collections::BTreeMap;

#[cfg(test)]
#[path = "../descriptor_abi_tests.rs"]
mod descriptor_abi_tests;

#[cfg(test)]
#[path = "../polymorphic_dictionary_pipeline_tests.rs"]
mod polymorphic_dictionary_pipeline_tests;

#[cfg(test)]
#[path = "../tuple_pipeline_tests.rs"]
mod tuple_pipeline_tests;

#[cfg(test)]
#[path = "../nominal_pipeline_tests.rs"]
mod nominal_pipeline_tests;

#[cfg(test)]
#[path = "../dictionary_pipeline_tests.rs"]
mod dictionary_pipeline_tests;

impl<'ctx> CodeGen<'ctx> {
    fn erased_dictionary_type(&self) -> StructType<'ctx> {
        let word = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(
            &[
                word.into(),
                word.into(),
                word.into(),
                word.into(),
                ptr.into(),
                word.into(),
                ptr.into(),
                word.into(),
                ptr.into(),
                word.into(),
                ptr.into(),
                word.into(),
                ptr.into(),
                word.into(),
                ptr.into(),
                word.into(),
            ],
            false,
        )
    }

    fn dictionary_environment_array(
        &self,
        dictionary: PointerValue<'ctx>,
        field: u32,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let slot = self
            .builder
            .build_struct_gep(
                self.erased_dictionary_type(),
                dictionary,
                field,
                "dictionary.environment.slot",
            )
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                slot,
                "dictionary.environment",
            )
            .map(|v| v.into_pointer_value())
            .map_err(|e| Self::erased_error(e.to_string()))
    }

    fn dictionary_environment_entry(
        &self,
        array: PointerValue<'ctx>,
        index: usize,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let slot = unsafe {
            self.builder.build_gep(
                ptr,
                array,
                &[self.context.i64_type().const_int(index as u64, false)],
                "dictionary.environment.entry.slot",
            )
        }
        .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder
            .build_load(ptr, slot, "dictionary.environment.entry")
            .map(|v| v.into_pointer_value())
            .map_err(|e| Self::erased_error(e.to_string()))
    }

    fn descriptor_factory_target(&self) -> Result<PointerValue<'ctx>, CodegenError> {
        if let Some(global) = self.module.get_global("rock.descriptor.factory.target") {
            return Ok(global.as_pointer_value());
        }
        let word = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let key = self.descriptor_target_key()?;
        let key_pointer = self
            .module
            .get_global("rock.descriptor.target")
            .ok_or_else(|| Self::erased_error("factory target identity missing"))?
            .as_pointer_value();
        let ty = self.context.struct_type(
            &[
                word.into(),
                word.into(),
                word.into(),
                word.into(),
                word.into(),
                ptr.into(),
            ],
            false,
        );
        let global = self
            .module
            .add_global(ty, None, "rock.descriptor.factory.target");
        global.set_initializer(
            &ty.const_named_struct(&[
                word.const_int(crate::mir::descriptor_identity::DESCRIPTOR_ABI, false)
                    .into(),
                word.const_int(64, false).into(),
                word.const_int(1, false).into(),
                word.const_int(i64::MAX as u64, false).into(),
                word.const_int(key.len() as u64, false).into(),
                key_pointer.into(),
            ]),
        );
        global.set_constant(true);
        global.set_linkage(Linkage::Private);
        Ok(global.as_pointer_value())
    }

    fn emit_dictionary_factory(
        &self,
        function: inkwell::values::FunctionValue<'ctx>,
        requirement: &MirDictionaryFactoryRequirement,
        layouts: &BTreeMap<MirErasedType, PointerValue<'ctx>>,
        dictionaries: &[PointerValue<'ctx>],
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let factory = self
            .module
            .get_function(crate::products::RuntimeRootSymbol::SelectedDictionaryFactoryV1.symbol())
            .ok_or_else(|| {
                Self::erased_error("selected dictionary factory has no observed runtime helper")
            })?;
        let template = self
            .module
            .get_global(&format!(
                "rock.dictionary.template.{}.{}",
                requirement.implementation.crate_id.0, requirement.implementation.local.0
            ))
            .ok_or_else(|| Self::erased_error("selected dictionary template missing"))?;
        let types = self.factory_evidence_array(
            &requirement.arguments,
            layouts,
            "dictionary.factory.types",
        )?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let count = u32::try_from(requirement.children.len())
            .map_err(|_| Self::erased_error("dictionary children exceed LLVM array indexing"))?;
        let array = ptr.array_type(count);
        let children = self
            .builder
            .build_alloca(array, "dictionary.factory.children")
            .map_err(|e| Self::erased_error(e.to_string()))?;
        for (index, child) in requirement.children.iter().enumerate() {
            let value = dictionaries
                .get(*child as usize)
                .ok_or_else(|| Self::erased_error("dictionary child is not entry-dominating"))?;
            let slot = unsafe {
                self.builder.build_gep(
                    array,
                    children,
                    &[
                        self.context.i32_type().const_zero(),
                        self.context.i32_type().const_int(index as u64, false),
                    ],
                    "dictionary.factory.child",
                )
            }
            .map_err(|e| Self::erased_error(e.to_string()))?;
            self.builder
                .build_store(slot, *value)
                .map_err(|e| Self::erased_error(e.to_string()))?;
        }
        let out = self
            .builder
            .build_alloca(ptr, "dictionary.factory.result")
            .map_err(|e| Self::erased_error(e.to_string()))?;
        let result = self
            .builder
            .build_call(
                factory,
                &[
                    self.descriptor_factory_target()?.into(),
                    template.as_pointer_value().into(),
                    types.into(),
                    children.into(),
                    out.into(),
                ],
                "dictionary.factory.status",
            )
            .map_err(|e| Self::erased_error(e.to_string()))?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| Self::erased_error("dictionary status missing"))?
            .into_int_value();
        let valid = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                result,
                self.context.i32_type().const_zero(),
                "dictionary.factory.valid",
            )
            .map_err(|e| Self::erased_error(e.to_string()))?;
        let ready = self
            .context
            .append_basic_block(function, "dictionary.factory.ready");
        let failed = self
            .context
            .append_basic_block(function, "dictionary.factory.failed");
        self.builder
            .build_conditional_branch(valid, ready, failed)
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder.position_at_end(failed);
        let trap = self.module.get_function("llvm.trap").unwrap_or_else(|| {
            self.module.add_function(
                "llvm.trap",
                self.context.void_type().fn_type(&[], false),
                None,
            )
        });
        self.builder
            .build_call(trap, &[], "")
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder
            .build_unreachable()
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder.position_at_end(ready);
        self.builder
            .build_load(ptr, out, "dictionary.factory.dictionary")
            .map(|v| v.into_pointer_value())
            .map_err(|e| Self::erased_error(e.to_string()))
    }
    pub(super) fn erased_descriptor_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        self.context.struct_type(
            &[
                word.into(),
                word.into(),
                ptr.into(),
                word.into(),
                ptr.into(),
                word.into(),
                word.into(),
                word.into(),
                word.into(),
                ptr.into(),
                word.into(),
                ptr.into(),
                word.into(),
                ptr.into(),
                word.into(),
                ptr.into(),
                word.into(),
            ],
            false,
        )
    }
    fn descriptor_bytes(&self, bytes: &[u8], name: &str) -> PointerValue<'ctx> {
        let value = self.context.i8_type().const_array(
            &bytes
                .iter()
                .map(|b| self.context.i8_type().const_int(*b as u64, false))
                .collect::<Vec<_>>(),
        );
        let global = self.module.add_global(value.get_type(), None, name);
        global.set_initializer(&value);
        global.set_constant(true);
        global.set_linkage(Linkage::Private);
        global.as_pointer_value()
    }
    pub(super) fn descriptor_opaque_suffix(&self) -> Vec<BasicValueEnum<'ctx>> {
        let word = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        vec![
            word.const_int(crate::mir::descriptor_identity::DESCRIPTOR_ABI, false)
                .into(),
            word.const_zero().into(),
            word.const_int(2, false).into(),
            word.const_zero().into(),
            ptr.const_null().into(),
            word.const_zero().into(),
            ptr.const_null().into(),
            word.const_zero().into(),
            ptr.const_null().into(),
            word.const_zero().into(),
            ptr.const_null().into(),
            word.const_int(i64::MAX as u64, false).into(),
        ]
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
            let mut scoped = std::collections::BTreeSet::new();
            ty.scoped_binders(&mut scoped);
            if !scoped.is_empty() {
                return self
                    .builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        *layout,
                        "opened.descriptor",
                    )
                    .map_err(|e| Self::erased_error(e.to_string()))
                    .map(|value| value.into_pointer_value());
            }
            return Ok(*layout);
        }
        // Direct scoped handles reuse their canonical descriptor; synthesized
        // uniform metadata is needed only for broader aggregate layouts.
        if matches!(ty, MirErasedType::Reference { inner, .. } | MirErasedType::Pointer(inner)
            if matches!(inner.as_ref(), MirErasedType::Scoped(_)))
        {
            return self
                .module
                .get_global("rock.descriptor.thin_reference")
                .map(|global| global.as_pointer_value())
                .ok_or_else(|| Self::erased_error("thin scoped reference descriptor missing"));
        }
        if let Some(global) = self
            .module
            .get_global(&format!("rock.uniform.erased.{ty:?}"))
        {
            return Ok(global.as_pointer_value());
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
        if let Some(descriptor) = ty
            .instantiate(&[], self.type_context())
            .ok()
            .and_then(|ty| self.type_context().id_for_type(&ty))
            .and_then(|id| self.erased_descriptors.get(&id))
        {
            return Ok(descriptor.as_pointer_value());
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

    fn emit_body_factory(
        &self,
        function: inkwell::values::FunctionValue<'ctx>,
        requirement: &MirBodyFactoryRequirement,
        layouts: &BTreeMap<MirErasedType, PointerValue<'ctx>>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let (root, label, fields) = match requirement {
            MirBodyFactoryRequirement::Tuple(r) => (
                crate::products::RuntimeRootSymbol::TupleDescriptorFactoryV1,
                "tuple.factory",
                r.children.clone(),
            ),
            MirBodyFactoryRequirement::Nominal(r) => (
                crate::products::RuntimeRootSymbol::NominalDescriptorFactoryV1,
                "nominal.factory",
                r.fields.iter().map(|(_, ty)| ty.clone()).collect(),
            ),
        };
        let factory = self.module.get_function(root.symbol()).ok_or_else(|| {
            Self::erased_error("body factory has no observed runtime helper declaration")
        })?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        let target = if let Some(global) = self.module.get_global("rock.descriptor.factory.target")
        {
            global
        } else {
            let key = self.descriptor_target_key()?;
            let key_pointer = self
                .module
                .get_global("rock.descriptor.target")
                .ok_or_else(|| Self::erased_error("tuple factory target identity missing"))?
                .as_pointer_value();
            let target_type = self.context.struct_type(
                &[
                    word.into(),
                    word.into(),
                    word.into(),
                    word.into(),
                    word.into(),
                    ptr.into(),
                ],
                false,
            );
            let global =
                self.module
                    .add_global(target_type, None, "rock.descriptor.factory.target");
            global.set_initializer(
                &target_type.const_named_struct(&[
                    word.const_int(crate::mir::descriptor_identity::DESCRIPTOR_ABI, false)
                        .into(),
                    word.const_int(64, false).into(),
                    word.const_int(1, false).into(),
                    word.const_int(i64::MAX as u64, false).into(),
                    word.const_int(key.len() as u64, false).into(),
                    key_pointer.into(),
                ]),
            );
            global.set_constant(true);
            global.set_linkage(Linkage::Private);
            global
        };
        let children =
            self.factory_evidence_array(&fields, layouts, &format!("{label}.children"))?;
        let output = self
            .builder
            .build_alloca(ptr, &format!("{label}.result"))
            .map_err(|e| Self::erased_error(e.to_string()))?;
        let arguments: Vec<BasicMetadataValueEnum<'ctx>> = match requirement {
            MirBodyFactoryRequirement::Tuple(_) => vec![
                target.as_pointer_value().into(),
                word.const_int(fields.len() as u64, false).into(),
                children.into(),
                output.into(),
            ],
            MirBodyFactoryRequirement::Nominal(r) => {
                let bytes = self.descriptor_bytes(&r.template, "rock.nominal.template.key");
                let template_ty = self
                    .context
                    .struct_type(&[word.into(), word.into(), ptr.into()], false);
                let template = self
                    .module
                    .add_global(template_ty, None, "rock.nominal.template");
                template.set_initializer(&template_ty.const_named_struct(&[
                    word.const_int(r.descriptor_abi, false).into(),
                    word.const_int(r.template.len() as u64, false).into(),
                    bytes.into(),
                ]));
                template.set_constant(true);
                template.set_linkage(Linkage::Private);
                let parameters = self.factory_evidence_array(
                    &r.arguments,
                    layouts,
                    "nominal.factory.arguments",
                )?;
                vec![
                    target.as_pointer_value().into(),
                    template.as_pointer_value().into(),
                    parameters.into(),
                    children.into(),
                    output.into(),
                ]
            }
        };
        let result = self
            .builder
            .build_call(factory, &arguments, &format!("{label}.status"))
            .map_err(|e| Self::erased_error(e.to_string()))?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| Self::erased_error("tuple factory status is not an integer"))?
            .into_int_value();
        let valid = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                result,
                self.context.i32_type().const_zero(),
                "tuple.factory.valid",
            )
            .map_err(|e| Self::erased_error(e.to_string()))?;
        let ready = self
            .context
            .append_basic_block(function, "tuple.factory.ready");
        let failed = self
            .context
            .append_basic_block(function, "tuple.factory.failed");
        self.builder
            .build_conditional_branch(valid, ready, failed)
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder.position_at_end(failed);
        let trap = self.module.get_function("llvm.trap").unwrap_or_else(|| {
            self.module.add_function(
                "llvm.trap",
                self.context.void_type().fn_type(&[], false),
                None,
            )
        });
        self.builder
            .build_call(trap, &[], "")
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder
            .build_unreachable()
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder.position_at_end(ready);
        self.builder
            .build_load(ptr, output, &format!("{label}.descriptor"))
            .map(|value| value.into_pointer_value())
            .map_err(|e| Self::erased_error(e.to_string()))
    }
    fn factory_evidence_array(
        &self,
        types: &[MirErasedType],
        layouts: &BTreeMap<MirErasedType, PointerValue<'ctx>>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let count = u32::try_from(types.len())
            .map_err(|_| Self::erased_error("factory child count exceeds LLVM array indexing"))?;
        let array = self
            .context
            .ptr_type(AddressSpace::default())
            .array_type(count);
        let address = self
            .builder
            .build_alloca(array, name)
            .map_err(|e| Self::erased_error(e.to_string()))?;
        for (index, ty) in types.iter().enumerate() {
            let slot = unsafe {
                self.builder.build_gep(
                    array,
                    address,
                    &[
                        self.context.i32_type().const_zero(),
                        self.context.i32_type().const_int(index as u64, false),
                    ],
                    "factory.child",
                )
            }
            .map_err(|e| Self::erased_error(e.to_string()))?;
            self.builder
                .build_store(slot, self.erased_layout(ty, layouts)?)
                .map_err(|e| Self::erased_error(e.to_string()))?;
        }
        Ok(address)
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

    fn copy_erased_operand_bytes(
        &mut self,
        destination: PointerValue<'ctx>,
        source: PointerValue<'ctx>,
        descriptor: PointerValue<'ctx>,
        operand: &MirErasedOperand,
        ty: &MirErasedType,
        layouts: &BTreeMap<MirErasedType, PointerValue<'ctx>>,
    ) -> Result<(), CodegenError> {
        self.move_erased_bytes(destination, source, descriptor)?;
        if matches!(operand, MirErasedOperand::Copy(_)) {
            self.duplicate_erased_copy_environment(destination, descriptor, ty, layouts)?;
        }
        Ok(())
    }

    fn duplicate_erased_copy_environment(
        &mut self,
        address: PointerValue<'ctx>,
        descriptor: PointerValue<'ctx>,
        ty: &MirErasedType,
        layouts: &BTreeMap<MirErasedType, PointerValue<'ctx>>,
    ) -> Result<(), CodegenError> {
        match ty {
            MirErasedType::Concrete(id) => {
                let value = self
                    .builder
                    .build_load(self.llvm_type_id(*id), address, "erased.copy.value")
                    .map_err(|e| Self::erased_error(e.to_string()))?;
                let value = self.copy_mir_value(value, &self.structural_type_for(*id))?;
                self.builder
                    .build_store(address, value)
                    .map_err(|e| Self::erased_error(e.to_string()))?;
            }
            MirErasedType::Tuple(fields) => {
                for (index, field) in fields.iter().enumerate() {
                    let address = self.erased_field_address(address, descriptor, index as u32)?;
                    self.duplicate_erased_copy_environment(
                        address,
                        self.erased_layout(field, layouts)?,
                        field,
                        layouts,
                    )?;
                }
            }
            MirErasedType::Reference { mutable: false, .. } => {}
            _ => {
                return Err(Self::erased_error(
                    "erased Copy lacks concrete structural authority",
                ))
            }
        }
        Ok(())
    }
    pub(super) fn drop_erased_value(
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
        if !crate::mir::descriptor_identity::validate_descriptor_identities(
            contract,
            self.type_context(),
        ) {
            return Err(Self::erased_error(
                "descriptor semantic/drop identity disagrees with canonical compiler authority",
            ));
        }
        let target_key = self.descriptor_target_key()?;
        for support in &contract.erased.imported_runtime_support {
            support.validate().map_err(Self::erased_error)?;
            if support.target_key() != target_key.as_slice() {
                return Err(Self::erased_error("imported tuple descriptor runtime has an incompatible native LLVM target contract"));
            }
        }
        let target_pointer = self.descriptor_bytes(&target_key, "rock.descriptor.target");
        self.erased_contract = contract.erased.clone();
        let ptr = self.context.ptr_type(AddressSpace::default());
        if contract.erased.functions.values().any(|function| {
            function
                .body
                .as_ref()
                .is_some_and(|body| !body.scopes.is_empty())
        }) {
            // Opened references are thin native pointers, not payload layouts.
            let drop = self.module.add_function(
                "rock.descriptor.thin_reference.drop",
                self.context
                    .void_type()
                    .fn_type(&[ptr.into(), ptr.into()], false),
                Some(Linkage::Internal),
            );
            self.builder
                .position_at_end(self.context.append_basic_block(drop, "entry"));
            self.builder
                .build_return(None)
                .map_err(|e| Self::erased_error(e.to_string()))?;
            let global = self.module.add_global(
                self.erased_descriptor_type(),
                None,
                "rock.descriptor.thin_reference",
            );
            let mut values: Vec<BasicValueEnum<'ctx>> = vec![
                ptr.size_of().into(),
                self.context
                    .struct_type(&[ptr.into()], false)
                    .get_alignment()
                    .into(),
                drop.as_global_value().as_pointer_value().into(),
                self.context.i64_type().const_zero().into(),
                ptr.const_null().into(),
            ];
            values.extend(self.descriptor_opaque_suffix());
            global.set_initializer(&self.erased_descriptor_type().const_named_struct(&values));
            global.set_constant(true);
            global.set_linkage(Linkage::Private);
        }
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
        // Declare first: immutable environments may refer to later descriptors.
        for id in contract.erased.descriptors.keys() {
            let global = self.module.add_global(
                self.erased_descriptor_type(),
                None,
                &format!("rock.descriptor.{}", id.0),
            );
            global.set_constant(true);
            global.set_linkage(Linkage::Private);
            self.erased_descriptors.insert(*id, global);
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
            let mut values: Vec<BasicValueEnum<'ctx>> = vec![
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
            ];
            let identity = contract.descriptor_identities.get(id).filter(|_| {
                crate::mir::descriptor_identity::concrete_descriptor_is_sealed(contract, *id)
            });
            if let Some(identity) = identity {
                let children = descriptor
                    .fields
                    .iter()
                    .map(|ty| self.erased_descriptors[ty].as_pointer_value())
                    .collect::<Vec<_>>();
                let array = ptr.const_array(&children);
                let environment = self.module.add_global(
                    array.get_type(),
                    None,
                    &format!("rock.descriptor.children.{}", id.0),
                );
                environment.set_initializer(&array);
                environment.set_constant(true);
                environment.set_linkage(Linkage::Private);
                let kind = if matches!(
                    self.type_context().ty(*id),
                    crate::type_context::Ty::Tuple(_)
                ) {
                    2
                } else {
                    1
                };
                let suffix: [BasicValueEnum<'ctx>; 12] = [
                    word.const_int(crate::mir::descriptor_identity::DESCRIPTOR_ABI, false)
                        .into(),
                    word.const_int(kind, false).into(),
                    word.const_int(2, false).into(),
                    word.const_int(identity.semantic.len() as u64, false).into(),
                    self.descriptor_bytes(
                        &identity.semantic,
                        &format!("rock.descriptor.key.{}", id.0),
                    )
                    .into(),
                    word.const_int(identity.drop_template.len() as u64, false)
                        .into(),
                    self.descriptor_bytes(
                        &identity.drop_template,
                        &format!("rock.descriptor.dropkey.{}", id.0),
                    )
                    .into(),
                    word.const_int(children.len() as u64, false).into(),
                    environment.as_pointer_value().into(),
                    word.const_int(target_key.len() as u64, false).into(),
                    target_pointer.into(),
                    word.const_int(i64::MAX as u64, false).into(),
                ];
                values.extend(suffix);
            } else {
                values.extend(self.descriptor_opaque_suffix());
            }
            let initializer = self.erased_descriptor_type().const_named_struct(&values);
            let global = self.erased_descriptors[id];
            global.set_initializer(&initializer);
            global.set_constant(true);
            global.set_linkage(Linkage::Private);
            self.erased_descriptors.insert(*id, global);
        }
        self.emit_erased_dictionaries(contract)?;
        self.emit_selected_dictionary_templates(contract)?;
        for schema in contract.packages.values() {
            if package_payload_drop_free(contract, self.type_context(), schema.package) {
                self.emit_uniform_descriptor(
                    &schema.payload_layout,
                    &format!("rock.uniform.package.{}", schema.package.0),
                )?;
            }
        }
        for body in contract
            .erased
            .functions
            .values()
            .filter_map(|entry| entry.body.as_ref())
        {
            for ty in &body.locals {
                let mut scoped = std::collections::BTreeSet::new();
                ty.scoped_binders(&mut scoped);
                if !scoped.is_empty() {
                    if let Ok(layout) =
                        erased_uniform_layout(contract, &mut self.type_context().clone(), ty)
                    {
                        self.emit_uniform_descriptor(
                            &layout,
                            &format!("rock.uniform.erased.{ty:?}"),
                        )?;
                    }
                }
            }
        }
        // Tables reference erased declarations and descriptors; bodies can in
        // turn package those immutable tables, so declare metadata first.
        self.emit_object_vtables(contract)?;
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
                if let MirDictionaryMemberTarget::Polymorphic(key) = &member.target {
                    let authority = &contract.erased.member_entries[key];
                    entries.push(self.emit_polymorphic_dictionary_adapter(
                        contract,
                        key,
                        &authority.member,
                        &authority.captures,
                        &[],
                        &format!("rock.dictionary.{}.{}", id.0, index),
                    )?);
                    continue;
                }
                let native = member
                    .target
                    .native()
                    .ok_or_else(|| Self::erased_error("dictionary target authority missing"))?;
                let target = contract
                    .callable(native)
                    .ok_or_else(|| Self::erased_error("dictionary entry callable missing"))?;
                let symbol =
                    self.resolve_mir_callable_symbol(&MirCallable::Resolved(native.clone()))?;
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
                        .fn_type(&vec![ptr.into(); target.signature.params.len() + 2], false),
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
                        .get_nth_param(parameter as u32 + 2)
                        .ok_or_else(|| Self::erased_error("dictionary argument missing"))?
                        .into_pointer_value();
                    let receiver_address = parameter == 0
                        && member.receiver.is_some()
                        && (member.receiver != Some(crate::types::ReceiverMode::Move)
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
            let entry_global = self.module.add_global(
                value.get_type(),
                None,
                &format!("rock.dictionary.entries.{}", id.0),
            );
            entry_global.set_initializer(&value);
            entry_global.set_constant(true);
            entry_global.set_linkage(Linkage::Private);
            // Opaque callable/associated metadata remains executable but cannot
            // enter an exact selected-factory environment.
            let keys = match native_dictionary_keys(contract, self.type_context(), *id) {
                Ok(keys) => Some(keys),
                Err(error)
                    if dictionary.members.iter().any(|member| {
                        member.receiver.is_none()
                            || matches!(member.target, MirDictionaryMemberTarget::Polymorphic(_))
                    }) =>
                {
                    return Err(Self::erased_error(format!(
                        "static/polymorphic dictionary lacks its exact binding contract: {error:?}"
                    )))
                }
                Err(_) => None,
            };
            let word = self.context.i64_type();
            let (schema_size, schema, binding_size, binding) = if let Some((schema, binding)) =
                &keys
            {
                (
                    schema.len(),
                    self.descriptor_bytes(schema, &format!("rock.dictionary.schema.{}", id.0)),
                    binding.len(),
                    self.descriptor_bytes(binding, &format!("rock.dictionary.binding.{}", id.0)),
                )
            } else {
                (0, ptr.const_null(), 0, ptr.const_null())
            };
            let target = self.descriptor_target_key()?;
            let target_pointer = self
                .module
                .get_global("rock.descriptor.target")
                .ok_or_else(|| Self::erased_error("dictionary target missing"))?
                .as_pointer_value();
            let global = self.module.add_global(
                self.erased_dictionary_type(),
                None,
                &format!("rock.dictionary.{}", id.0),
            );
            global.set_initializer(&self.erased_dictionary_type().const_named_struct(&[
                word.const_int(DICTIONARY_ABI, false).into(),
                word.const_int(u64::from(keys.is_some()), false).into(),
                word.const_int(2, false).into(),
                word.const_int(schema_size as u64, false).into(),
                schema.into(),
                word.const_int(binding_size as u64, false).into(),
                binding.into(),
                word.const_int(entries.len() as u64, false).into(),
                entry_global.as_pointer_value().into(),
                word.const_zero().into(),
                ptr.const_null().into(),
                word.const_zero().into(),
                ptr.const_null().into(),
                word.const_int(target.len() as u64, false).into(),
                target_pointer.into(),
                word.const_int(i64::MAX as u64, false).into(),
            ]));
            global.set_constant(true);
            global.set_linkage(Linkage::Private);
            self.erased_dictionaries.insert(*id, global);
        }
        Ok(())
    }

    fn emit_selected_dictionary_templates(
        &mut self,
        contract: &MirBackendContract,
    ) -> Result<(), CodegenError> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        for template in contract.erased.dictionary_templates.values() {
            let key = dictionary_template_key(contract, self.type_context(), template)
                .map_err(|e| Self::erased_error(format!("{e:?}")))?;
            let mut entries = Vec::new();
            for (binding, member) in template.members.iter().zip(&template.result.members) {
                if !member.binder.parameters.is_empty() {
                    entries.push(self.emit_polymorphic_dictionary_adapter(
                        contract,
                        &binding.entry,
                        member,
                        &template.parameters,
                        &template.bounds,
                        &format!(
                            "rock.dictionary.selected.{}.{}.{}",
                            template.implementation.crate_id.0,
                            template.implementation.local.0,
                            binding.member_id.local.0
                        ),
                    )?);
                    continue;
                }
                let declaration = &contract.erased.functions[&binding.entry];
                let entry = self.module.add_function(
                    &format!(
                        "rock.dictionary.selected.{}.{}.{}",
                        template.implementation.crate_id.0,
                        template.implementation.local.0,
                        binding.member_id.local.0
                    ),
                    self.context
                        .void_type()
                        .fn_type(&vec![ptr.into(); member.params.len() + 2], false),
                    Some(Linkage::Internal),
                );
                self.builder
                    .position_at_end(self.context.append_basic_block(entry, "entry"));
                let parameter = |index| {
                    entry
                        .get_nth_param(index)
                        .map(|v| v.into_pointer_value())
                        .ok_or_else(|| {
                            Self::erased_error("selected dictionary adapter parameter missing")
                        })
                };
                let environment = parameter(1)?;
                let types = self.dictionary_environment_array(environment, 10)?;
                let children = self.dictionary_environment_array(environment, 12)?;
                let mut layouts = BTreeMap::new();
                for index in 0..template.parameters.len() {
                    layouts.insert(
                        MirErasedType::Parameter(index as u32),
                        self.dictionary_environment_entry(types, index)?,
                    );
                }
                let mut args: Vec<BasicMetadataValueEnum<'ctx>> = vec![parameter(0)?.into()];
                for layout in &declaration.signature.layouts {
                    let descriptor = if matches!(layout, MirErasedType::Reference { inner, .. } if matches!(inner.as_ref(), MirErasedType::Parameter(_)))
                    {
                        self.selected_reference_descriptor()?
                    } else {
                        self.erased_layout(layout, &layouts)?
                    };
                    args.push(descriptor.into());
                }
                for index in 0..template.bounds.len() {
                    args.push(self.dictionary_environment_entry(children, index)?.into());
                }
                if let Some(mode) = member.receiver {
                    let receiver = parameter(2)?;
                    if mode == crate::types::ReceiverMode::Move {
                        args.push(receiver.into());
                    } else {
                        let reference = self
                            .builder
                            .build_alloca(ptr, "dictionary.receiver.reference")
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        self.builder
                            .build_store(reference, receiver)
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        args.push(reference.into());
                    }
                }
                for index in usize::from(member.receiver.is_some())..member.params.len() {
                    args.push(parameter(index as u32 + 2)?.into());
                }
                self.builder
                    .build_call(
                        self.erased_functions[&binding.entry],
                        &args,
                        "dictionary.selected.invoke",
                    )
                    .map_err(|e| Self::erased_error(e.to_string()))?;
                if matches!(
                    member.ret.instantiate(&[], self.type_context()),
                    Ok(crate::types::Type::Never)
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
            let array = ptr.const_array(&entries);
            let entry_global =
                self.module
                    .add_global(array.get_type(), None, "rock.dictionary.selected.entries");
            entry_global.set_initializer(&array);
            entry_global.set_constant(true);
            entry_global.set_linkage(Linkage::Private);
            let ty = self.context.struct_type(
                &[
                    word.into(),
                    word.into(),
                    ptr.into(),
                    word.into(),
                    ptr.into(),
                ],
                false,
            );
            let global = self.module.add_global(
                ty,
                None,
                &format!(
                    "rock.dictionary.template.{}.{}",
                    template.implementation.crate_id.0, template.implementation.local.0
                ),
            );
            global.set_initializer(
                &ty.const_named_struct(&[
                    word.const_int(DICTIONARY_ABI, false).into(),
                    word.const_int(key.len() as u64, false).into(),
                    self.descriptor_bytes(&key, "rock.dictionary.template.key")
                        .into(),
                    word.const_int(entries.len() as u64, false).into(),
                    entry_global.as_pointer_value().into(),
                ]),
            );
            global.set_constant(true);
            global.set_linkage(Linkage::Private);
        }
        Ok(())
    }

    fn emit_polymorphic_dictionary_adapter(
        &mut self,
        contract: &MirBackendContract,
        key: &MirErasedKey,
        member: &MirErasedDictionaryMember,
        captures: &[MirErasedParameter],
        bounds: &[MirErasedDictionarySchema],
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        if member.receiver != Some(crate::types::ReceiverMode::Shared) {
            return Err(Self::erased_error(
                "polymorphic member receiver authority missing",
            ));
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let declaration = &contract.erased.functions[key];
        let locals = member.binder.parameters.len();
        let entry = self.module.add_function(
            name,
            self.context
                .void_type()
                .fn_type(&vec![ptr.into(); member.params.len() + locals + 2], false),
            Some(Linkage::Internal),
        );
        self.builder
            .position_at_end(self.context.append_basic_block(entry, "entry"));
        let parameter = |index| {
            entry
                .get_nth_param(index)
                .map(|v| v.into_pointer_value())
                .ok_or_else(|| {
                    Self::erased_error("polymorphic dictionary adapter parameter missing")
                })
        };
        let environment = parameter(1)?;
        let types = self.dictionary_environment_array(environment, 10)?;
        let children = self.dictionary_environment_array(environment, 12)?;
        let mut layouts = BTreeMap::new();
        for index in 0..captures.len() {
            layouts.insert(
                MirErasedType::Parameter(index as u32),
                self.dictionary_environment_entry(types, index)?,
            );
        }
        for index in 0..locals {
            layouts.insert(
                MirErasedType::Parameter((captures.len() + index) as u32),
                parameter(index as u32 + 3)?,
            );
        }
        let mut args: Vec<BasicMetadataValueEnum<'ctx>> = vec![parameter(0)?.into()];
        for layout in &declaration.signature.layouts {
            let descriptor = if matches!(layout, MirErasedType::Reference { inner, .. } if matches!(inner.as_ref(), MirErasedType::Parameter(_)))
            {
                self.selected_reference_descriptor()?
            } else {
                self.erased_layout(layout, &layouts)?
            };
            args.push(descriptor.into());
        }
        for index in 0..bounds.len() {
            args.push(self.dictionary_environment_entry(children, index)?.into());
        }
        let reference = self
            .builder
            .build_alloca(ptr, "dictionary.receiver.reference")
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder
            .build_store(reference, parameter(2)?)
            .map_err(|e| Self::erased_error(e.to_string()))?;
        args.push(reference.into());
        for index in 1..member.params.len() {
            args.push(parameter((index + locals + 2) as u32)?.into());
        }
        self.builder
            .build_call(
                self.erased_functions[key],
                &args,
                "dictionary.polymorphic.invoke",
            )
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder
            .build_return(None)
            .map_err(|e| Self::erased_error(e.to_string()))?;
        Ok(entry.as_global_value().as_pointer_value())
    }

    fn selected_reference_descriptor(&self) -> Result<PointerValue<'ctx>, CodegenError> {
        if let Some(global) = self
            .module
            .get_global("rock.dictionary.reference.descriptor")
        {
            return Ok(global.as_pointer_value());
        }
        // Only a local thin-reference ABI layout, never exact factory evidence.
        let ptr = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        let drop = self.module.add_function(
            "rock.dictionary.reference.drop",
            self.context
                .void_type()
                .fn_type(&[ptr.into(), ptr.into()], false),
            Some(Linkage::Internal),
        );
        let block = self
            .builder
            .get_insert_block()
            .ok_or_else(|| Self::erased_error("dictionary adapter has no block"))?;
        self.builder
            .position_at_end(self.context.append_basic_block(drop, "entry"));
        self.builder
            .build_return(None)
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder.position_at_end(block);
        let mut fields: Vec<BasicValueEnum<'ctx>> = vec![
            word.const_int(8, false).into(),
            word.const_int(8, false).into(),
            drop.as_global_value().as_pointer_value().into(),
            word.const_zero().into(),
            ptr.const_null().into(),
        ];
        fields.extend(self.descriptor_opaque_suffix());
        let global = self.module.add_global(
            self.erased_descriptor_type(),
            None,
            "rock.dictionary.reference.descriptor",
        );
        global.set_initializer(&self.erased_descriptor_type().const_named_struct(&fields));
        global.set_constant(true);
        global.set_linkage(Linkage::Private);
        Ok(global.as_pointer_value())
    }

    fn erased_place_address(
        &self,
        body: &MirErasedFunction,
        locals: &[PointerValue<'ctx>],
        place: &MirErasedPlace,
        layouts: &BTreeMap<MirErasedType, PointerValue<'ctx>>,
        contract: &MirBackendContract,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let slot = *locals
            .get(place.local.0 as usize)
            .ok_or_else(|| Self::erased_error("erased local missing"))?;
        let mut address = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                slot,
                "erased.local.address",
            )
            .map_err(|e| Self::erased_error(e.to_string()))?
            .into_pointer_value();
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
                MirErasedProjection::NominalField { owner, field } => {
                    let index = nominal_field(contract, self.type_context(), &ty, *owner, *field)
                        .map_err(|error| Self::erased_error(format!("{error:?}")))?
                        .0;
                    self.erased_field_address(address, self.erased_layout(&ty, layouts)?, index)?
                }
            };
            prefix.projection.push(projection.clone());
        }
        Ok(address)
    }

    fn erased_argument_address(
        &mut self,
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
        self.copy_erased_operand_bytes(copy, address, descriptor, operand, &ty, layouts)?;
        Ok(copy)
    }

    fn initialize_erased_fields<'a>(
        &mut self,
        body: &MirErasedFunction,
        locals: &[PointerValue<'ctx>],
        initialized: &[PointerValue<'ctx>],
        layouts: &BTreeMap<MirErasedType, PointerValue<'ctx>>,
        contract: &MirBackendContract,
        destination: PointerValue<'ctx>,
        descriptor: PointerValue<'ctx>,
        operands: impl Iterator<Item = &'a MirErasedOperand>,
    ) -> Result<(), CodegenError> {
        for (index, operand) in operands.enumerate() {
            let place = match operand {
                MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
            };
            let index = u32::try_from(index)
                .map_err(|_| Self::erased_error("record field index exceeds LLVM indexing"))?;
            let field = self.erased_field_address(destination, descriptor, index)?;
            let ty = erased_place_type(body, place, contract, self.type_context())
                .map_err(|e| Self::erased_error(format!("{e:?}")))?;
            self.copy_erased_operand_bytes(
                field,
                self.erased_place_address(body, locals, place, layouts, contract)?,
                self.erased_layout(&ty, layouts)?,
                operand,
                &ty,
                layouts,
            )?;
            if matches!(operand, MirErasedOperand::Move(_)) {
                self.builder
                    .build_store(
                        initialized[place.local.0 as usize],
                        self.context.bool_type().const_zero(),
                    )
                    .map_err(|e| Self::erased_error(e.to_string()))?;
            }
        }
        Ok(())
    }

    fn erased_native_argument(
        &self,
        address: PointerValue<'ctx>,
        parameter: &MirParamAbi,
    ) -> Result<BasicMetadataValueEnum<'ctx>, CodegenError> {
        if parameter.pass_mode == MirPassMode::Pointer
            && !self.type_id_lowers_to_pointer(parameter.semantic_ty)
        {
            return Ok(address.into());
        }
        self.builder
            .build_load(
                self.llvm_type_id(parameter.semantic_ty),
                address,
                "erased.native.argument",
            )
            .map(|value| value.into())
            .map_err(|e| Self::erased_error(e.to_string()))
    }

    fn cleanup_erased_local(
        &mut self,
        body: &MirErasedFunction,
        local: MirErasedLocal,
        locals: &[PointerValue<'ctx>],
        initialized: &[PointerValue<'ctx>],
        layouts: &BTreeMap<MirErasedType, PointerValue<'ctx>>,
        contract: &MirBackendContract,
    ) -> Result<(), CodegenError> {
        let index = local.0 as usize;
        let flag = *initialized
            .get(index)
            .ok_or_else(|| Self::erased_error("cleanup local missing"))?;
        let ty = body
            .body
            .as_ref()
            .and_then(|body| body.locals.get(index))
            .ok_or_else(|| Self::erased_error("cleanup local type missing"))?;
        let function = self.erased_functions[&body.key];
        let live = self
            .builder
            .build_load(self.context.bool_type(), flag, "erased.drop.live")
            .map_err(|e| Self::erased_error(e.to_string()))?
            .into_int_value();
        let drop_block = self.context.append_basic_block(function, "erased.drop");
        let next_block = self
            .context
            .append_basic_block(function, "erased.drop.next");
        self.builder
            .build_conditional_branch(live, drop_block, next_block)
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder.position_at_end(drop_block);
        self.builder
            .build_store(flag, self.context.bool_type().const_zero())
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.drop_erased_value(
            self.erased_place_address(
                body,
                locals,
                &MirErasedPlace::local(local),
                layouts,
                contract,
            )?,
            self.erased_layout(ty, layouts)?,
        )?;
        self.builder
            .build_unconditional_branch(next_block)
            .map_err(|e| Self::erased_error(e.to_string()))?;
        self.builder.position_at_end(next_block);
        Ok(())
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
        // Entry-dominating requirements are checked dependency-first before any
        // payload local allocation. The C runtime freezes the temporary array.
        for requirement in collect_body_factories(body, contract, self.type_context())
            .map_err(|error| Self::erased_error(format!("{error:?}")))?
        {
            let descriptor = self.emit_body_factory(function, &requirement, &layouts)?;
            layouts.insert(requirement.result().clone(), descriptor);
        }
        let dictionary_base = 1 + body.signature.layouts.len();
        let args_base = dictionary_base + body.signature.dictionaries.len();
        let dictionary_schemas = validate_dictionary_factories(
            body,
            contract,
            self.type_context(),
            &validate_body_factories(body, contract, self.type_context())
                .map_err(|e| Self::erased_error(format!("{e:?}")))?,
        )
        .map_err(|e| Self::erased_error(format!("{e:?}")))?;
        let mut body_dictionaries = (0..body.signature.dictionaries.len())
            .map(|index| {
                function
                    .get_nth_param((dictionary_base + index) as u32)
                    .map(|v| v.into_pointer_value())
                    .ok_or_else(|| Self::erased_error("public dictionary argument missing"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for requirement in contract
            .erased
            .dictionary_factories
            .get(&body.key)
            .into_iter()
            .flatten()
        {
            let dictionary =
                self.emit_dictionary_factory(function, requirement, &layouts, &body_dictionaries)?;
            body_dictionaries.push(dictionary);
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let mut scope_tables = Vec::new();
        let mut scope_payload_descriptors = Vec::new();
        for (index, scope) in code.scopes.iter().enumerate() {
            let descriptor = self
                .builder
                .build_alloca(ptr, "opened.descriptor.slot")
                .map_err(|e| Self::erased_error(e.to_string()))?;
            let table = self
                .builder
                .build_alloca(ptr, "opened.table.slot")
                .map_err(|e| Self::erased_error(e.to_string()))?;
            layouts.insert(MirErasedType::Scoped(index as u32), descriptor);
            scope_tables.push(table);
            let payload_descriptor = self
                .builder
                .build_alloca(ptr, "package.payload.descriptor.slot")
                .map_err(|e| Self::erased_error(e.to_string()))?;
            if let MirErasedScopeSource::Package { package } = scope.source {
                let payload = scoped_package_payload(
                    package,
                    index as u32,
                    scope.witness,
                    &mut self.type_context().clone(),
                )
                .map_err(|error| Self::erased_error(format!("{error:?}")))?;
                layouts.insert(payload, payload_descriptor);
            }
            scope_payload_descriptors.push(payload_descriptor);
        }
        let mut locals = Vec::new();
        let mut initialized = Vec::new();
        for (index, ty) in code.locals.iter().enumerate() {
            let flag = self
                .builder
                .build_alloca(self.context.bool_type(), "erased.live")
                .map_err(|e| Self::erased_error(e.to_string()))?;
            self.builder
                .build_store(
                    flag,
                    self.context
                        .bool_type()
                        .const_int(u64::from(index < body.signature.params.len()), false),
                )
                .map_err(|e| Self::erased_error(e.to_string()))?;
            initialized.push(flag);
            let address = if index < body.signature.params.len() {
                function
                    .get_nth_param((args_base + index) as u32)
                    .ok_or_else(|| Self::erased_error("erased value parameter missing"))?
                    .into_pointer_value()
            } else {
                let mut scoped = std::collections::BTreeSet::new();
                ty.scoped_binders(&mut scoped);
                if scoped.is_empty() {
                    self.allocate_erased_local(self.erased_layout(ty, &layouts)?)?
                } else {
                    ptr.const_null()
                }
            };
            let slot = self
                .builder
                .build_alloca(ptr, "erased.local.slot")
                .map_err(|e| Self::erased_error(e.to_string()))?;
            self.builder
                .build_store(slot, address)
                .map_err(|e| Self::erased_error(e.to_string()))?;
            locals.push(slot);
        }
        let blocks = code
            .blocks
            .iter()
            .enumerate()
            .map(|(index, _)| {
                self.context
                    .append_basic_block(function, &format!("erased.block.{index}"))
            })
            .collect::<Vec<_>>();
        self.builder
            .build_unconditional_branch(blocks[0])
            .map_err(|e| Self::erased_error(e.to_string()))?;
        for (index, block) in code.blocks.iter().enumerate() {
            self.builder.position_at_end(blocks[index]);
            for statement in &block.statements {
                let destination_place = MirErasedPlace::local(statement.destination);
                if let MirErasedRvalue::PackageOpen { scope, source } = &statement.value {
                    let evidence = &code.scopes[*scope as usize];
                    let MirErasedScopeSource::Package { package } = evidence.source else {
                        return Err(Self::erased_error("package opening has object authority"));
                    };
                    let address =
                        self.erased_argument_address(body, &locals, source, &layouts, contract)?;
                    let package_type = self.package_type(package)?;
                    let table_field = self
                        .builder
                        .build_struct_gep(package_type, address, 1, "package.open.table.address")
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                    let table = self
                        .builder
                        .build_load(ptr, table_field, "package.open.table")
                        .map_err(|e| Self::erased_error(e.to_string()))?
                        .into_pointer_value();
                    let payload_field = self
                        .builder
                        .build_struct_gep(
                            package_type,
                            address,
                            2,
                            "package.open.payload.descriptor.address",
                        )
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                    let payload_descriptor = self
                        .builder
                        .build_load(ptr, payload_field, "package.open.payload.descriptor")
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                    let descriptor_field = self
                        .builder
                        .build_struct_gep(
                            self.object_table_type(&contract.object_schemas[&evidence.object]),
                            table,
                            5,
                            "package.open.witness.descriptor.address",
                        )
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                    let witness_descriptor = self
                        .builder
                        .build_load(ptr, descriptor_field, "package.open.witness.descriptor")
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                    self.builder
                        .build_store(scope_tables[*scope as usize], table)
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                    self.builder
                        .build_store(layouts[&MirErasedType::Scoped(*scope)], witness_descriptor)
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                    self.builder
                        .build_store(
                            scope_payload_descriptors[*scope as usize],
                            payload_descriptor,
                        )
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                }
                let mut scoped = std::collections::BTreeSet::new();
                code.locals[statement.destination.0 as usize].scoped_binders(&mut scoped);
                let destination = if scoped.is_empty() {
                    self.erased_place_address(
                        body,
                        &locals,
                        &destination_place,
                        &layouts,
                        contract,
                    )?
                } else {
                    // The checked Open dominates every descriptor-sized allocation.
                    let address = self.allocate_erased_local(self.erased_layout(
                        &code.locals[statement.destination.0 as usize],
                        &layouts,
                    )?)?;
                    self.builder
                        .build_store(locals[statement.destination.0 as usize], address)
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                    address
                };
                match &statement.value {
                    MirErasedRvalue::PackageOpen { scope, source } => {
                        let MirErasedScopeSource::Package { package } =
                            code.scopes[*scope as usize].source
                        else {
                            return Err(Self::erased_error("invalid package scope"));
                        };
                        let source_address = self
                            .erased_argument_address(body, &locals, source, &layouts, contract)?;
                        let payload = self
                            .builder
                            .build_struct_gep(
                                self.package_type(package)?,
                                source_address,
                                0,
                                "package.open.payload",
                            )
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        let descriptor = self
                            .builder
                            .build_load(
                                ptr,
                                scope_payload_descriptors[*scope as usize],
                                "package.open.actual.descriptor",
                            )
                            .map_err(|e| Self::erased_error(e.to_string()))?
                            .into_pointer_value();
                        self.move_erased_bytes(destination, payload, descriptor)?;
                        let MirErasedOperand::Move(place) = source else {
                            return Err(Self::erased_error("package open must consume its source"));
                        };
                        self.builder
                            .build_store(
                                initialized[place.local.0 as usize],
                                self.context.bool_type().const_zero(),
                            )
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                    }
                    MirErasedRvalue::PackagePack { payload, plan } => {
                        let package_type = self.package_type(plan.package)?;
                        let payload_address = self
                            .builder
                            .build_struct_gep(package_type, destination, 0, "package.pack.payload")
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        let source_address = self
                            .erased_argument_address(body, &locals, payload, &layouts, contract)?;
                        let descriptor = match plan.payload_descriptor {
                            MirPackagePayloadDescriptor::Concrete(id) => {
                                self.erased_descriptors[&id].as_pointer_value()
                            }
                            MirPackagePayloadDescriptor::UniformDropFree => self
                                .module
                                .get_global(&format!("rock.uniform.package.{}", plan.package.0))
                                .ok_or_else(|| {
                                    Self::erased_error("package uniform descriptor missing")
                                })?
                                .as_pointer_value(),
                            MirPackagePayloadDescriptor::FromPackage(scope) => self
                                .builder
                                .build_load(
                                    ptr,
                                    scope_payload_descriptors[scope as usize],
                                    "package.repack.actual.descriptor",
                                )
                                .map_err(|e| Self::erased_error(e.to_string()))?
                                .into_pointer_value(),
                        };
                        self.move_erased_bytes(payload_address, source_address, descriptor)?;
                        let table = match plan.witness {
                            MirPackageWitnessEvidence::Concrete { table, .. } => {
                                self.object_vtables[&table].as_pointer_value()
                            }
                            MirPackageWitnessEvidence::Scoped { scope, view } => {
                                let table = self
                                    .builder
                                    .build_load(
                                        ptr,
                                        scope_tables[scope as usize],
                                        "package.repack.table",
                                    )
                                    .map_err(|e| Self::erased_error(e.to_string()))?
                                    .into_pointer_value();
                                if !crate::mir::object::object_view_is_admitted(
                                    self.type_context(),
                                    code.scopes[scope as usize].object,
                                    view,
                                ) {
                                    return Err(Self::erased_error("package repack view missing"));
                                }
                                table
                            }
                        };
                        let table_field = self
                            .builder
                            .build_struct_gep(
                                package_type,
                                destination,
                                1,
                                "package.pack.table.address",
                            )
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        let descriptor_field = self
                            .builder
                            .build_struct_gep(
                                package_type,
                                destination,
                                2,
                                "package.pack.descriptor.address",
                            )
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        self.builder
                            .build_store(table_field, table)
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        self.builder
                            .build_store(descriptor_field, descriptor)
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        if let MirErasedOperand::Move(place) = payload {
                            self.builder
                                .build_store(
                                    initialized[place.local.0 as usize],
                                    self.context.bool_type().const_zero(),
                                )
                                .map_err(|e| Self::erased_error(e.to_string()))?;
                        }
                    }
                    MirErasedRvalue::Open { scope, source } => {
                        let evidence = &code.scopes[*scope as usize];
                        let address = self
                            .erased_argument_address(body, &locals, source, &layouts, contract)?;
                        let value = self
                            .builder
                            .build_load(self.object_handle_type(), address, "opened.object")
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        let (data, metadata) = self.object_parts(value)?;
                        let schema = &contract.object_schemas[&evidence.object];
                        let field = self
                            .builder
                            .build_struct_gep(
                                self.object_table_type(schema),
                                metadata,
                                5,
                                "opened.descriptor.address",
                            )
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        let descriptor = self
                            .builder
                            .build_load(ptr, field, "opened.payload.descriptor")
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        self.builder
                            .build_store(layouts[&MirErasedType::Scoped(*scope)], descriptor)
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        self.builder
                            .build_store(scope_tables[*scope as usize], metadata)
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        self.builder
                            .build_store(destination, data)
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        if let MirErasedOperand::Move(place) = source {
                            self.builder
                                .build_store(
                                    initialized[place.local.0 as usize],
                                    self.context.bool_type().const_zero(),
                                )
                                .map_err(|e| Self::erased_error(e.to_string()))?;
                        }
                    }
                    MirErasedRvalue::Reclose {
                        scope,
                        value,
                        object,
                    } => {
                        let source =
                            self.erased_argument_address(body, &locals, value, &layouts, contract)?;
                        let data = self
                            .builder
                            .build_load(ptr, source, "reclosed.data")
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        let metadata = self
                            .builder
                            .build_load(ptr, scope_tables[*scope as usize], "reclosed.table")
                            .map_err(|e| Self::erased_error(e.to_string()))?
                            .into_pointer_value();
                        if !crate::mir::object::object_view_is_admitted(
                            self.type_context(),
                            code.scopes[*scope as usize].object,
                            *object,
                        ) {
                            return Err(Self::erased_error(
                                "reclose has no dominating view evidence",
                            ));
                        }
                        let value_handle = self
                            .builder
                            .build_insert_value(
                                self.object_handle_type().get_undef(),
                                data,
                                0,
                                "reclosed.data.handle",
                            )
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        let value_handle = self
                            .builder
                            .build_insert_value(value_handle, metadata, 1, "reclosed.handle")
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        self.builder
                            .build_store(destination, value_handle.into_struct_value())
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        if let MirErasedOperand::Move(place) = value {
                            self.builder
                                .build_store(
                                    initialized[place.local.0 as usize],
                                    self.context.bool_type().const_zero(),
                                )
                                .map_err(|e| Self::erased_error(e.to_string()))?;
                        }
                    }
                    MirErasedRvalue::OpenedCall {
                        scope,
                        slot,
                        receiver,
                        args,
                    } => {
                        let schema = &contract.object_schemas[&code.scopes[*scope as usize].object];
                        let member = &schema.slots[*slot as usize];
                        let metadata = self
                            .builder
                            .build_load(ptr, scope_tables[*scope as usize], "opened.call.table")
                            .map_err(|e| Self::erased_error(e.to_string()))?
                            .into_pointer_value();
                        let target = self.load_object_entry(metadata, schema, 3, *slot)?;
                        let receiver =
                            self.erased_place_address(body, &locals, receiver, &layouts, contract)?;
                        let mut values: Vec<BasicMetadataValueEnum> = vec![receiver.into()];
                        for (operand, parameter) in
                            args.iter().zip(member.signature.params.iter().skip(1))
                        {
                            let address = self.erased_argument_address(
                                body, &locals, operand, &layouts, contract,
                            )?;
                            values.push(self.erased_native_argument(address, parameter)?);
                            if let MirErasedOperand::Move(place) = operand {
                                self.builder
                                    .build_store(
                                        initialized[place.local.0 as usize],
                                        self.context.bool_type().const_zero(),
                                    )
                                    .map_err(|e| Self::erased_error(e.to_string()))?;
                            }
                        }
                        let result = self
                            .builder
                            .build_indirect_call(
                                self.virtual_function_type(member),
                                target,
                                &values,
                                "opened.call",
                            )
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        if let Some(value) = result.try_as_basic_value().left() {
                            self.builder
                                .build_store(destination, value)
                                .map_err(|e| Self::erased_error(e.to_string()))?;
                        }
                    }
                    MirErasedRvalue::OpenedErasedCall {
                        scope,
                        slot,
                        type_args,
                        dictionaries,
                        receiver,
                        args,
                    } => {
                        let schema = &contract.object_schemas[&code.scopes[*scope as usize].object];
                        let signature = &schema.erased_slots[*slot as usize].signature;
                        let metadata = self
                            .builder
                            .build_load(ptr, scope_tables[*scope as usize], "opened.erased.table")
                            .map_err(|e| Self::erased_error(e.to_string()))?
                            .into_pointer_value();
                        let target = self.load_object_entry(metadata, schema, 6, *slot)?;
                        let mut values: Vec<BasicMetadataValueEnum> = vec![destination.into()];
                        for layout in &signature.layouts {
                            let ty = layout
                                .substitute(type_args)
                                .map_err(|e| Self::erased_error(format!("{e:?}")))?;
                            values.push(self.erased_layout(&ty, &layouts)?.into());
                        }
                        for dictionary in dictionaries {
                            values.push(
                                (*body_dictionaries.get(*dictionary as usize).ok_or_else(
                                    || Self::erased_error("opened erased dictionary missing"),
                                )?)
                                .into(),
                            );
                        }
                        // A borrowed erased receiver is an address of its thin
                        // reference value, not an address of an object handle.
                        let data =
                            self.erased_place_address(body, &locals, receiver, &layouts, contract)?;
                        let reference = self
                            .builder
                            .build_alloca(ptr, "opened.erased.receiver")
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        self.builder
                            .build_store(reference, data)
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        values.push(reference.into());
                        for operand in args {
                            values.push(
                                self.erased_argument_address(
                                    body, &locals, operand, &layouts, contract,
                                )?
                                .into(),
                            );
                            if let MirErasedOperand::Move(place) = operand {
                                self.builder
                                    .build_store(
                                        initialized[place.local.0 as usize],
                                        self.context.bool_type().const_zero(),
                                    )
                                    .map_err(|e| Self::erased_error(e.to_string()))?;
                            }
                        }
                        self.builder
                            .build_indirect_call(
                                self.erased_function_type(signature),
                                target,
                                &values,
                                "opened.erased.call",
                            )
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                    }
                    MirErasedRvalue::NativeCall { target, args }
                    | MirErasedRvalue::NativeApply { target, args } => {
                        let declaration = contract.callable(target).ok_or_else(|| {
                            Self::erased_error("native erased bridge target missing")
                        })?;
                        let target = self.adapter_target(target)?;
                        let mut values = Vec::with_capacity(args.len());
                        for (operand, parameter) in args.iter().zip(&declaration.signature.params) {
                            let address = self.erased_argument_address(
                                body, &locals, operand, &layouts, contract,
                            )?;
                            values.push(self.erased_native_argument(address, parameter)?);
                            if let MirErasedOperand::Move(place) = operand {
                                self.builder
                                    .build_store(
                                        initialized[place.local.0 as usize],
                                        self.context.bool_type().const_zero(),
                                    )
                                    .map_err(|e| Self::erased_error(e.to_string()))?;
                            }
                        }
                        let result = self
                            .builder
                            .build_call(target, &values, "erased.native.call")
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                        if let Some(value) = result.try_as_basic_value().left() {
                            self.builder
                                .build_store(destination, value)
                                .map_err(|e| Self::erased_error(e.to_string()))?;
                        } else if matches!(
                            self.structural_type_for(declaration.signature.ret.semantic_ty),
                            crate::types::Type::Unit
                        ) {
                            self.builder
                                .build_store(destination, self.context.i64_type().const_zero())
                                .map_err(|e| Self::erased_error(e.to_string()))?;
                        } else {
                            return Err(Self::erased_error(
                                "native erased bridge lost its result value",
                            ));
                        }
                    }
                    MirErasedRvalue::Use(operand) => {
                        let place = match operand {
                            MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
                        };
                        let source =
                            self.erased_place_address(body, &locals, place, &layouts, contract)?;
                        self.copy_erased_operand_bytes(
                            destination,
                            source,
                            self.erased_layout(
                                &code.locals[statement.destination.0 as usize],
                                &layouts,
                            )?,
                            operand,
                            &code.locals[statement.destination.0 as usize],
                            &layouts,
                        )?;
                        if matches!(operand, MirErasedOperand::Move(_)) {
                            self.builder
                                .build_store(
                                    initialized[place.local.0 as usize],
                                    self.context.bool_type().const_zero(),
                                )
                                .map_err(|e| Self::erased_error(e.to_string()))?;
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
                    MirErasedRvalue::Aggregate(operands)
                    | MirErasedRvalue::NativeAggregate {
                        fields: operands, ..
                    } => {
                        let descriptor = self.erased_layout(
                            &code.locals[statement.destination.0 as usize],
                            &layouts,
                        )?;
                        self.initialize_erased_fields(
                            body,
                            &locals,
                            &initialized,
                            &layouts,
                            contract,
                            destination,
                            descriptor,
                            operands.iter(),
                        )?;
                    }
                    MirErasedRvalue::NominalAggregate { fields, .. } => {
                        let descriptor = self.erased_layout(
                            &code.locals[statement.destination.0 as usize],
                            &layouts,
                        )?;
                        self.initialize_erased_fields(
                            body,
                            &locals,
                            &initialized,
                            &layouts,
                            contract,
                            destination,
                            descriptor,
                            fields.iter().map(|(_, value)| value),
                        )?;
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
                                (*body_dictionaries.get(*dictionary as usize).ok_or_else(
                                    || Self::erased_error("erased dictionary argument missing"),
                                )?)
                                .into(),
                            );
                        }
                        for operand in args {
                            let place = match operand {
                                MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => {
                                    place
                                }
                            };
                            values.push(
                                self.erased_argument_address(
                                    body, &locals, operand, &layouts, contract,
                                )?
                                .into(),
                            );
                            if matches!(operand, MirErasedOperand::Move(_)) {
                                self.builder
                                    .build_store(
                                        initialized[place.local.0 as usize],
                                        self.context.bool_type().const_zero(),
                                    )
                                    .map_err(|e| Self::erased_error(e.to_string()))?;
                            }
                        }
                        self.builder
                            .build_call(self.erased_functions[target], &values, "erased.call")
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                    }
                    MirErasedRvalue::DictionaryCall {
                        dictionary,
                        member,
                        type_args,
                        dictionaries,
                        receiver,
                        args,
                    } => {
                        let table = *body_dictionaries
                            .get(*dictionary as usize)
                            .ok_or_else(|| Self::erased_error("dictionary environment missing"))?;
                        let entries = self.dictionary_environment_array(table, 8)?;
                        let pointer = unsafe {
                            self.builder.build_gep(
                                self.context.ptr_type(AddressSpace::default()),
                                entries,
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
                        let mut values: Vec<BasicMetadataValueEnum> =
                            vec![destination.into(), table.into()];
                        if let Some(receiver) = receiver {
                            values.push(
                                self.erased_place_address(
                                    body, &locals, receiver, &layouts, contract,
                                )?
                                .into(),
                            );
                        }
                        for ty in type_args {
                            values.push(self.erased_layout(ty, &layouts)?.into());
                        }
                        for index in dictionaries {
                            values.push(
                                body_dictionaries
                                    .get(*index as usize)
                                    .ok_or_else(|| {
                                        Self::erased_error("member dictionary evidence missing")
                                    })?
                                    .to_owned()
                                    .into(),
                            );
                        }
                        if dictionary_schemas[*dictionary as usize].members[*member as usize]
                            .receiver
                            == Some(crate::types::ReceiverMode::Move)
                        {
                            let receiver = receiver.as_ref().ok_or_else(|| {
                                Self::erased_error("consuming dictionary receiver missing")
                            })?;
                            self.builder
                                .build_store(
                                    initialized[receiver.local.0 as usize],
                                    self.context.bool_type().const_zero(),
                                )
                                .map_err(|e| Self::erased_error(e.to_string()))?;
                        }
                        for operand in args {
                            let place = match operand {
                                MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => {
                                    place
                                }
                            };
                            values.push(
                                self.erased_argument_address(
                                    body, &locals, operand, &layouts, contract,
                                )?
                                .into(),
                            );
                            if matches!(operand, MirErasedOperand::Move(_)) {
                                self.builder
                                    .build_store(
                                        initialized[place.local.0 as usize],
                                        self.context.bool_type().const_zero(),
                                    )
                                    .map_err(|e| Self::erased_error(e.to_string()))?;
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
                self.builder
                    .build_store(
                        initialized[statement.destination.0 as usize],
                        self.context.bool_type().const_int(1, false),
                    )
                    .map_err(|e| Self::erased_error(e.to_string()))?;
            }
            match &block.terminator {
                MirErasedTerminator::Cleanup {
                    locals: cleanup,
                    target,
                } => {
                    for local in crate::mir::cleanup::cleanup_order(cleanup) {
                        self.cleanup_erased_local(
                            body,
                            local,
                            &locals,
                            &initialized,
                            &layouts,
                            contract,
                        )?;
                    }
                    self.builder
                        .build_unconditional_branch(blocks[target.0 as usize])
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                }
                MirErasedTerminator::Goto(target) => {
                    self.builder
                        .build_unconditional_branch(blocks[target.0 as usize])
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                }
                MirErasedTerminator::Branch {
                    condition,
                    then_block,
                    else_block,
                } => {
                    let address =
                        self.erased_argument_address(body, &locals, condition, &layouts, contract)?;
                    let value = self
                        .builder
                        .build_load(self.context.bool_type(), address, "erased.condition")
                        .map_err(|e| Self::erased_error(e.to_string()))?
                        .into_int_value();
                    if let MirErasedOperand::Move(place) = condition {
                        self.builder
                            .build_store(
                                initialized[place.local.0 as usize],
                                self.context.bool_type().const_zero(),
                            )
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                    }
                    self.builder
                        .build_conditional_branch(
                            value,
                            blocks[then_block.0 as usize],
                            blocks[else_block.0 as usize],
                        )
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                }
                MirErasedTerminator::Unreachable => {
                    self.builder
                        .build_unreachable()
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                }
                MirErasedTerminator::Return(returned) => {
                    let result = match returned {
                        MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
                    };
                    self.copy_erased_operand_bytes(
                        out,
                        self.erased_place_address(body, &locals, result, &layouts, contract)?,
                        self.erased_layout(&body.signature.ret, &layouts)?,
                        returned,
                        &body.signature.ret,
                        &layouts,
                    )?;
                    if matches!(returned, MirErasedOperand::Move(_)) {
                        self.builder
                            .build_store(
                                initialized[result.local.0 as usize],
                                self.context.bool_type().const_zero(),
                            )
                            .map_err(|e| Self::erased_error(e.to_string()))?;
                    }
                    let remaining = (0..initialized.len())
                        .map(|index| MirErasedLocal(index as u32))
                        .collect::<Vec<_>>();
                    for local in crate::mir::cleanup::cleanup_order(&remaining) {
                        self.cleanup_erased_local(
                            body,
                            local,
                            &locals,
                            &initialized,
                            &layouts,
                            contract,
                        )?;
                    }
                    self.builder
                        .build_return(None)
                        .map_err(|e| Self::erased_error(e.to_string()))?;
                }
            }
        }
        Ok(())
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
                let value = self.copy_mir_value(value, &self.structural_type_for(ty))?;
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
                let value = self.copy_mir_value(value, &self.structural_type_for(ty))?;
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
