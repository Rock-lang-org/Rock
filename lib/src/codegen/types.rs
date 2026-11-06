//! Type mapping and coercion utilities for codegen.
//!
//! Handles conversion between Rock types and LLVM types,
//! value coercion, and default value generation.

use std::collections::HashMap;

use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType, StructType};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::AddressSpace;

use crate::type_services::layout::TypeLayout;
use crate::type_services::projection::{ProjectionNormalizer, ProjectionProvider};
use crate::types::{GenericParamId, Type};

use super::{CodeGen, CodegenError};

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn slice_layout_type(&self) -> StructType<'ctx> {
        self.context.struct_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i64_type().into(),
            ],
            false,
        )
    }

    pub(crate) fn is_fat_pointer_type(&self, ty: &Type) -> bool {
        TypeLayout::is_fat_pointer_type(self, ty)
    }

    pub(crate) fn resolve_projection_type(&self, ty: &Type) -> Type {
        let resolved = self.normalize_projection_type(ty);
        if Self::contains_resolvable_projection(&resolved) {
            panic!("missing MIR projection output for {}", resolved);
        }
        resolved
    }

    pub(crate) fn normalize_projection_type(&self, ty: &Type) -> Type {
        let projected = ProjectionNormalizer::normalize(self, ty);
        if crate::type_services::visit::type_any(&projected, |nested| {
            matches!(nested, Type::Apply { .. } | Type::Lambda { .. })
        }) {
            self.type_context()
                .normalize_type(&projected)
                .expect("accepted constructor substitutions must remain kind-correct")
        } else {
            projected
        }
    }

    fn contains_resolvable_projection(ty: &Type) -> bool {
        crate::type_services::visit::type_any(ty, |nested| {
            matches!(
                nested,
                Type::Projection {
                    trait_id,
                    assoc_type,
                    ..
                } if assoc_type.owner == *trait_id
            )
        })
    }

    pub(crate) fn struct_substitution_by_id(
        &self,
        id: crate::ids::DefId,
        type_args: &[Type],
    ) -> HashMap<GenericParamId, Type> {
        self.nominal_layouts.struct_substitution(id, type_args)
    }

    fn enum_substitution_by_id(
        &self,
        id: crate::ids::DefId,
        type_args: &[Type],
    ) -> HashMap<GenericParamId, Type> {
        self.nominal_layouts.enum_substitution(id, type_args)
    }

    pub(crate) fn enum_variant_payload_type_by_id(
        &self,
        enum_id: crate::ids::DefId,
        type_args: &[Type],
        variant_idx: usize,
    ) -> Option<Type> {
        let subst = self.enum_substitution_by_id(enum_id, type_args);

        self.nominal_layouts
            .enum_variants(&enum_id)
            .and_then(|variants| variants.get(variant_idx))
            .map(|variant| match &variant.fields {
                crate::codegen::CodegenEnumVariantFields::Unit => Type::Unit,
                crate::codegen::CodegenEnumVariantFields::Positional(types) => {
                    if types.len() == 1 {
                        self.structural_type_for(types[0])
                            .substitute_generics(&subst)
                    } else {
                        Type::Tuple(
                            types
                                .iter()
                                .map(|ty| self.structural_type_for(*ty).substitute_generics(&subst))
                                .collect(),
                        )
                    }
                }
                crate::codegen::CodegenEnumVariantFields::Named(fields) => Type::Tuple(
                    fields
                        .iter()
                        .map(|(_, ty)| self.structural_type_for(*ty).substitute_generics(&subst))
                        .collect(),
                ),
            })
            .map(|payload| {
                self.type_context()
                    .normalize_type(&payload)
                    .expect("enum layout substitution must normalize")
            })
    }

    pub(crate) fn enum_variant_payload_is_unwrapped(
        &self,
        enum_id: crate::ids::DefId,
        variant_index: usize,
    ) -> Result<bool, crate::codegen::CodegenError> {
        let variant = self
            .nominal_layouts
            .enum_variants(&enum_id)
            .and_then(|variants| variants.get(variant_index))
            .ok_or_else(|| {
                crate::codegen::CodegenError::layout("Missing enum variant payload layout")
            })?;
        Ok(
            matches!(&variant.fields, crate::codegen::CodegenEnumVariantFields::Positional(fields) if fields.len() == 1),
        )
    }

    pub(crate) fn enum_layout_types_by_id(
        &self,
        id: crate::ids::DefId,
        type_args: &[Type],
    ) -> Option<Vec<BasicTypeEnum<'ctx>>> {
        let mut field_types = vec![self.context.i32_type().into()];
        let variants = self.nominal_layouts.enum_variants(&id)?;
        for variant_idx in 0..variants.len() {
            let payload_ty = self.enum_variant_payload_type_by_id(id, type_args, variant_idx)?;
            field_types.push(self.llvm_type(&payload_ty));
        }
        Some(field_types)
    }

    pub(crate) fn callable_type(&self) -> StructType<'ctx> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        self.context
            .struct_type(&[ptr_ty.into(), ptr_ty.into()], false)
    }

    pub(crate) fn callable_code_type(
        &self,
        param_types: &[Type],
        ret_type: &Type,
    ) -> FunctionType<'ctx> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let mut params: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
        params.extend(
            param_types
                .iter()
                .map(|t| BasicMetadataTypeEnum::from(self.llvm_type(t))),
        );

        match ret_type {
            Type::Unit => self.context.void_type().fn_type(&params, false),
            _ => self.llvm_type(ret_type).fn_type(&params, false),
        }
    }

    pub(crate) fn llvm_type_id(&self, ty: crate::ids::TypeId) -> BasicTypeEnum<'ctx> {
        assert_eq!(
            self.type_view().kind(ty),
            &crate::type_services::kind::Kind::Type,
            "backend layout requested for non-runtime TypeId"
        );
        let structural = self.structural_type_for(ty);
        self.llvm_type(&structural)
    }

    pub(crate) fn build_callable_value(
        &self,
        code_ptr: PointerValue<'ctx>,
        env_ptr: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let callable_ty = self.callable_type();
        let mut callable = callable_ty.get_undef();
        callable = self
            .builder
            .build_insert_value(callable, code_ptr, 0, "callable_code")
            .map_err(|e| CodegenError::from(format!("Failed to store callable code ptr: {}", e)))?
            .into_struct_value();
        callable = self
            .builder
            .build_insert_value(callable, env_ptr, 1, "callable_env")
            .map_err(|e| CodegenError::from(format!("Failed to store callable env ptr: {}", e)))?
            .into_struct_value();

        Ok(callable.into())
    }

    /// Convert a Rock type to an LLVM type
    pub(crate) fn llvm_type(&self, ty: &Type) -> BasicTypeEnum<'ctx> {
        if matches!(ty, Type::Projection { .. }) {
            let resolved = self.resolve_projection_type(ty);
            if &resolved != ty {
                return self.llvm_type(&resolved);
            }
        }

        match ty {
            Type::I8 | Type::U8 => self.context.i8_type().into(),
            Type::I16 | Type::U16 => self.context.i16_type().into(),
            Type::I32 | Type::U32 => self.context.i32_type().into(),
            Type::I64 | Type::U64 => self.context.i64_type().into(),
            Type::F32 => self.context.f32_type().into(),
            Type::F64 => self.context.f64_type().into(),
            Type::Bool => self.context.bool_type().into(),
            Type::Char => self.context.i32_type().into(),
            Type::Unit => self.context.i64_type().into(),
            Type::Str => self.slice_layout_type().into(),
            Type::Never => self.context.i64_type().into(),
            Type::Slice(_) => self.slice_layout_type().into(),
            Type::Array(inner, len) => self.llvm_type(inner).array_type(*len as u32).into(),
            Type::Tuple(elems) => {
                let field_types: Vec<BasicTypeEnum<'ctx>> =
                    elems.iter().map(|t| self.llvm_type(t)).collect();
                self.context.struct_type(&field_types, false).into()
            }
            Type::Struct {
                id,
                args: type_args,
            } => {
                let fields = self
                    .nominal_layouts
                    .struct_fields(id)
                    .unwrap_or_else(|| panic!("unknown nominal struct DefId {:?}", id));
                let subst = self.struct_substitution_by_id(*id, type_args);
                let field_types: Vec<BasicTypeEnum<'ctx>> = fields
                    .iter()
                    .map(|(_, t)| {
                        let t = self.structural_type_for(*t);
                        let concrete = t.substitute_generics(&subst);
                        let concrete = self
                            .type_context()
                            .normalize_type(&concrete)
                            .expect("struct layout substitution must normalize");
                        self.llvm_type(&concrete)
                    })
                    .collect();
                self.context.struct_type(&field_types, false).into()
            }
            Type::Enum {
                id,
                args: type_args,
            } => {
                let layout_types = self
                    .enum_layout_types_by_id(*id, type_args)
                    .unwrap_or_else(|| panic!("unknown nominal enum DefId {:?}", id));
                self.context.struct_type(&layout_types, false).into()
            }
            Type::Function { .. } => self.callable_type().into(),
            Type::Reference { .. } | Type::Pointer(_) if self.is_fat_pointer_type(ty) => {
                self.slice_layout_type().into()
            }
            Type::Reference { .. } | Type::Pointer(_) => {
                self.context.ptr_type(AddressSpace::default()).into()
            }
            Type::TypeVar(_)
            | Type::Generic(_)
            | Type::Projection { .. }
            | Type::Constructor { .. }
            | Type::Apply { .. }
            | Type::Lambda { .. }
            | Type::BoundVar { .. }
            | Type::Error => {
                panic!("backend-forbidden type reached LLVM lowering: {ty}")
            }
        }
    }

    /// Coerce a value to a target LLVM type (int truncation/extension)
    pub(crate) fn coerce_value(
        &self,
        val: BasicValueEnum<'ctx>,
        target_ty: BasicTypeEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match (val, target_ty) {
            (BasicValueEnum::IntValue(iv), BasicTypeEnum::IntType(target_int)) => {
                let src_width = iv.get_type().get_bit_width();
                let tgt_width = target_int.get_bit_width();
                if src_width == tgt_width {
                    Ok(val)
                } else if src_width > tgt_width {
                    Ok(self
                        .builder
                        .build_int_truncate(iv, target_int, "trunc")
                        .map_err(|e| CodegenError::from(format!("Failed to truncate: {}", e)))?
                        .into())
                } else {
                    Ok(self
                        .builder
                        .build_int_s_extend(iv, target_int, "sext")
                        .map_err(|e| CodegenError::from(format!("Failed to extend: {}", e)))?
                        .into())
                }
            }
            (BasicValueEnum::FloatValue(fv), BasicTypeEnum::FloatType(target_float)) => {
                if fv.get_type() == target_float {
                    Ok(val)
                } else {
                    Ok(val)
                }
            }
            _ => Ok(val),
        }
    }

    /// Ensure two integer values have the same bit width by extending the narrower one
    pub(crate) fn coerce_int_widths(
        &self,
        l: IntValue<'ctx>,
        r: IntValue<'ctx>,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), CodegenError> {
        let l_width = l.get_type().get_bit_width();
        let r_width = r.get_type().get_bit_width();

        if l_width == r_width {
            return Ok((l, r));
        }

        if l_width > r_width {
            let r_ext = self
                .builder
                .build_int_s_extend(r, l.get_type(), "sext")
                .map_err(|e| CodegenError::from(format!("Failed to extend int: {}", e)))?;
            Ok((l, r_ext))
        } else {
            let l_ext = self
                .builder
                .build_int_s_extend(l, r.get_type(), "sext")
                .map_err(|e| CodegenError::from(format!("Failed to extend int: {}", e)))?;
            Ok((l_ext, r))
        }
    }

    /// Ensure two unsigned integer values have the same bit width by zero-extending the narrower one.
    pub(crate) fn coerce_uint_widths(
        &self,
        l: IntValue<'ctx>,
        r: IntValue<'ctx>,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), CodegenError> {
        let l_width = l.get_type().get_bit_width();
        let r_width = r.get_type().get_bit_width();

        if l_width == r_width {
            return Ok((l, r));
        }

        if l_width > r_width {
            let r_ext = self
                .builder
                .build_int_z_extend(r, l.get_type(), "zext")
                .map_err(|e| CodegenError::from(format!("Failed to extend unsigned int: {}", e)))?;
            Ok((l, r_ext))
        } else {
            let l_ext = self
                .builder
                .build_int_z_extend(l, r.get_type(), "zext")
                .map_err(|e| CodegenError::from(format!("Failed to extend unsigned int: {}", e)))?;
            Ok((l_ext, r))
        }
    }
}

impl<'ctx> ProjectionProvider for CodeGen<'ctx> {
    fn resolve_projection_output(
        &self,
        base_ty: &Type,
        trait_id: crate::ids::DefId,
        assoc_type_id: crate::ids::AssocTypeId,
        trait_args: &[Type],
    ) -> Option<Type> {
        let base_ty = self.type_context().id_for_type(base_ty)?;
        let trait_args = trait_args
            .iter()
            .map(|arg| self.type_context().id_for_type(arg))
            .collect::<Option<Vec<_>>>()?;
        self.projection_outputs
            .get(&crate::mir::MirProjectionKey {
                base: base_ty,
                trait_id,
                assoc_type_id,
                trait_args,
            })
            .copied()
            .map(|output| self.type_context().type_for(output))
    }

    fn find_projection_impl(
        &self,
        _base_ty: &Type,
        _trait_id: crate::ids::DefId,
        _trait_args: &[Type],
    ) -> Option<crate::type_services::projection::ProjectionImpl> {
        None
    }
}

#[cfg(test)]
mod tests;
