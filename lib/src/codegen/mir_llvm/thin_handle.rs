//! Exact thin-handle identities, frozen by the existing checked tuple interner.
use std::collections::BTreeMap;

use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{
    MirBodyFactoryRequirement, MirErasedType, MirThinHandleFactoryRequirement,
    MirTupleFactoryRequirement,
};

impl<'ctx> CodeGen<'ctx> {
    fn thin_handle_guard(
        &self,
        function: FunctionValue<'ctx>,
        valid: IntValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let ready = self.context.append_basic_block(function, "thin.handle.ready");
        let failed = self.context.append_basic_block(function, "thin.handle.failed");
        self.builder.build_conditional_branch(valid, ready, failed)
            .map_err(|error| Self::erased_error(error.to_string()))?;
        self.builder.position_at_end(failed);
        let trap = self.module.get_function("llvm.trap").unwrap_or_else(|| {
            self.module.add_function("llvm.trap", self.context.void_type().fn_type(&[], false), None)
        });
        self.builder.build_call(trap, &[], "")
            .map_err(|error| Self::erased_error(error.to_string()))?;
        self.builder.build_unreachable().map_err(|error| Self::erased_error(error.to_string()))?;
        self.builder.position_at_end(ready);
        Ok(())
    }

    fn thin_handle_offset(
        &self,
        address: PointerValue<'ctx>,
        offset: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        unsafe { self.builder.build_gep(self.context.i8_type(), address, &[offset], "thin.handle.bytes") }
            .map_err(|error| Self::erased_error(error.to_string()))
    }

    fn thin_handle_store_word(
        &self,
        address: PointerValue<'ctx>,
        offset: u64,
        value: IntValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let address = self.thin_handle_offset(address, self.context.i64_type().const_int(offset, false))?;
        let instruction = self.builder.build_store(address, value)
            .map_err(|error| Self::erased_error(error.to_string()))?;
        instruction.set_alignment(1).map_err(|error| Self::erased_error(error.to_string()))
    }

    pub(super) fn emit_thin_handle_factory(
        &self,
        function: FunctionValue<'ctx>,
        requirement: &MirThinHandleFactoryRequirement,
        layouts: &BTreeMap<MirErasedType, PointerValue<'ctx>>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let tag = requirement.tag().map_err(|error| Self::erased_error(format!("{error:?}")))?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        let pointee = self.erased_layout(&requirement.pointee, layouts)?;
        let length = self.erased_descriptor_word(pointee, 8)?;
        // Bound both nested tuple keys before allocating or copying any bytes.
        let valid = self.builder.build_int_compare(IntPredicate::ULE, length,
            word.const_int(16 * 1024 * 1024 - 128, false), "thin.handle.key.bounded")
            .map_err(|error| Self::erased_error(error.to_string()))?;
        self.thin_handle_guard(function, valid)?;
        let semantic_length = self.builder.build_int_add(length, word.const_int(29, false), "thin.handle.key.length")
            .map_err(|error| Self::erased_error(error.to_string()))?;
        let drop_length = self.builder.build_int_add(semantic_length, word.const_int(30, false), "thin.handle.drop.length")
            .map_err(|error| Self::erased_error(error.to_string()))?;
        let allocation_length = self.builder.build_int_add(semantic_length, drop_length, "thin.handle.metadata.length")
            .map_err(|error| Self::erased_error(error.to_string()))?;
        let malloc = self.module.get_function("malloc").ok_or_else(|| Self::erased_error("thin handle metadata allocator missing"))?;
        let free = self.module.get_function("free").ok_or_else(|| Self::erased_error("thin handle metadata release missing"))?;
        let semantic = self.builder.build_call(malloc, &[allocation_length.into()], "thin.handle.metadata")
            .map_err(|error| Self::erased_error(error.to_string()))?
            .try_as_basic_value().left().ok_or_else(|| Self::erased_error("metadata allocator result missing"))?
            .into_pointer_value();
        let allocated = self.builder.build_is_not_null(semantic, "thin.handle.allocated")
            .map_err(|error| Self::erased_error(error.to_string()))?;
        self.thin_handle_guard(function, allocated)?;
        // RK01: tag, empty payload, one framed pointee semantic key.
        self.thin_handle_store_word(semantic, 0, word.const_int(u32::from_le_bytes(*b"RK01") as u64 | ((tag as u64) << 32), false))?;
        self.thin_handle_store_word(semantic, 5, word.const_zero())?;
        self.thin_handle_store_word(semantic, 13, word.const_int(1, false))?;
        self.thin_handle_store_word(semantic, 21, length)?;
        let child_bytes = self.erased_descriptor_pointer(pointee, 9)?;
        let destination = self.thin_handle_offset(semantic, word.const_int(29, false))?;
        self.builder.build_memcpy(destination, 1, child_bytes, 1, length)
            .map_err(|error| Self::erased_error(error.to_string()))?;

        // RD01 n: framed semantic identity, no user destructor, no children.
        // In particular, a handle never inherits its pointee's drop plan.
        let drop_key = self.thin_handle_offset(semantic, semantic_length)?;
        self.thin_handle_store_word(drop_key, 0, word.const_int(u32::from_le_bytes(*b"RD01") as u64 | ((b'n' as u64) << 32), false))?;
        let payload_length = self.builder.build_int_add(semantic_length, word.const_int(9, false), "thin.handle.drop.payload.length")
            .map_err(|error| Self::erased_error(error.to_string()))?;
        self.thin_handle_store_word(drop_key, 5, payload_length)?;
        self.thin_handle_store_word(drop_key, 13, semantic_length)?;
        let drop_semantic = self.thin_handle_offset(drop_key, word.const_int(21, false))?;
        self.builder.build_memcpy(drop_semantic, 1, semantic, 1, semantic_length)
            .map_err(|error| Self::erased_error(error.to_string()))?;
        let suffix = self.thin_handle_offset(drop_semantic, semantic_length)?;
        self.thin_handle_store_word(suffix, 0, word.const_zero())?;
        let last = self.thin_handle_offset(suffix, word.const_int(1, false))?;
        self.thin_handle_store_word(last, 0, word.const_zero())?;

        let shape = self.selected_reference_descriptor()?;
        let drop = self.erased_descriptor_pointer(shape, 2)?;
        let target_key = self.descriptor_target_key()?;
        let target = self.module.get_global("rock.descriptor.target")
            .ok_or_else(|| Self::erased_error("thin handle target identity missing"))?.as_pointer_value();
        let values: Vec<BasicValueEnum<'ctx>> = vec![
            word.const_int(8, false).into(), word.const_int(8, false).into(), drop.into(),
            word.const_zero().into(), ptr.const_null().into(),
            word.const_int(requirement.descriptor_abi, false).into(), word.const_int(1, false).into(),
            word.const_int(2, false).into(), semantic_length.into(), semantic.into(),
            drop_length.into(), drop_key.into(), word.const_zero().into(), ptr.const_null().into(),
            word.const_int(target_key.len() as u64, false).into(), target.into(), word.const_int(i64::MAX as u64, false).into(),
        ];
        let descriptor_type = self.erased_descriptor_type();
        let mut value = descriptor_type.const_zero();
        for (index, field) in values.into_iter().enumerate() {
            value = self.builder.build_insert_value(value, field, index as u32, "thin.handle.descriptor")
                .map_err(|error| Self::erased_error(error.to_string()))?.into_struct_value();
        }
        let candidate = self.builder.build_alloca(descriptor_type, "thin.handle.candidate")
            .map_err(|error| Self::erased_error(error.to_string()))?;
        self.builder.build_store(candidate, value).map_err(|error| Self::erased_error(error.to_string()))?;
        let mut evidence = layouts.clone();
        let mut scoped = std::collections::BTreeSet::new();
        requirement.result.scoped_binders(&mut scoped);
        let candidate = if scoped.is_empty() { candidate } else {
            // Scoped layouts are late-bound descriptor slots, including this
            // temporary candidate; they are not direct descriptor addresses.
            let slot = self.builder.build_alloca(ptr, "thin.handle.candidate.slot")
                .map_err(|error| Self::erased_error(error.to_string()))?;
            self.builder.build_store(slot, candidate).map_err(|error| Self::erased_error(error.to_string()))?;
            slot
        };
        evidence.insert(requirement.result.clone(), candidate);
        // Reuse the checked interner rather than publishing caller-owned key
        // buffers or introducing an unversioned metadata/runtime protocol.
        let tuple = self.emit_body_factory(function, &MirBodyFactoryRequirement::Tuple(MirTupleFactoryRequirement {
            descriptor_abi: requirement.descriptor_abi,
            result: MirErasedType::Tuple(vec![requirement.result.clone()]),
            children: vec![requirement.result.clone()],
        }), &evidence)?;
        self.builder.build_call(free, &[semantic.into()], "")
            .map_err(|error| Self::erased_error(error.to_string()))?;
        let children = self.erased_descriptor_pointer(tuple, 13)?;
        self.builder.build_load(ptr, children, "thin.handle.frozen.descriptor")
            .map_err(|error| Self::erased_error(error.to_string()))
            .map(|value| value.into_pointer_value())
    }
}
