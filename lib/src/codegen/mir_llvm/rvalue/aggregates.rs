use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::BasicValueEnum;

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{AggregateKind, MirFunction, Operand};
use crate::types::Type;

use super::super::MirFunctionContext;

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_mir_aggregate(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        kind: &AggregateKind,
        operands: &[Operand],
        expected_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match kind {
            AggregateKind::Tuple => {
                let Type::Tuple(fields) = expected_ty else {
                    return Err(CodegenError::layout(format!(
                        "MIR tuple aggregate expected tuple destination, got {}",
                        self.display_type_for_diagnostic(expected_ty)
                    )));
                };
                self.validate_mir_aggregate_operand_count(
                    "tuple aggregate",
                    fields.len(),
                    operands.len(),
                )?;
                let field_types = fields
                    .iter()
                    .map(|ty| self.llvm_type(ty))
                    .collect::<Vec<_>>();
                let struct_ty = self.context.struct_type(&field_types, false);
                let mut value = struct_ty.get_undef();

                for (index, operand) in operands.iter().enumerate() {
                    let operand_value = self.compile_mir_operand(function, context, operand)?;
                    let coerced = self.coerce_value(operand_value, field_types[index])?;
                    value = self
                        .builder
                        .build_insert_value(value, coerced, index as u32, "tuple_field")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to insert MIR tuple field: {}", e))
                        })?
                        .into_struct_value();
                }

                Ok(value.into())
            }
            AggregateKind::Array => {
                let Type::Array(element_ty, len) = expected_ty else {
                    return Err(CodegenError::layout(format!(
                        "MIR array aggregate expected array destination, got {}",
                        self.display_type_for_diagnostic(expected_ty)
                    )));
                };
                self.validate_mir_aggregate_operand_count("array aggregate", len.concrete().ok_or_else(|| CodegenError::non_concrete_array_len(len))? as usize, operands.len())?;
                let element_llvm_ty = self.llvm_type(element_ty);
                let array_ty = element_llvm_ty.array_type(len.concrete().ok_or_else(|| CodegenError::non_concrete_array_len(len))? as u32);
                let mut value = array_ty.get_undef();

                for (index, operand) in operands.iter().enumerate() {
                    let operand_value = self.compile_mir_operand(function, context, operand)?;
                    let coerced = self.coerce_value(operand_value, element_llvm_ty)?;
                    value = self
                        .builder
                        .build_insert_value(value, coerced, index as u32, "array_elem")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to insert MIR array element: {}", e))
                        })?
                        .into_array_value();
                }

                Ok(value.into())
            }
            AggregateKind::Struct { id, display_name } => {
                let Type::Struct { id: ty_id, args } = expected_ty else {
                    return Err(CodegenError::layout(format!(
                        "MIR struct aggregate expected struct destination in {}, got {} for {}",
                        function.name,
                        self.display_type_for_diagnostic(expected_ty),
                        display_name
                    )));
                };
                let layout_name = display_name.as_str();
                if id != ty_id {
                    return Err(CodegenError::layout(
                        "MIR struct aggregate identity does not match its destination",
                    ));
                }
                let fields = self
                    .nominal_layouts
                    .struct_fields(id)
                    .cloned()
                    .ok_or_else(|| CodegenError::layout("Unknown MIR struct layout"))?;
                self.validate_mir_aggregate_operand_count(
                    &format!("struct aggregate {}", layout_name),
                    fields.len(),
                    operands.len(),
                )?;
                let subst = self.struct_substitution_by_id(*id, args);
                let field_types: Vec<BasicTypeEnum<'ctx>> = fields
                    .iter()
                    .map(|(_, ty)| {
                        let ty = self.structural_type_for(*ty);
                        self.llvm_type(
                            &self.normalize_projection_type(&ty.substitute_generics(&subst)),
                        )
                    })
                    .collect();
                let struct_ty = self.context.struct_type(&field_types, false);
                let mut value = struct_ty.get_undef();

                for (index, operand) in operands.iter().enumerate() {
                    let operand_value = self.compile_mir_operand(function, context, operand)?;
                    let coerced = self.coerce_value(operand_value, field_types[index])?;
                    value = self
                        .builder
                        .build_insert_value(value, coerced, index as u32, "struct_field")
                        .map_err(|e| {
                            CodegenError::from(format!("Failed to insert MIR struct field: {}", e))
                        })?
                        .into_struct_value();
                }

                Ok(value.into())
            }
            AggregateKind::EnumVariant {
                enum_id,
                variant_id,
                enum_name,
                variant_name: _,
            } => {
                let Type::Enum { id: ty_id, args } = expected_ty else {
                    return Err(CodegenError::layout(format!(
                        "MIR enum aggregate expected enum destination, got {}",
                        self.display_type_for_diagnostic(expected_ty)
                    )));
                };
                let layout_name = enum_name.as_str();
                if enum_id != ty_id {
                    return Err(CodegenError::layout(
                        "MIR enum aggregate identity does not match its destination",
                    ));
                }
                let variant_index = variant_id.0 as usize;
                let expected_payload_count =
                    self.mir_enum_payload_operand_count(*enum_id, variant_index, enum_name)?;
                self.validate_mir_aggregate_operand_count(
                    &format!("enum payload {}::{}", layout_name, variant_index),
                    expected_payload_count,
                    operands.len(),
                )?;
                let layout_types = self
                    .enum_layout_types_by_id(*enum_id, args)
                    .ok_or_else(|| CodegenError::layout("Unknown MIR enum layout"))?;
                let enum_ty = self.context.struct_type(&layout_types, false);
                let mut value = enum_ty.get_undef();
                value = self
                    .builder
                    .build_insert_value(
                        value,
                        self.context
                            .i32_type()
                            .const_int(variant_index as u64, false),
                        0,
                        "tag",
                    )
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to insert MIR enum tag: {}", e))
                    })?
                    .into_struct_value();

                let payload_ty = self
                    .enum_variant_payload_type_by_id(*enum_id, args, variant_index)
                    .ok_or_else(|| {
                        CodegenError::layout(format!(
                            "Unknown MIR enum variant index {}",
                            variant_index
                        ))
                    })?;
                let payload_llvm_ty = self.llvm_type(&payload_ty);
                // One positional field is the payload itself, even when its
                // type is a tuple; named fields retain their aggregate wrapper.
                let single_positional =
                    self.enum_variant_payload_is_unwrapped(*enum_id, variant_index)?;
                let payload = if single_positional {
                    let operand = self.compile_mir_operand(function, context, &operands[0])?;
                    self.coerce_value(operand, payload_llvm_ty)?
                } else {
                    self.compile_mir_aggregate_payload(
                        function,
                        context,
                        operands,
                        &payload_ty,
                        payload_llvm_ty,
                    )?
                };
                value = self
                    .builder
                    .build_insert_value(value, payload, variant_index as u32 + 1, "payload")
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to insert MIR enum payload: {}", e))
                    })?
                    .into_struct_value();

                Ok(value.into())
            }
        }
    }

    fn validate_mir_aggregate_operand_count(
        &self,
        context: &str,
        expected: usize,
        actual: usize,
    ) -> Result<(), CodegenError> {
        if expected == actual {
            return Ok(());
        }

        Err(CodegenError::layout(format!(
            "MIR {} operand count mismatch: expected {}, got {}",
            context, expected, actual
        )))
    }

    fn mir_enum_payload_operand_count(
        &self,
        enum_id: crate::ids::DefId,
        variant_index: usize,
        display_name: &str,
    ) -> Result<usize, CodegenError> {
        let variants = self.nominal_layouts.enum_variants(&enum_id).ok_or_else(|| {
            CodegenError::layout(format!("Unknown MIR enum layout: {}", display_name))
        })?;
        let variant = variants.get(variant_index).ok_or_else(|| {
            CodegenError::from(format!(
                "MIR enum variant index {} is out of bounds for {}",
                variant_index, display_name
            ))
        })?;

        Ok(match &variant.fields {
            crate::codegen::CodegenEnumVariantFields::Unit => 0,
            crate::codegen::CodegenEnumVariantFields::Positional(fields) => fields.len(),
            crate::codegen::CodegenEnumVariantFields::Named(fields) => fields.len(),
        })
    }

    fn compile_mir_aggregate_payload(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        operands: &[Operand],
        payload_ty: &Type,
        payload_llvm_ty: BasicTypeEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if operands.is_empty() {
            return Ok(payload_llvm_ty.const_zero());
        }

        if let Type::Tuple(fields) = payload_ty {
            let field_types = fields
                .iter()
                .map(|ty| self.llvm_type(ty))
                .collect::<Vec<_>>();
            let struct_ty = self.context.struct_type(&field_types, false);
            let mut payload = struct_ty.get_undef();

            for (index, operand) in operands.iter().enumerate() {
                let operand_value = self.compile_mir_operand(function, context, operand)?;
                let coerced = self.coerce_value(operand_value, field_types[index])?;
                payload = self
                    .builder
                    .build_insert_value(payload, coerced, index as u32, "payload_field")
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to insert MIR payload field: {}", e))
                    })?
                    .into_struct_value();
            }

            return Ok(payload.into());
        }

        if operands.len() == 1 {
            let value = self.compile_mir_operand(function, context, &operands[0])?;
            return self.coerce_value(value, payload_llvm_ty);
        }

        Err(CodegenError::layout(format!(
            "MIR enum payload got {} operands for non-tuple payload {}",
            operands.len(),
            self.display_type_for_diagnostic(payload_ty)
        )))
    }
}
