mod aggregates;
mod casts;
mod closures;
mod operand_types;

use inkwell::values::BasicValueEnum;

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{MirFunction, Rvalue};
use crate::types::Type;

use super::MirFunctionContext;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn compile_mir_rvalue_with_expected(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        rvalue: &Rvalue,
        expected_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        self.compile_mir_rvalue_inner(function, context, rvalue, Some(expected_ty))
    }

    fn compile_mir_rvalue_inner(
        &mut self,
        function: &MirFunction,
        context: &MirFunctionContext<'ctx>,
        rvalue: &Rvalue,
        expected_ty: Option<&Type>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match rvalue {
            Rvalue::Use(operand) => self.compile_mir_operand(function, context, operand),
            Rvalue::BinaryOp(op, lhs, rhs) => {
                let lhs_value = self.compile_mir_operand(function, context, lhs)?;
                let rhs_value = self.compile_mir_operand(function, context, rhs)?;
                let op_ty = self.mir_binary_op_ty(context, lhs, rhs)?;
                self.compile_binop(*op, lhs_value, rhs_value, &op_ty)?
                    .ok_or_else(|| CodegenError::from("MIR binary op produced no value"))
            }
            Rvalue::UnaryOp(op, operand) => {
                let value = self.compile_mir_operand(function, context, operand)?;
                let ty = self.mir_operand_ty(context, operand)?;
                self.compile_unaryop(*op, value, &ty)?
                    .ok_or_else(|| CodegenError::from("MIR unary op produced no value"))
            }
            Rvalue::Cast(operand, target_ty) => {
                let value = self.compile_mir_operand(function, context, operand)?;
                let from_ty = self.mir_operand_ty(context, operand)?;
                let target_ty = self.structural_type_for(*target_ty);
                self.compile_mir_cast_value(value, &from_ty, &target_ty)
            }
            Rvalue::Ref(_, place) => {
                let expected_ty = expected_ty.ok_or_else(|| {
                    CodegenError::from("MIR reference codegen requires destination type")
                })?;
                self.compile_mir_ref_rvalue(context, place, expected_ty)
            }
            Rvalue::Discriminant(place) => {
                let (_, ty) = self.compile_mir_place_local(context, place)?;
                if !matches!(ty, Type::Enum { .. }) {
                    return Err(CodegenError::layout(format!(
                        "MIR discriminant codegen expected enum place, got {}",
                        self.display_type_for_diagnostic(&ty)
                    )));
                }

                let value = self
                    .compile_mir_place_value(context, place)?
                    .into_struct_value();
                self.builder
                    .build_extract_value(value, 0, "discriminant")
                    .map_err(|e| {
                        CodegenError::from(format!("Failed to extract MIR discriminant: {}", e))
                    })
            }
            Rvalue::Aggregate(kind, operands) => {
                let expected_ty = expected_ty.ok_or_else(|| {
                    CodegenError::from("MIR aggregate codegen requires destination type")
                })?;
                self.compile_mir_aggregate(function, context, kind, operands, expected_ty)
            }
            Rvalue::Closure(closure) => {
                let expected_ty = expected_ty.ok_or_else(|| {
                    CodegenError::from("MIR closure codegen requires destination type")
                })?;
                self.compile_mir_closure_value(context, closure, expected_ty)
            }
        }
    }

    fn compile_mir_ref_rvalue(
        &mut self,
        context: &MirFunctionContext<'ctx>,
        place: &crate::mir::Place,
        expected_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let (address, source_ty) = self.compile_mir_place_local(context, place)?;

        if self.is_fat_pointer_type(expected_ty) {
            return match &source_ty {
                Type::Array(element_ty, len) => {
                    let zero = self.context.i64_type().const_int(0, false);
                    let array_ty = self.llvm_type(&Type::Array(element_ty.clone(), *len))?;
                    let data_ptr = unsafe {
                        self.builder
                            .build_gep(array_ty, address, &[zero, zero], "array_ref_slice_ptr")
                            .map_err(|e| {
                                CodegenError::from(format!(
                                    "Failed to build MIR array ref slice ptr: {}",
                                    e
                                ))
                            })?
                    };
                    self.make_fat_slice_value(
                        data_ptr,
                        self.context.i64_type().const_int(len.concrete().ok_or_else(|| CodegenError::non_concrete_array_len(len))?, false),
                    )
                }
                Type::Slice(_) | Type::Str | Type::Reference { .. } | Type::Pointer(_) => {
                    let value = self
                        .builder
                        .build_load(self.llvm_type(&source_ty)?, address, "mir_ref_slice_load")
                        .map_err(|e| {
                            CodegenError::from(format!(
                                "Failed to load MIR slice-compatible ref source: {}",
                                e
                            ))
                        })?;
                    let (data_ptr, len) = self.slice_parts_from_value(value, &source_ty)?;
                    self.make_fat_slice_value(data_ptr, len)
                }
                _ => Err(CodegenError::layout(format!(
                    "MIR fat reference expected slice-compatible source, got {}",
                    self.display_type_for_diagnostic(&source_ty)
                ))),
            };
        }

        Ok(address.into())
    }
}
