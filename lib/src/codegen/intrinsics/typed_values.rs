use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;
use inkwell::AtomicOrdering;

use crate::codegen::{CodeGen, CodegenError};
use crate::ids::TypeId;
use crate::mir::MirIntrinsicId;
use crate::type_context::Ty;
use crate::types::Type;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn compile_intrinsic_values_with_types(
        &mut self,
        intrinsic: MirIntrinsicId,
        compiled_args: &[BasicValueEnum<'ctx>],
        arg_types: &[Type],
        ret_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        if let MirIntrinsicId::Integer(integer) = intrinsic {
            return self.compile_integer_intrinsic(integer, compiled_args, arg_types);
        }
        if intrinsic.is_value_intrinsic() {
            return self.compile_intrinsic_values(intrinsic, compiled_args, ret_ty);
        }

        match intrinsic {
            MirIntrinsicId::AtomicU64Store => {
                if compiled_args.len() != 2 {
                    return Err(CodegenError::from(format!(
                        "Atomic U64 store expected 2 arguments, found {}",
                        compiled_args.len()
                    )));
                }
                let instruction = self
                    .builder
                    .build_store(
                        compiled_args[0].into_pointer_value(),
                        compiled_args[1].into_int_value(),
                    )
                    .map_err(|error| {
                        CodegenError::from(format!("Failed atomic U64 store: {error}"))
                    })?;
                instruction
                    .set_atomic_ordering(AtomicOrdering::SequentiallyConsistent)
                    .map_err(|error| {
                        CodegenError::from(format!("Failed atomic U64 store ordering: {error}"))
                    })?;
                instruction.set_alignment(8).map_err(|error| {
                    CodegenError::from(format!("Failed atomic U64 store alignment: {error}"))
                })?;
                Ok(None)
            }
            MirIntrinsicId::ArrayRefToSlice => {
                let Type::Reference { inner, .. } = self.resolve_projection_type(ret_ty) else {
                    return Err(CodegenError::from(
                        "ArrayRefToSlice expected slice reference result",
                    ));
                };
                let Type::Slice(_) = inner.as_ref() else {
                    return Err(CodegenError::from(
                        "ArrayRefToSlice expected slice reference result",
                    ));
                };
                let Type::Reference {
                    inner: array_inner, ..
                } = self.resolve_projection_type(&arg_types[0])
                else {
                    return Err(CodegenError::from(
                        "ArrayRefToSlice expected borrowed array input",
                    ));
                };
                let Type::Array(elem, len) = array_inner.as_ref() else {
                    return Err(CodegenError::from(
                        "ArrayRefToSlice expected borrowed array input",
                    ));
                };

                let array_ptr = compiled_args[0].into_pointer_value();
                let zero = self.context.i64_type().const_int(0, false);
                let array_ty = self.llvm_type(&Type::Array(elem.clone(), *len));
                let data_ptr = unsafe {
                    self.builder
                        .build_gep(array_ty, array_ptr, &[zero, zero], "array_ref_slice_ptr")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed ArrayRefToSlice gep: {}", e))
                        })?
                };
                self.make_fat_slice_value(
                    data_ptr,
                    self.context.i64_type().const_int(len.concrete().ok_or_else(|| CodegenError::non_concrete_array_len(len))?, false),
                )
                .map(Some)
            }
            MirIntrinsicId::ArrayLen => match self.resolve_projection_type(&arg_types[0]) {
                Type::Slice(_) => {
                    let arr = compiled_args[0].into_struct_value();
                    let len = self
                        .builder
                        .build_extract_value(arr, 1, "arr_len")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to extract array length: {}", e))
                        })?;
                    Ok(Some(len))
                }
                Type::Reference { inner, .. }
                    if matches!(inner.as_ref(), Type::Slice(_) | Type::Str) =>
                {
                    let arr = match compiled_args[0] {
                        BasicValueEnum::PointerValue(ptr) => self
                            .builder
                            .build_load(self.slice_layout_type(), ptr, "slice_ref_load")
                            .map_err(|e| {
                                CodegenError::from(format!("Failed to load slice ref: {}", e))
                            })?
                            .into_struct_value(),
                        value => value.into_struct_value(),
                    };
                    let len = self
                        .builder
                        .build_extract_value(arr, 1, "arr_len")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to extract array length: {}", e))
                        })?;
                    Ok(Some(len))
                }
                Type::Array(_, len) => Ok(Some(
                    self.context.i64_type().const_int(len.concrete().ok_or_else(|| CodegenError::non_concrete_array_len(len))?, false).into(),
                )),
                Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::Array(_, _)) => {
                    let Type::Array(_, len) = inner.as_ref() else {
                        unreachable!();
                    };
                    Ok(Some(
                        self.context.i64_type().const_int(len.concrete().ok_or_else(|| CodegenError::non_concrete_array_len(len))?, false).into(),
                    ))
                }
                other => Err(CodegenError::from(format!(
                    "ArrayLen expected slice/array, got {}",
                    other
                ))),
            },
            MirIntrinsicId::PtrOffset => {
                let ptr = compiled_args[0].into_pointer_value();
                let idx = compiled_args[1].into_int_value();
                let ptr_arg_ty = self.normalize_projection_type(&arg_types[0]);
                let elem_ty = match &ptr_arg_ty {
                    Type::Pointer(inner) => self.llvm_type(inner),
                    other => {
                        return Err(CodegenError::from(format!(
                            "PtrOffset expected pointer input, got {other}"
                        )))
                    }
                };
                let result = unsafe {
                    self.builder
                        .build_gep(elem_ty, ptr, &[idx], "ptr_offset")
                        .map_err(|e| CodegenError::from(format!("Failed to build gep: {}", e)))?
                };
                Ok(Some(result.into()))
            }
            MirIntrinsicId::ArrPtr => match self.resolve_projection_type(&arg_types[0]) {
                Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::Array(_, _)) => {
                    Ok(Some(compiled_args[0]))
                }
                Type::Slice(_) => {
                    let arr = compiled_args[0].into_struct_value();
                    let ptr = self
                        .builder
                        .build_extract_value(arr, 0, "arr_raw_ptr")
                        .map_err(|e| CodegenError::from(format!("Failed ArrPtr: {}", e)))?;
                    Ok(Some(ptr))
                }
                Type::Reference { inner, .. }
                    if matches!(inner.as_ref(), Type::Slice(_) | Type::Str) =>
                {
                    let arr = match compiled_args[0] {
                        BasicValueEnum::PointerValue(ptr) => self
                            .builder
                            .build_load(self.slice_layout_type(), ptr, "slice_ref_load")
                            .map_err(|e| {
                                CodegenError::from(format!("Failed to load slice ref: {}", e))
                            })?
                            .into_struct_value(),
                        value => value.into_struct_value(),
                    };
                    let ptr = self
                        .builder
                        .build_extract_value(arr, 0, "arr_raw_ptr")
                        .map_err(|e| CodegenError::from(format!("Failed ArrPtr: {}", e)))?;
                    Ok(Some(ptr))
                }
                Type::Array(_, _) => {
                    let temp = self
                        .builder
                        .build_alloca(self.llvm_type(&arg_types[0]), "arr_ptr_tmp")
                        .map_err(|e| CodegenError::from(format!("Failed ArrPtr alloca: {}", e)))?;
                    self.builder
                        .build_store(temp, compiled_args[0])
                        .map_err(|e| CodegenError::from(format!("Failed ArrPtr store: {}", e)))?;
                    let zero = self.context.i64_type().const_int(0, false);
                    let array_ty = self.llvm_type(&arg_types[0]);
                    let ptr = unsafe {
                        self.builder
                            .build_gep(array_ty, temp, &[zero, zero], "arr_raw_ptr")
                            .map_err(|e| CodegenError::from(format!("Failed ArrPtr gep: {}", e)))?
                    };
                    Ok(Some(ptr.into()))
                }
                other => Err(CodegenError::from(format!(
                    "ArrPtr expected slice/array, got {}",
                    other
                ))),
            },
            MirIntrinsicId::SizeOf => {
                let elem_ty = match &arg_types[0] {
                    Type::Pointer(inner) => inner.as_ref(),
                    ty => ty,
                };
                let llvm_ty = self.llvm_type(elem_ty);
                let size_val = llvm_ty
                    .size_of()
                    .unwrap_or_else(|| self.context.i64_type().const_int(8, false));
                let size_i64 = self
                    .builder
                    .build_int_cast(size_val, self.context.i64_type(), "sizeof_i64")
                    .map_err(|e| CodegenError::from(format!("SizeOf cast: {}", e)))?;
                Ok(Some(size_i64.into()))
            }
            MirIntrinsicId::Forget => Ok(None),
            other => Err(CodegenError::from(format!("Unknown intrinsic: {}", other))),
        }
    }

    pub(crate) fn compile_intrinsic_values_with_type_ids(
        &mut self,
        intrinsic: MirIntrinsicId,
        compiled_args: &[BasicValueEnum<'ctx>],
        arg_type_ids: &[TypeId],
        ret_type_id: TypeId,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        if let MirIntrinsicId::Integer(integer) = intrinsic {
            let types = arg_type_ids
                .iter()
                .map(|id| self.structural_type_for(*id))
                .collect::<Vec<_>>();
            return self.compile_integer_intrinsic(integer, compiled_args, &types);
        }
        if self.type_id_contains_projection(ret_type_id)
            || arg_type_ids
                .iter()
                .any(|id| self.type_id_contains_projection(*id))
        {
            let arg_types: Vec<Type> = arg_type_ids
                .iter()
                .map(|id| self.structural_type_for(*id))
                .collect();
            let ret_ty = self.structural_type_for(ret_type_id);
            return self.compile_intrinsic_values_with_types(
                intrinsic,
                compiled_args,
                &arg_types,
                &ret_ty,
            );
        }

        if intrinsic.is_value_intrinsic() {
            let ret_ty = self.structural_type_for(ret_type_id);
            return self.compile_intrinsic_values(intrinsic, compiled_args, &ret_ty);
        }

        match intrinsic {
            MirIntrinsicId::ArrayRefToSlice => {
                let Ty::Reference {
                    inner: ret_inner, ..
                } = self.type_view().ty(ret_type_id)
                else {
                    return Err(CodegenError::from(
                        "ArrayRefToSlice expected slice reference result",
                    ));
                };
                let Ty::Slice(_) = self.type_view().ty(*ret_inner) else {
                    return Err(CodegenError::from(
                        "ArrayRefToSlice expected slice reference result",
                    ));
                };
                let Ty::Reference {
                    inner: array_inner, ..
                } = self.type_view().ty(arg_type_ids[0])
                else {
                    return Err(CodegenError::from(
                        "ArrayRefToSlice expected borrowed array input",
                    ));
                };
                let Ty::Array { len, .. } = self.type_view().ty(*array_inner) else {
                    return Err(CodegenError::from(
                        "ArrayRefToSlice expected borrowed array input",
                    ));
                };

                let array_ptr = compiled_args[0].into_pointer_value();
                let zero = self.context.i64_type().const_int(0, false);
                let array_ty = self.llvm_type_id(*array_inner);
                let data_ptr = unsafe {
                    self.builder
                        .build_gep(array_ty, array_ptr, &[zero, zero], "array_ref_slice_ptr")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed ArrayRefToSlice gep: {}", e))
                        })?
                };
                self.make_fat_slice_value(
                    data_ptr,
                    self.context.i64_type().const_int(len.concrete().ok_or_else(|| CodegenError::non_concrete_array_len(len))?, false),
                )
                .map(Some)
            }
            MirIntrinsicId::ArrayLen => match self.type_view().ty(arg_type_ids[0]) {
                Ty::Slice(_) => {
                    let arr = compiled_args[0].into_struct_value();
                    let len = self
                        .builder
                        .build_extract_value(arr, 1, "arr_len")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to extract array length: {}", e))
                        })?;
                    Ok(Some(len))
                }
                Ty::Reference { inner, .. }
                    if matches!(self.type_view().ty(*inner), Ty::Slice(_) | Ty::Str) =>
                {
                    let arr = match compiled_args[0] {
                        BasicValueEnum::PointerValue(ptr) => self
                            .builder
                            .build_load(self.slice_layout_type(), ptr, "slice_ref_load")
                            .map_err(|e| {
                                CodegenError::from(format!("Failed to load slice ref: {}", e))
                            })?
                            .into_struct_value(),
                        value => value.into_struct_value(),
                    };
                    let len = self
                        .builder
                        .build_extract_value(arr, 1, "arr_len")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to extract array length: {}", e))
                        })?;
                    Ok(Some(len))
                }
                Ty::Reference { inner, .. } => match self.type_view().ty(*inner) {
                    Ty::Array { len, .. } => Ok(Some(
                        self.context.i64_type().const_int(len.concrete().ok_or_else(|| CodegenError::non_concrete_array_len(len))?, false).into(),
                    )),
                    other => Err(CodegenError::layout(format!(
                        "ArrayLen expected slice/array, got {:?}",
                        other
                    ))),
                },
                Ty::Array { len, .. } => Ok(Some(
                    self.context.i64_type().const_int(len.concrete().ok_or_else(|| CodegenError::non_concrete_array_len(len))?, false).into(),
                )),
                other => Err(CodegenError::layout(format!(
                    "ArrayLen expected slice/array, got {:?}",
                    other
                ))),
            },
            MirIntrinsicId::PtrOffset => {
                let ptr = compiled_args[0].into_pointer_value();
                let idx = compiled_args[1].into_int_value();
                let ptr_arg_ty =
                    self.normalize_projection_type(&self.structural_type_for(arg_type_ids[0]));
                let elem_ty = match &ptr_arg_ty {
                    Type::Pointer(inner) => self.llvm_type(inner),
                    other => {
                        return Err(CodegenError::from(format!(
                            "PtrOffset expected pointer input, got {other}"
                        )))
                    }
                };
                let result = unsafe {
                    self.builder
                        .build_gep(elem_ty, ptr, &[idx], "ptr_offset")
                        .map_err(|e| CodegenError::from(format!("Failed to build gep: {}", e)))?
                };
                Ok(Some(result.into()))
            }
            MirIntrinsicId::ArrPtr => match self.type_view().ty(arg_type_ids[0]) {
                Ty::Reference { inner, .. }
                    if matches!(self.type_view().ty(*inner), Ty::Array { .. }) =>
                {
                    Ok(Some(compiled_args[0]))
                }
                Ty::Slice(_) => {
                    let arr = compiled_args[0].into_struct_value();
                    let ptr = self
                        .builder
                        .build_extract_value(arr, 0, "arr_raw_ptr")
                        .map_err(|e| CodegenError::from(format!("Failed ArrPtr: {}", e)))?;
                    Ok(Some(ptr))
                }
                Ty::Reference { inner, .. }
                    if matches!(self.type_view().ty(*inner), Ty::Slice(_) | Ty::Str) =>
                {
                    let arr = match compiled_args[0] {
                        BasicValueEnum::PointerValue(ptr) => self
                            .builder
                            .build_load(self.slice_layout_type(), ptr, "slice_ref_load")
                            .map_err(|e| {
                                CodegenError::from(format!("Failed to load slice ref: {}", e))
                            })?
                            .into_struct_value(),
                        value => value.into_struct_value(),
                    };
                    let ptr = self
                        .builder
                        .build_extract_value(arr, 0, "arr_raw_ptr")
                        .map_err(|e| CodegenError::from(format!("Failed ArrPtr: {}", e)))?;
                    Ok(Some(ptr))
                }
                Ty::Array { .. } => {
                    let temp = self
                        .builder
                        .build_alloca(self.llvm_type_id(arg_type_ids[0]), "arr_ptr_tmp")
                        .map_err(|e| CodegenError::from(format!("Failed ArrPtr alloca: {}", e)))?;
                    self.builder
                        .build_store(temp, compiled_args[0])
                        .map_err(|e| CodegenError::from(format!("Failed ArrPtr store: {}", e)))?;
                    let zero = self.context.i64_type().const_int(0, false);
                    let array_ty = self.llvm_type_id(arg_type_ids[0]);
                    let ptr = unsafe {
                        self.builder
                            .build_gep(array_ty, temp, &[zero, zero], "arr_raw_ptr")
                            .map_err(|e| CodegenError::from(format!("Failed ArrPtr gep: {}", e)))?
                    };
                    Ok(Some(ptr.into()))
                }
                other => Err(CodegenError::layout(format!(
                    "ArrPtr expected slice/array, got {:?}",
                    other
                ))),
            },
            MirIntrinsicId::SizeOf => {
                let elem_ty = match self.type_view().ty(arg_type_ids[0]) {
                    Ty::Pointer(inner) => self.llvm_type_id(*inner),
                    _ => self.llvm_type_id(arg_type_ids[0]),
                };
                let size_val = elem_ty
                    .size_of()
                    .unwrap_or_else(|| self.context.i64_type().const_int(8, false));
                let size_i64 = self
                    .builder
                    .build_int_cast(size_val, self.context.i64_type(), "sizeof_i64")
                    .map_err(|e| CodegenError::from(format!("SizeOf cast: {}", e)))?;
                Ok(Some(size_i64.into()))
            }
            MirIntrinsicId::Forget => Ok(None),
            other => {
                let arg_types: Vec<Type> = arg_type_ids
                    .iter()
                    .map(|id| self.structural_type_for(*id))
                    .collect();
                let ret_ty = self.structural_type_for(ret_type_id);
                self.compile_intrinsic_values_with_types(other, compiled_args, &arg_types, &ret_ty)
            }
        }
    }

    fn type_id_contains_projection(&self, id: TypeId) -> bool {
        match self.type_view().ty(id) {
            Ty::Projection { .. } => true,
            Ty::Slice(inner) | Ty::Pointer(inner) | Ty::Reference { inner, .. } => {
                self.type_id_contains_projection(*inner)
            }
            Ty::Array { inner, .. } => self.type_id_contains_projection(*inner),
            Ty::Tuple(elems) => elems
                .iter()
                .any(|elem| self.type_id_contains_projection(*elem)),
            Ty::Function {
                params,
                ret,
                captures,
                ..
            } => {
                params
                    .iter()
                    .any(|param| self.type_id_contains_projection(*param))
                    || self.type_id_contains_projection(*ret)
                    || captures
                        .iter()
                        .any(|capture| self.type_id_contains_projection(capture.ty))
            }
            Ty::Struct { args, .. } | Ty::Enum { args, .. } => args
                .iter()
                .any(|arg| self.type_id_contains_projection(*arg)),
            Ty::Apply { constructor, args } => {
                self.type_id_contains_projection(*constructor)
                    || args
                        .iter()
                        .any(|arg| self.type_id_contains_projection(*arg))
            }
            Ty::Lambda { body, .. } => self.type_id_contains_projection(*body),
            Ty::I8
            | Ty::I16
            | Ty::I32
            | Ty::I64
            | Ty::U8
            | Ty::U16
            | Ty::U32
            | Ty::U64
            | Ty::F32
            | Ty::F64
            | Ty::Bool
            | Ty::Str
            | Ty::Char
            | Ty::Unit
            | Ty::Never
            | Ty::ConstValue(_)
            | Ty::TypeVar(_)
            | Ty::Generic(_)
            | Ty::Constructor { .. }
            | Ty::BoundVar { .. }
            | Ty::Error => false,
        }
    }
}
