use inkwell::types::{BasicMetadataTypeEnum, BasicType, FunctionType, StructType};
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::{module::Linkage, AddressSpace};

use super::MirFunctionContext;
use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{
    MirBackendContract, MirCallable, MirFunction, MirObjectConversion, MirObjectSchema,
    MirObjectSlot, MirVirtualTarget, Operand, Place,
};
use crate::types::Type;

#[path = "object_lookup.rs"]
mod lookup;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn compile_object_metadata_intrinsic(
        &mut self,
        intrinsic: crate::mir::MirIntrinsicId,
        value: BasicValueEnum<'ctx>,
        object: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        use crate::mir::MirIntrinsicId;
        let schema = self
            .object_schemas
            .values()
            .find(|schema| self.structural_type_for(schema.object) == *object)
            .cloned()
            .ok_or_else(|| CodegenError::backend_contract("object metadata schema missing"))?;
        let (data, metadata) = self.object_parts(value)?;
        let field = match intrinsic {
            MirIntrinsicId::SizeOfValue => 0,
            MirIntrinsicId::AlignOfValue => 1,
            MirIntrinsicId::DropInPlace => 2,
            _ => {
                return Err(CodegenError::backend_contract(
                    "invalid object metadata intrinsic",
                ))
            }
        };
        let address = self
            .builder
            .build_struct_gep(
                self.object_table_type(&schema),
                metadata,
                field,
                "object.layout.address",
            )
            .map_err(|e| CodegenError::from(e.to_string()))?;
        if intrinsic != MirIntrinsicId::DropInPlace {
            return self
                .builder
                .build_load(self.context.i64_type(), address, "object.layout")
                .map(Some)
                .map_err(|e| CodegenError::from(e.to_string()));
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let destructor = self
            .builder
            .build_load(ptr, address, "object.payload.drop")
            .map_err(|e| CodegenError::from(e.to_string()))?
            .into_pointer_value();
        let function = self
            .builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .ok_or_else(|| CodegenError::backend_contract("object drop outside a function"))?;
        let drop_block = self.context.append_basic_block(function, "object.drop");
        let continuation = self
            .context
            .append_basic_block(function, "object.drop.done");
        let present = self
            .builder
            .build_is_not_null(destructor, "object.has.drop")
            .map_err(|e| CodegenError::from(e.to_string()))?;
        self.builder
            .build_conditional_branch(present, drop_block, continuation)
            .map_err(|e| CodegenError::from(e.to_string()))?;
        self.builder.position_at_end(drop_block);
        self.builder
            .build_indirect_call(
                self.context.void_type().fn_type(&[ptr.into()], false),
                destructor,
                &[data.into()],
                "",
            )
            .map_err(|e| CodegenError::from(e.to_string()))?;
        self.builder
            .build_unconditional_branch(continuation)
            .map_err(|e| CodegenError::from(e.to_string()))?;
        self.builder.position_at_end(continuation);
        Ok(None)
    }

    pub(crate) fn object_handle_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(&[ptr.into(), ptr.into()], false)
    }

    pub(super) fn object_table_type(&self, _schema: &MirObjectSchema) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(
            &[
                self.context.i64_type().into(), // target size constant expression
                self.context.i64_type().into(), // target alignment constant expression
                ptr.into(),                     // payload destructor, never deallocator
                ptr.into(),                     // independent native member evidence
                self.context.i64_type().into(), // native member count
                ptr.into(),                     // concrete witness descriptor for erased entries
                ptr.into(),                     // independent erased member evidence
                self.context.i64_type().into(), // erased member count
            ],
            false,
        )
    }

    pub(crate) fn emit_object_vtables(
        &mut self,
        contract: &MirBackendContract,
    ) -> Result<(), CodegenError> {
        self.object_schemas = contract.object_schemas.clone();
        self.owned_object_calls = contract.owned_object_calls.clone();
        if !contract.object_schemas.is_empty() {
            self.emit_object_lookup()?;
        }
        for (id, schema) in &contract.object_schemas {
            for (field, count) in [(3, schema.slots.len()), (6, schema.erased_slots.len())] {
                for index in 0..count {
                    let key = crate::mir::object_identity::member_key(
                        contract,
                        self.type_context(),
                        schema,
                        field,
                        index,
                    )
                    .map_err(CodegenError::backend_contract)?;
                    let value = self.context.const_string(&key, false);
                    let global = self.module.add_global(
                        value.get_type(),
                        None,
                        &format!("rock.object.key.{}.{}.{}", id.0, field, index),
                    );
                    global.set_linkage(Linkage::Private);
                    global.set_constant(true);
                    global.set_initializer(&value);
                    self.object_entry_keys
                        .insert((*id, field, index as u32), (global, key.len() as u64));
                }
            }
        }
        // The runtime table has a fixed prefix, independent of schema width.
        for (id, table) in &contract.vtables {
            let schema = contract
                .object_schemas
                .get(&table.object)
                .ok_or_else(|| CodegenError::backend_contract("vtable schema missing"))?;
            let global = self.module.add_global(
                self.object_table_type(schema),
                None,
                &format!("rock.vtable.{}", id.0),
            );
            global.set_linkage(Linkage::Private);
            global.set_constant(true);
            self.object_vtables.insert(*id, global);
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        for (id, table) in &contract.vtables {
            let schema = &contract.object_schemas[&table.object];
            let payload = self.llvm_type(&self.structural_type_for(table.concrete));
            let size = payload
                .size_of()
                .ok_or_else(|| CodegenError::layout("unsized object payload"))?;
            let align = self.context.struct_type(&[payload], false).get_alignment();
            let resolve =
                |key: &crate::mir::MirCallableKey| -> Result<PointerValue<'ctx>, CodegenError> {
                    let symbol =
                        self.resolve_mir_callable_symbol(&MirCallable::Resolved(key.clone()))?;
                    self.functions
                        .get(&symbol)
                        .copied()
                        .or_else(|| self.module.get_function(&symbol))
                        .map(|function| function.as_global_value().as_pointer_value())
                        .ok_or_else(|| {
                            CodegenError::backend_contract(
                                "vtable entry has no declared link symbol",
                            )
                        })
                };
            let methods = table
                .methods
                .iter()
                .map(resolve)
                .collect::<Result<Vec<_>, _>>()?;
            let drop = table
                .drop
                .as_ref()
                .map(resolve)
                .transpose()?
                .unwrap_or_else(|| ptr.const_null());
            let native = self.emit_object_entries(table.object, 3, &methods, id.0)?;
            let erased_methods = table
                .erased_methods
                .iter()
                .map(|key| {
                    self.erased_functions
                        .get(key)
                        .map(|function| function.as_global_value().as_pointer_value())
                        .ok_or_else(|| {
                            CodegenError::backend_contract("erased vtable entry missing")
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let erased = self.emit_object_entries(table.object, 6, &erased_methods, id.0)?;
            let value = self.object_table_type(schema).const_named_struct(&[
                size.into(),
                align.into(),
                drop.into(),
                native.into(),
                self.context
                    .i64_type()
                    .const_int(methods.len() as u64, false)
                    .into(),
                self.erased_descriptors
                    .get(&table.concrete)
                    .map(|descriptor| descriptor.as_pointer_value())
                    .ok_or_else(|| {
                        CodegenError::backend_contract(
                            "object producer table requires its concrete witness descriptor",
                        )
                    })?
                    .into(),
                erased.into(),
                self.context
                    .i64_type()
                    .const_int(erased_methods.len() as u64, false)
                    .into(),
            ]);
            self.object_vtables[id].set_initializer(&value);
        }
        Ok(())
    }

    pub(super) fn object_parts(
        &self,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>), CodegenError> {
        let BasicValueEnum::StructValue(value) = value else {
            return Err(CodegenError::backend_contract(
                "object handle must be a data/vtable pair",
            ));
        };
        let data = self
            .builder
            .build_extract_value(value, 0, "object.data")
            .map_err(|e| CodegenError::from(e.to_string()))?;
        let metadata = self
            .builder
            .build_extract_value(value, 1, "object.metadata")
            .map_err(|e| CodegenError::from(e.to_string()))?;
        Ok((data.into_pointer_value(), metadata.into_pointer_value()))
    }

    pub(super) fn load_object_entry(
        &self,
        metadata: PointerValue<'ctx>,
        schema: &MirObjectSchema,
        field: u32,
        index: u32,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let count_field = match field {
            3 => 4,
            6 => 7,
            _ => {
                return Err(CodegenError::backend_contract(
                    "invalid object evidence lookup",
                ))
            }
        };
        let array = self
            .builder
            .build_struct_gep(
                self.object_table_type(schema),
                metadata,
                field,
                "object.entries.address",
            )
            .map_err(|e| CodegenError::from(e.to_string()))?;
        let array = self
            .builder
            .build_load(ptr, array, "object.entries")
            .map_err(|e| CodegenError::from(e.to_string()))?;
        let count = self
            .builder
            .build_struct_gep(
                self.object_table_type(schema),
                metadata,
                count_field,
                "object.entries.count.address",
            )
            .map_err(|e| CodegenError::from(e.to_string()))?;
        let count = self
            .builder
            .build_load(self.context.i64_type(), count, "object.entries.count")
            .map_err(|e| CodegenError::from(e.to_string()))?;
        let (key, length) = self
            .object_entry_keys
            .get(&(schema.object, field, index))
            .ok_or_else(|| CodegenError::backend_contract("object member identity missing"))?;
        let lookup = self
            .module
            .get_function("__rock_object_lookup")
            .ok_or_else(|| CodegenError::backend_contract("object lookup function missing"))?;
        self.builder
            .build_call(
                lookup,
                &[
                    array.into(),
                    count.into(),
                    key.as_pointer_value().into(),
                    self.context.i64_type().const_int(*length, false).into(),
                ],
                "object.entry.load",
            )
            .map_err(|e| CodegenError::from(e.to_string()))?
            .try_as_basic_value()
            .left()
            .map(|value| value.into_pointer_value())
            .ok_or_else(|| CodegenError::backend_contract("object lookup returned no entry"))
    }

    pub(super) fn compile_object_conversion(
        &mut self,
        function: &MirFunction,
        ctx: &MirFunctionContext<'ctx>,
        operand: &Operand,
        conversion: &MirObjectConversion,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let value = self.compile_mir_operand(function, ctx, operand)?;
        let (data, metadata) = match conversion {
            MirObjectConversion::Concrete(id) => {
                let BasicValueEnum::PointerValue(data) = value else {
                    return Err(CodegenError::backend_contract(
                        "concrete erasure requires a thin borrowed pointer",
                    ));
                };
                let table = self.object_vtables.get(id).ok_or_else(|| {
                    CodegenError::backend_contract("object construction vtable missing")
                })?;
                (data, table.as_pointer_value())
            }
            // A view narrows static authority, not the producer's evidence.
            // Subsequent members are selected by exact portable identity.
            MirObjectConversion::Upcast { .. } => self.object_parts(value)?,
        };
        let value = self
            .builder
            .build_insert_value(
                self.object_handle_type().get_undef(),
                data,
                0,
                "object.pack.data",
            )
            .map_err(|e| CodegenError::from(e.to_string()))?;
        self.builder
            .build_insert_value(value, metadata, 1, "object.pack.metadata")
            .map(|v| v.into_struct_value().into())
            .map_err(|e| CodegenError::from(e.to_string()))
    }

    pub(super) fn virtual_function_type(&self, slot: &MirObjectSlot) -> FunctionType<'ctx> {
        let mut params: Vec<BasicMetadataTypeEnum<'ctx>> =
            vec![self.context.ptr_type(AddressSpace::default()).into()];
        params.extend(
            slot.signature
                .params
                .iter()
                .skip(1)
                .map(|parameter| self.llvm_param_type(parameter)),
        );
        if matches!(
            slot.result,
            crate::mir::MirObjectResult::BorrowedSelf { .. }
        ) {
            return self
                .context
                .ptr_type(AddressSpace::default())
                .fn_type(&params, false);
        }
        let ret = self.structural_type_for(slot.signature.ret.abi_ty);
        if matches!(&ret, Type::Unit | Type::Never) {
            self.context.void_type().fn_type(&params, false)
        } else {
            self.llvm_type(&ret).fn_type(&params, false)
        }
    }

    pub(super) fn compile_owned_object_call(
        &mut self,
        function: &MirFunction,
        ctx: &MirFunctionContext<'ctx>,
        key: &crate::mir::MirOwnedObjectKey,
        args: &[Operand],
        destination: &Place,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let plan = self
            .owned_object_calls
            .get(key)
            .cloned()
            .ok_or_else(|| CodegenError::backend_contract("owned call plan missing"))?;
        let schema = self.object_schemas[&key.object].clone();
        let split_callable = MirCallable::Resolved(plan.into_parts.clone());
        let signature = self
            .resolve_mir_callable_signature(&split_callable)
            .ok_or_else(|| CodegenError::backend_contract("owner split signature missing"))?
            .clone();
        let split_symbol = self.resolve_mir_callable_symbol(&split_callable)?;
        let split = self
            .module
            .get_function(&split_symbol)
            .ok_or_else(|| CodegenError::backend_contract("owner split callable missing"))?;
        let owner =
            self.compile_mir_call_args(function, ctx, &args[..1], Some(&signature.params))?;
        let parts = self
            .builder
            .build_call(split, &owner, "owner.parts")
            .map_err(|e| CodegenError::from(e.to_string()))?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::backend_contract("owner split returned void"))?
            .into_struct_value();
        let handle = self
            .builder
            .build_extract_value(parts, 0, "owner.handle")
            .map_err(|e| CodegenError::from(e.to_string()))?;
        let state = self
            .builder
            .build_extract_value(parts, 1, "owner.state")
            .map_err(|e| CodegenError::from(e.to_string()))?;
        let (data, metadata) = self.object_parts(handle)?;
        let result = match &key.slot {
            crate::mir::MirOwnedObjectSlot::Concrete(index) => {
                let slot = &schema.slots[*index as usize];
                let entry = self.load_object_entry(metadata, &schema, 3, *index)?;
                let mut values = vec![data.into()];
                values.extend(self.compile_mir_call_args(
                    function,
                    ctx,
                    &args[1..],
                    Some(&slot.signature.params[1..]),
                )?);
                self.builder
                    .build_indirect_call(
                        self.virtual_function_type(slot),
                        entry,
                        &values,
                        "owner.consume",
                    )
                    .map_err(|e| CodegenError::from(e.to_string()))?
                    .try_as_basic_value()
                    .left()
            }
            crate::mir::MirOwnedObjectSlot::Erased { .. } => {
                self.compile_owned_erased_entry(
                    ctx,
                    key,
                    &plan,
                    data,
                    metadata,
                    &args[1..],
                    destination,
                )?;
                None
            }
        };
        let (_, result_ty) = self.compile_mir_place_local_id(ctx, destination)?;
        if matches!(self.structural_type_for(result_ty), Type::Never) {
            return Ok(result);
        }
        let release_symbol =
            self.resolve_mir_callable_symbol(&MirCallable::Resolved(plan.release))?;
        let release = self
            .module
            .get_function(&release_symbol)
            .ok_or_else(|| CodegenError::backend_contract("owner release callable missing"))?;
        self.builder
            .build_call(release, &[data.into(), state.into()], "")
            .map_err(|e| CodegenError::from(e.to_string()))?;
        Ok(result)
    }

    pub(super) fn compile_virtual_call(
        &mut self,
        function: &MirFunction,
        ctx: &MirFunctionContext<'ctx>,
        target: &MirVirtualTarget,
        args: &[Operand],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let schema = self
            .object_schemas
            .get(&target.object)
            .cloned()
            .ok_or_else(|| CodegenError::backend_contract("virtual schema missing"))?;
        let slot = schema
            .slots
            .get(target.slot as usize)
            .ok_or_else(|| CodegenError::backend_contract("virtual slot missing"))?;
        let receiver = args
            .first()
            .ok_or_else(|| CodegenError::backend_contract("virtual receiver missing"))?;
        let value = self.compile_mir_operand(function, ctx, receiver)?;
        let (data, metadata) = self.object_parts(value)?;
        let code = self.load_object_entry(metadata, &schema, 3, target.slot)?;
        let mut values = vec![data.into()];
        values.extend(self.compile_mir_call_args(
            function,
            ctx,
            &args[1..],
            Some(&slot.signature.params[1..]),
        )?);
        let call = self
            .builder
            .build_indirect_call(
                self.virtual_function_type(slot),
                code,
                &values,
                "object.call",
            )
            .map_err(|e| CodegenError::from(e.to_string()))?;
        let result = call.try_as_basic_value().left();
        if matches!(
            slot.result,
            crate::mir::MirObjectResult::BorrowedSelf { .. }
        ) {
            let returned_data = result.ok_or_else(|| {
                CodegenError::backend_contract("borrowed Self entry returned void")
            })?;
            let value = self
                .builder
                .build_insert_value(
                    self.object_handle_type().get_undef(),
                    returned_data,
                    0,
                    "object.return.data",
                )
                .map_err(|e| CodegenError::from(e.to_string()))?;
            return self
                .builder
                .build_insert_value(value, metadata, 1, "object.return.metadata")
                .map(|value| Some(value.into_struct_value().into()))
                .map_err(|e| CodegenError::from(e.to_string()));
        }
        Ok(result)
    }
}
