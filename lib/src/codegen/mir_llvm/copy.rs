//! Owning copies duplicate callable storage; borrowed reads never enter here.
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodeGen, CodegenError};
use crate::types::{CaptureKind, Type};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn copy_mir_value_requires_allocation(ty: &Type) -> bool {
        match ty {
            Type::Function { captures, .. } => !captures.is_empty(),
            Type::Tuple(fields) | Type::StructRepresentation { fields, .. } => {
                fields.iter().any(Self::copy_mir_value_requires_allocation)
            }
            Type::Exists(package) => Self::copy_mir_value_requires_allocation(&package.payload),
            _ => false,
        }
    }

    pub(super) fn copy_mir_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if !Self::copy_mir_value_requires_allocation(ty) {
            return Ok(value);
        }
        match ty {
            Type::Tuple(fields) | Type::StructRepresentation { fields, .. }
                if !fields.is_empty() =>
            {
                let mut copy = value.into_struct_value();
                for (index, field) in fields.iter().enumerate() {
                    let original = self
                        .builder
                        .build_extract_value(copy, index as u32, "copy.field")
                        .map_err(|e| CodegenError::from(e.to_string()))?;
                    let copied = self.copy_mir_value(original, field)?;
                    copy = self
                        .builder
                        .build_insert_value(copy, copied, index as u32, "copy.tuple")
                        .map_err(|e| CodegenError::from(e.to_string()))?
                        .into_struct_value();
                }
                Ok(copy.into())
            }
            Type::Function { captures, .. } if !captures.is_empty() => {
                if !ty.is_copy() {
                    return Err(CodegenError::backend_contract(
                        "owning copy of a non-Copy environment",
                    ));
                }
                let callable = value.into_struct_value();
                let original = self
                    .builder
                    .build_extract_value(callable, 1, "copy.environment")
                    .map_err(|e| CodegenError::from(e.to_string()))?
                    .into_pointer_value();
                let fields = captures
                    .iter()
                    .map(|capture| match capture.kind {
                        CaptureKind::Move => capture.ty.clone(),
                        CaptureKind::SharedBorrow => Type::Reference {
                            inner: Box::new(capture.ty.clone()),
                            mutable: false,
                        },
                        CaptureKind::MutableBorrow => {
                            unreachable!("non-Copy environment checked above")
                        }
                    })
                    .collect::<Vec<_>>();
                let layout = self.context.struct_type(
                    &fields
                        .iter()
                        .map(|field| self.llvm_type(field))
                        .collect::<Vec<_>>(),
                    false,
                );
                let malloc = self.functions.get("malloc").copied().ok_or_else(|| {
                    CodegenError::backend_contract("owning environment copy requires HeapAlloc")
                })?;
                let size = layout
                    .size_of()
                    .ok_or_else(|| CodegenError::layout("unsized Copy environment"))?;
                let environment = self
                    .builder
                    .build_call(malloc, &[size.into()], "copy.environment.allocate")
                    .map_err(|e| CodegenError::from(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .ok_or_else(|| CodegenError::backend_contract("HeapAlloc returned void"))?
                    .into_pointer_value();
                // Allocation failure cannot leave a partially initialized owner.
                let function = self
                    .builder
                    .get_insert_block()
                    .and_then(|block| block.get_parent())
                    .ok_or_else(|| {
                        CodegenError::backend_contract("environment copy outside a body")
                    })?;
                let initialized = self
                    .context
                    .append_basic_block(function, "copy.environment.initialize");
                let failed = self
                    .context
                    .append_basic_block(function, "copy.environment.failed");
                let null = self
                    .builder
                    .build_is_null(environment, "copy.environment.null")
                    .map_err(|e| CodegenError::from(e.to_string()))?;
                self.builder
                    .build_conditional_branch(null, failed, initialized)
                    .map_err(|e| CodegenError::from(e.to_string()))?;
                self.builder.position_at_end(failed);
                let trap = inkwell::intrinsics::Intrinsic::find("llvm.trap")
                    .and_then(|intrinsic| intrinsic.get_declaration(&self.module, &[]))
                    .ok_or_else(|| {
                        CodegenError::backend_contract("allocation failure trap unavailable")
                    })?;
                self.builder
                    .build_call(trap, &[], "")
                    .map_err(|e| CodegenError::from(e.to_string()))?;
                self.builder
                    .build_unreachable()
                    .map_err(|e| CodegenError::from(e.to_string()))?;
                self.builder.position_at_end(initialized);
                for (index, field) in fields.iter().enumerate() {
                    let source = self
                        .builder
                        .build_struct_gep(layout, original, index as u32, "copy.capture.source")
                        .map_err(|e| CodegenError::from(e.to_string()))?;
                    let loaded = self
                        .builder
                        .build_load(self.llvm_type(field), source, "copy.capture.value")
                        .map_err(|e| CodegenError::from(e.to_string()))?;
                    let copied = self.copy_mir_value(loaded, field)?;
                    let destination = self
                        .builder
                        .build_struct_gep(
                            layout,
                            environment,
                            index as u32,
                            "copy.capture.destination",
                        )
                        .map_err(|e| CodegenError::from(e.to_string()))?;
                    self.builder
                        .build_store(destination, copied)
                        .map_err(|e| CodegenError::from(e.to_string()))?;
                }
                self.builder
                    .build_insert_value(callable, environment, 1, "copy.callable")
                    .map(|v| v.into_struct_value().into())
                    .map_err(|e| CodegenError::from(e.to_string()))
            }
            Type::Exists(package) => {
                let package_value = value.into_struct_value();
                let payload = self
                    .builder
                    .build_extract_value(package_value, 0, "copy.package.payload")
                    .map_err(|e| CodegenError::from(e.to_string()))?;
                let payload = self.copy_mir_value(payload, &package.payload)?;
                self.builder
                    .build_insert_value(package_value, payload, 0, "copy.package")
                    .map(|v| v.into_struct_value().into())
                    .map_err(|e| CodegenError::from(e.to_string()))
            }
            _ => Ok(value),
        }
    }
}
