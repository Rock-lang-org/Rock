use inkwell::values::{BasicValueEnum, PointerValue};

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{Place, Projection};
use crate::types::Type;

use super::MirFunctionContext;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn compile_mir_place_address(
        &mut self,
        ctx: &MirFunctionContext<'ctx>,
        place: &Place,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let (pointer, _) = self.compile_mir_place_local(ctx, place)?;
        Ok(pointer)
    }

    pub(crate) fn compile_mir_place_value(
        &mut self,
        ctx: &MirFunctionContext<'ctx>,
        place: &Place,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let (pointer, ty) = self.compile_mir_place_local(ctx, place)?;
        self.builder
            .build_load(self.llvm_type(&ty), pointer, "mir_place")
            .map_err(|e| CodegenError::from(format!("Failed to load MIR place: {}", e)))
    }

    pub(crate) fn compile_mir_place_local_id(
        &self,
        ctx: &MirFunctionContext<'ctx>,
        place: &Place,
    ) -> Result<(PointerValue<'ctx>, crate::ids::TypeId), CodegenError> {
        if place.projection.is_empty() {
            let Some(Some((pointer, ty))) = ctx.locals.get(place.local.0) else {
                return Err(CodegenError::backend_contract(format!(
                    "MIR local {} is not available for codegen",
                    place.local.0
                )));
            };
            return Ok((*pointer, *ty));
        }

        let (pointer, ty) = self.compile_mir_place_local(ctx, place)?;
        let id = self
            .type_context()
            .id_for_type(&ty)
            .unwrap_or_else(|| panic!("MIR projection type {} was not interned", ty));
        Ok((pointer, id))
    }

    pub(crate) fn compile_mir_place_local(
        &self,
        ctx: &MirFunctionContext<'ctx>,
        place: &Place,
    ) -> Result<(PointerValue<'ctx>, Type), CodegenError> {
        let Some(Some((pointer, ty))) = ctx.locals.get(place.local.0) else {
            return Err(CodegenError::backend_contract(format!(
                "MIR local {} is not available for codegen",
                place.local.0
            )));
        };

        let mut current_pointer = *pointer;
        let mut current_ty = self.normalize_projection_type(&self.structural_type_for(*ty));
        let mut unwrapped_downcast_payload = false;

        for projection in &place.projection {
            current_ty = self.normalize_projection_type(&current_ty);
            match projection {
                Projection::Field { index, .. } => {
                    if unwrapped_downcast_payload {
                        unwrapped_downcast_payload = false;
                        if *index == 0 {
                            continue;
                        }
                    }

                    let aggregate_ty = self.llvm_type(&current_ty);
                    let field_ty =
                        self.mir_field_projection_ty(&current_ty, *index)
                            .map_err(|_err| {
                                CodegenError::layout(format!(
                                    "invalid MIR field projection while lowering function {}",
                                    ctx.function.get_name().to_string_lossy()
                                ))
                            })?;
                    current_pointer = self
                        .builder
                        .build_struct_gep(aggregate_ty, current_pointer, *index as u32, "mir_field")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to get MIR field pointer: {}", e))
                        })?;
                    current_ty = self.normalize_projection_type(&field_ty);
                }
                Projection::Index(index_local) => {
                    unwrapped_downcast_payload = false;
                    let Some(Some((index_pointer, index_ty))) = ctx.locals.get(index_local.0)
                    else {
                        return Err(CodegenError::backend_contract(format!(
                            "MIR index local {} is not available for codegen",
                            index_local.0
                        )));
                    };
                    let index_value = self
                        .builder
                        .build_load(self.llvm_type_id(*index_ty), *index_pointer, "mir_index")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to load MIR index: {}", e))
                        })?;
                    let BasicValueEnum::IntValue(index_value) = index_value else {
                        return Err(CodegenError::backend_contract(format!(
                            "MIR index projection expected integer index, got {}",
                            self.display_type_for_diagnostic(&self.structural_type_for(*index_ty))
                        )));
                    };
                    let index_value = self.mir_index_as_i64(index_value)?;

                    match &current_ty {
                        Type::Array(element_ty, _) => {
                            let zero = self.context.i64_type().const_int(0, false);
                            let array_ty = self.llvm_type(&current_ty);
                            current_pointer = unsafe {
                                self.builder
                                    .build_gep(
                                        array_ty,
                                        current_pointer,
                                        &[zero, index_value],
                                        "mir_index_ptr",
                                    )
                                    .map_err(|e| {
                                        CodegenError::from(format!(
                                            "Failed to get MIR array index pointer: {}",
                                            e
                                        ))
                                    })?
                            };
                            current_ty = self.normalize_projection_type(element_ty);
                        }
                        Type::Slice(element_ty) => {
                            let slice_value = self
                                .builder
                                .build_load(
                                    self.llvm_type(&current_ty),
                                    current_pointer,
                                    "mir_slice",
                                )
                                .map_err(|e| {
                                    CodegenError::from(format!("Failed to load MIR slice: {}", e))
                                })?
                                .into_struct_value();
                            let data_ptr = self
                                .builder
                                .build_extract_value(slice_value, 0, "mir_slice_data")
                                .map_err(|e| {
                                    CodegenError::from(format!(
                                        "Failed to extract MIR slice data: {}",
                                        e
                                    ))
                                })?
                                .into_pointer_value();
                            let element_llvm_ty = self.llvm_type(element_ty);
                            current_pointer = unsafe {
                                self.builder
                                    .build_gep(
                                        element_llvm_ty,
                                        data_ptr,
                                        &[index_value],
                                        "mir_slice_index_ptr",
                                    )
                                    .map_err(|e| {
                                        CodegenError::from(format!(
                                            "Failed to get MIR slice index pointer: {}",
                                            e
                                        ))
                                    })?
                            };
                            current_ty = self.normalize_projection_type(element_ty);
                        }
                        Type::Pointer(pointee_ty)
                            if matches!(pointee_ty.as_ref(), Type::Slice(_) | Type::Str) =>
                        {
                            let pointer_value = self
                                .builder
                                .build_load(
                                    self.llvm_type(&current_ty),
                                    current_pointer,
                                    "mir_raw_slice_ptr",
                                )
                                .map_err(|e| {
                                    CodegenError::from(format!(
                                        "Failed to load MIR raw slice pointer: {}",
                                        e
                                    ))
                                })?
                                .into_struct_value();
                            let data_ptr = self
                                .builder
                                .build_extract_value(pointer_value, 0, "mir_raw_slice_data")
                                .map_err(|e| {
                                    CodegenError::from(format!(
                                        "Failed to extract MIR raw slice data: {}",
                                        e
                                    ))
                                })?
                                .into_pointer_value();
                            let element_ty = match pointee_ty.as_ref() {
                                Type::Slice(element_ty) => element_ty.as_ref().clone(),
                                Type::Str => Type::U8,
                                _ => unreachable!(),
                            };
                            let element_llvm_ty = self.llvm_type(&element_ty);
                            current_pointer = unsafe {
                                self.builder
                                    .build_gep(
                                        element_llvm_ty,
                                        data_ptr,
                                        &[index_value],
                                        "mir_raw_slice_index_ptr",
                                    )
                                    .map_err(|e| {
                                        CodegenError::from(format!(
                                            "Failed to get MIR raw slice index pointer: {}",
                                            e
                                        ))
                                    })?
                            };
                            current_ty = self.normalize_projection_type(&element_ty);
                        }
                        Type::Pointer(element_ty) => {
                            let pointer_value = self
                                .builder
                                .build_load(
                                    self.llvm_type(&current_ty),
                                    current_pointer,
                                    "mir_ptr_index_base",
                                )
                                .map_err(|e| {
                                    CodegenError::from(format!(
                                        "Failed to load MIR pointer index base: {}",
                                        e
                                    ))
                                })?;
                            let BasicValueEnum::PointerValue(data_ptr) = pointer_value else {
                                return Err(CodegenError::layout(format!(
                                    "MIR pointer index expected thin pointer, got {}",
                                    self.display_type_for_diagnostic(&current_ty)
                                )));
                            };
                            let element_llvm_ty = self.llvm_type(element_ty);
                            current_pointer = unsafe {
                                self.builder
                                    .build_gep(
                                        element_llvm_ty,
                                        data_ptr,
                                        &[index_value],
                                        "mir_ptr_index_ptr",
                                    )
                                    .map_err(|e| {
                                        CodegenError::from(format!(
                                            "Failed to get MIR pointer index pointer: {}",
                                            e
                                        ))
                                    })?
                            };
                            current_ty = self.normalize_projection_type(element_ty);
                        }
                        _ => {
                            return Err(CodegenError::layout(format!(
                                "MIR index projection expected array, slice, or pointer, got {}",
                                self.display_type_for_diagnostic(&current_ty)
                            )));
                        }
                    }
                }
                Projection::Downcast(variant_id) => {
                    let Type::Enum { id, args } = &current_ty else {
                        return Err(CodegenError::layout(format!(
                            "MIR downcast projection expected enum, got {}",
                            self.display_type_for_diagnostic(&current_ty)
                        )));
                    };
                    let variant_index = variant_id.0 as usize;
                    let payload_ty = self
                        .enum_variant_payload_type_by_id(*id, args, variant_index)
                        .ok_or_else(|| {
                            CodegenError::layout(format!(
                                "Unknown MIR enum variant {}",
                                variant_index
                            ))
                        })?;
                    let enum_llvm_ty = self.llvm_type(&current_ty);
                    current_pointer = self
                        .builder
                        .build_struct_gep(
                            enum_llvm_ty,
                            current_pointer,
                            variant_index as u32 + 1,
                            "mir_variant_payload",
                        )
                        .map_err(|e| {
                            CodegenError::from(format!(
                                "Failed to get MIR variant payload pointer: {}",
                                e
                            ))
                        })?;
                    unwrapped_downcast_payload =
                        self.enum_variant_payload_is_unwrapped(*id, variant_index)?;
                    current_ty = self.normalize_projection_type(&payload_ty);
                }
                Projection::Deref => {
                    let pointee_ty = match &current_ty {
                        Type::Pointer(inner) | Type::Reference { inner, .. } => {
                            inner.as_ref().clone()
                        }
                        _ => {
                            return Err(CodegenError::layout(format!(
                                "MIR deref projection expected pointer or reference, got {}",
                                self.display_type_for_diagnostic(&current_ty)
                            )))
                        }
                    };
                    if matches!(pointee_ty, Type::Slice(_) | Type::Str) {
                        current_ty = self.normalize_projection_type(&pointee_ty);
                        continue;
                    }
                    let pointer_value = self
                        .builder
                        .build_load(
                            self.llvm_type(&current_ty),
                            current_pointer,
                            "mir_deref_ptr",
                        )
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to load MIR deref pointer: {}", e))
                        })?;
                    let BasicValueEnum::PointerValue(pointer_value) = pointer_value else {
                        return Err(CodegenError::layout(format!(
                            "MIR deref projection expected thin pointer value, got {}",
                            self.display_type_for_diagnostic(&current_ty)
                        )));
                    };
                    current_pointer = pointer_value;
                    current_ty = self.normalize_projection_type(&pointee_ty);
                }
            }
        }

        Ok((current_pointer, self.normalize_projection_type(&current_ty)))
    }

    fn mir_field_projection_ty(&self, ty: &Type, index: usize) -> Result<Type, CodegenError> {
        match ty {
            Type::Tuple(fields) => fields.get(index).cloned().ok_or_else(|| {
                CodegenError::from(format!("MIR tuple field index {} is out of bounds", index))
            }),
            Type::Struct { id, args } => {
                let fields = self
                    .struct_layouts_by_id
                    .get(id)
                    .ok_or_else(|| CodegenError::layout("Unknown MIR struct layout"))?;
                let (_, field_ty) = fields.get(index).ok_or_else(|| {
                    CodegenError::layout(format!(
                        "MIR struct field index {} is out of bounds",
                        index
                    ))
                })?;
                let subst = self.struct_substitution_by_id(*id, args);
                let field_ty = self.structural_type_for(*field_ty);
                Ok(field_ty.substitute_generics(&subst))
            }
            other => Err(CodegenError::layout(format!(
                "MIR field projection expected aggregate, got {} in {}",
                self.display_type_for_diagnostic(other),
                self.current_function
                    .map(|function| function.get_name().to_string_lossy().into_owned())
                    .unwrap_or_else(|| "<unknown>".to_string())
            ))),
        }
    }

    fn mir_index_as_i64(
        &self,
        index_value: inkwell::values::IntValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let i64_ty = self.context.i64_type();
        let width = index_value.get_type().get_bit_width();
        if width == 64 {
            return Ok(index_value);
        }
        if width < 64 {
            return self
                .builder
                .build_int_z_extend(index_value, i64_ty, "mir_index_ext")
                .map_err(|e| CodegenError::from(format!("Failed to extend MIR index: {}", e)));
        }
        self.builder
            .build_int_truncate(index_value, i64_ty, "mir_index_trunc")
            .map_err(|e| CodegenError::from(format!("Failed to truncate MIR index: {}", e)))
    }
}
