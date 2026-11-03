use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{Constant, Operand};
use crate::types::Type;

use super::super::MirFunctionContext;

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn mir_operand_ty(
        &self,
        context: &MirFunctionContext<'ctx>,
        operand: &Operand,
    ) -> Result<Type, CodegenError> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let (_, ty) = self.compile_mir_place_local(context, place)?;
                Ok(ty)
            }
            Operand::Constant(Constant::Int(_)) => Ok(Type::I64),
            Operand::Constant(Constant::Float(_)) => Ok(Type::F64),
            Operand::Constant(Constant::Bool(_)) => Ok(Type::Bool),
            Operand::Constant(Constant::Char(_)) => Ok(Type::Char),
            Operand::Constant(Constant::Unit) => Ok(Type::Unit),
            Operand::Constant(Constant::TypeId(id)) => Ok(self.type_context().type_for(*id)),
            Operand::Constant(constant) => Err(CodegenError::backend_contract(format!(
                "MIR constant type is not available for codegen: {:?}",
                constant
            ))),
        }
    }

    pub(crate) fn mir_operand_ty_id(
        &self,
        context: &MirFunctionContext<'ctx>,
        operand: &Operand,
    ) -> Result<crate::ids::TypeId, CodegenError> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let (_, ty) = self.compile_mir_place_local_id(context, place)?;
                Ok(ty)
            }
            Operand::Constant(Constant::Int(_)) => self
                .type_context()
                .id_for_type(&Type::I64)
                .ok_or_else(|| CodegenError::from("MIR TypeContext is missing I64")),
            Operand::Constant(Constant::Float(_)) => self
                .type_context()
                .id_for_type(&Type::F64)
                .ok_or_else(|| CodegenError::from("MIR TypeContext is missing F64")),
            Operand::Constant(Constant::Bool(_)) => self
                .type_context()
                .id_for_type(&Type::Bool)
                .ok_or_else(|| CodegenError::from("MIR TypeContext is missing Bool")),
            Operand::Constant(Constant::Char(_)) => self
                .type_context()
                .id_for_type(&Type::Char)
                .ok_or_else(|| CodegenError::from("MIR TypeContext is missing Char")),
            Operand::Constant(Constant::Unit) => self
                .type_context()
                .id_for_type(&Type::Unit)
                .ok_or_else(|| CodegenError::from("MIR TypeContext is missing ()")),
            Operand::Constant(Constant::TypeId(id)) => Ok(*id),
            Operand::Constant(constant) => Err(CodegenError::backend_contract(format!(
                "MIR constant type is not available for codegen: {:?}",
                constant
            ))),
        }
    }

    pub(super) fn mir_binary_op_ty(
        &self,
        context: &MirFunctionContext<'ctx>,
        lhs: &Operand,
        rhs: &Operand,
    ) -> Result<Type, CodegenError> {
        let lhs_ty = self.mir_operand_ty(context, lhs)?;
        if matches!(lhs, Operand::Constant(Constant::Int(_))) {
            let rhs_ty = self.mir_operand_ty(context, rhs)?;
            if rhs_ty.is_integer() {
                return Ok(rhs_ty);
            }
        }
        Ok(lhs_ty)
    }
}
