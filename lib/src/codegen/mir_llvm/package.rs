use inkwell::module::Linkage;
use inkwell::types::{BasicType, BasicTypeEnum, StructType};
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{CodeGen, CodegenError};
use crate::ids::TypeId;
use crate::mir::*;

use super::MirFunctionContext;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn uniform_layout_type(&self, layout: &MirUniformLayout) -> BasicTypeEnum<'ctx> {
        match layout {
            MirUniformLayout::Native(id) => self.llvm_type_id(*id),
            MirUniformLayout::ThinHandle => self.context.ptr_type(AddressSpace::default()).into(),
            MirUniformLayout::Tuple(fields) | MirUniformLayout::Struct { fields, .. } => self
                .context
                .struct_type(
                    &fields
                        .iter()
                        .map(|field| self.uniform_layout_type(field))
                        .collect::<Vec<_>>(),
                    false,
                )
                .into(),
            MirUniformLayout::Array { element, len } => self
                .uniform_layout_type(element)
                .array_type(*len as u32)
                .into(),
        }
    }

    pub(crate) fn package_type(&self, package: TypeId) -> Result<StructType<'ctx>, CodegenError> {
        let schema = self
            .package_schemas
            .get(&package)
            .ok_or_else(|| CodegenError::backend_contract("package schema missing"))?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        Ok(self.context.struct_type(
            &[
                self.uniform_layout_type(&schema.payload_layout),
                ptr.into(),
                ptr.into(),
            ],
            false,
        ))
    }

    pub(super) fn compile_package_pack(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        payload: &Operand,
        plan: &MirPackagePackPlan,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let MirPackageWitnessEvidence::Concrete { table, .. } = plan.witness else {
            return Err(CodegenError::backend_contract(
                "scoped package plan reached concrete MIR",
            ));
        };
        let MirPackagePayloadDescriptor::Concrete(actual) = plan.payload_descriptor else {
            return Err(CodegenError::backend_contract(
                "native package lost actual payload descriptor",
            ));
        };
        let payload = self.compile_mir_operand(function, context, payload)?;
        let table = self
            .object_vtables
            .get(&table)
            .ok_or_else(|| CodegenError::backend_contract("package witness table missing"))?
            .as_pointer_value();
        let descriptor = self
            .erased_descriptors
            .get(&actual)
            .ok_or_else(|| {
                CodegenError::backend_contract("actual package payload descriptor missing")
            })?
            .as_pointer_value();
        let package = self.package_type(plan.package)?;
        let payload_type = package
            .get_field_type_at_index(0)
            .ok_or_else(|| CodegenError::backend_contract("package payload field missing"))?;
        let payload = self.coerce_value(payload, payload_type)?;
        let value = self
            .builder
            .build_insert_value(package.get_undef(), payload, 0, "package.payload")
            .map_err(|e| CodegenError::from(e.to_string()))?;
        let value = self
            .builder
            .build_insert_value(value, table, 1, "package.witness")
            .map_err(|e| CodegenError::from(e.to_string()))?;
        self.builder
            .build_insert_value(value, descriptor, 2, "package.descriptor")
            .map(|value| value.into_struct_value().into())
            .map_err(|e| CodegenError::from(e.to_string()))
    }

    pub(super) fn drop_package_value(
        &self,
        address: PointerValue<'ctx>,
        package: TypeId,
    ) -> Result<(), CodegenError> {
        let ty = self.package_type(package)?;
        let payload = self
            .builder
            .build_struct_gep(ty, address, 0, "package.drop.payload")
            .map_err(|e| CodegenError::from(e.to_string()))?;
        let field = self
            .builder
            .build_struct_gep(ty, address, 2, "package.drop.descriptor.address")
            .map_err(|e| CodegenError::from(e.to_string()))?;
        let descriptor = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                field,
                "package.drop.descriptor",
            )
            .map_err(|e| CodegenError::from(e.to_string()))?
            .into_pointer_value();
        self.drop_erased_value(payload, descriptor)
    }

    /// Immutable, target-computed metadata; never an address of a frame-local
    /// evidence slot. Validation has proved that this layout is drop-free.
    pub(super) fn emit_uniform_descriptor(
        &self,
        layout: &MirUniformLayout,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        if let Some(global) = self.module.get_global(name) {
            return Ok(global.as_pointer_value());
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        let ty = self.uniform_layout_type(layout);
        let drop_name = "rock.uniform.drop_free";
        let drop = if let Some(drop) = self.module.get_function(drop_name) {
            drop
        } else {
            let previous = self.builder.get_insert_block();
            let drop = self.module.add_function(
                drop_name,
                self.context
                    .void_type()
                    .fn_type(&[ptr.into(), ptr.into()], false),
                Some(Linkage::Internal),
            );
            self.builder
                .position_at_end(self.context.append_basic_block(drop, "entry"));
            self.builder
                .build_return(None)
                .map_err(|e| CodegenError::from(e.to_string()))?;
            if let Some(previous) = previous {
                self.builder.position_at_end(previous);
            }
            drop
        };
        let fields = match layout {
            MirUniformLayout::Tuple(fields) | MirUniformLayout::Struct { fields, .. } => {
                fields.len()
            }
            _ => 0,
        };
        let offsets = (0..fields)
            .map(|index| {
                unsafe {
                    ptr.const_null().const_gep(
                        ty,
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
        let offsets_global =
            self.module
                .add_global(offsets_value.get_type(), None, &format!("{name}.offsets"));
        offsets_global.set_initializer(&offsets_value);
        offsets_global.set_constant(true);
        offsets_global.set_linkage(Linkage::Private);
        let descriptor = self
            .module
            .add_global(self.erased_descriptor_type(), None, name);
        descriptor.set_initializer(
            &self.erased_descriptor_type().const_named_struct(&[
                ty.size_of()
                    .ok_or_else(|| CodegenError::backend_contract("uniform layout is not sized"))?
                    .into(),
                self.context
                    .struct_type(&[ty], false)
                    .get_alignment()
                    .into(),
                drop.as_global_value().as_pointer_value().into(),
                word.const_int(fields as u64, false).into(),
                offsets_global.as_pointer_value().into(),
            ]),
        );
        descriptor.set_constant(true);
        descriptor.set_linkage(Linkage::Private);
        Ok(descriptor.as_pointer_value())
    }
}
