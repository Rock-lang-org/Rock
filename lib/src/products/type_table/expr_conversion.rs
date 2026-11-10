use super::collection_codec::{decode_exprs, encode_exprs};
use super::hir_schema::{
    SerializedHirBlock, SerializedHirCallTarget, SerializedHirClosureCapture, SerializedHirExpr,
    SerializedHirExprKind, SerializedHirMatchArm, SerializedHirMethodCallTarget,
    SerializedHirParam, SerializedHirStructLiteralField, SerializedHirVarRef,
};
use super::type_codec::{ProductTypeDecoder, ProductTypeEncoder};

impl SerializedHirExprKind {
    pub(super) fn encode(
        value: &crate::hir::HirExprKindFor<crate::hir::AcceptedHir>,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(match value {
            crate::hir::HirExprKindFor::IntLiteral(value) => Self::IntLiteral(*value),
            crate::hir::HirExprKindFor::ConstParam(param) => Self::ConstParam(*param),
            crate::hir::HirExprKindFor::FloatLiteral(value) => Self::FloatLiteral(*value),
            crate::hir::HirExprKindFor::BoolLiteral(value) => Self::BoolLiteral(*value),
            crate::hir::HirExprKindFor::StringLiteral(value) => Self::StringLiteral(value.clone()),
            crate::hir::HirExprKindFor::CharLiteral(value) => Self::CharLiteral(*value),
            crate::hir::HirExprKindFor::ArrayLiteral(elems) => {
                Self::ArrayLiteral(encode_exprs(elems, encoder)?)
            }
            crate::hir::HirExprKindFor::ArrayRepeat(value, len) => {
                Self::ArrayRepeat(Box::new(SerializedHirExpr::encode(value, encoder)?), *len)
            }
            crate::hir::HirExprKindFor::TupleLiteral(elems) => {
                Self::TupleLiteral(encode_exprs(elems, encoder)?)
            }
            crate::hir::HirExprKindFor::Unit => Self::Unit,
            crate::hir::HirExprKindFor::Var(name) => Self::Var(name.clone()),
            crate::hir::HirExprKindFor::ResolvedVar(reference) => {
                Self::ResolvedVar(SerializedHirVarRef::from_hir(reference))
            }
            crate::hir::HirExprKindFor::FieldAccess(inner, name, location) => Self::FieldAccess(
                Box::new(SerializedHirExpr::encode(inner, encoder)?),
                name.clone(),
                location.clone(),
            ),
            crate::hir::HirExprKindFor::TupleIndex(inner, index) => {
                Self::TupleIndex(Box::new(SerializedHirExpr::encode(inner, encoder)?), *index)
            }
            crate::hir::HirExprKindFor::BinOp(op, lhs, rhs) => Self::BinOp(
                *op,
                Box::new(SerializedHirExpr::encode(lhs, encoder)?),
                Box::new(SerializedHirExpr::encode(rhs, encoder)?),
            ),
            crate::hir::HirExprKindFor::UnaryOp(op, inner) => {
                Self::UnaryOp(*op, Box::new(SerializedHirExpr::encode(inner, encoder)?))
            }
            crate::hir::HirExprKindFor::Call(func, args, target) => Self::Call(
                Box::new(SerializedHirExpr::encode(func, encoder)?),
                encode_exprs(args, encoder)?,
                target
                    .as_ref()
                    .map(|target| SerializedHirCallTarget::encode(target, encoder))
                    .transpose()?,
            ),
            crate::hir::HirExprKindFor::MethodCall(func, name, args, receiver, target) => {
                Self::MethodCall(
                    Box::new(SerializedHirExpr::encode(func, encoder)?),
                    name.clone(),
                    encode_exprs(args, encoder)?,
                    *receiver,
                    Some(SerializedHirMethodCallTarget::encode(target, encoder)?),
                )
            }
            crate::hir::HirExprKindFor::Try {
                expr,
                branch_method,
                branch_target: _,
                branch_self_receiver,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                control_flow_enum,
                break_variant,
                continue_variant,
            } => Self::Try {
                expr: Box::new(SerializedHirExpr::encode(expr, encoder)?),
                branch_method: Some(SerializedHirMethodCallTarget::encode(
                    branch_method,
                    encoder,
                )?),
                branch_self_receiver: *branch_self_receiver,
                from_residual_target: Some(SerializedHirCallTarget::encode(
                    from_residual_target,
                    encoder,
                )?),
                output_ty: encoder.encode_type(output_ty)?,
                residual_ty: encoder.encode_type(residual_ty)?,
                return_ty: encoder.encode_type(return_ty)?,
                control_flow_enum: *control_flow_enum,
                break_variant: break_variant.clone(),
                continue_variant: continue_variant.clone(),
            },
            crate::hir::HirExprKindFor::StructLiteral(name, struct_id, fields) => {
                Self::StructLiteral(
                    name.clone(),
                    *struct_id,
                    fields
                        .iter()
                        .map(|field| SerializedHirStructLiteralField::encode(field, encoder))
                        .collect::<Result<_, _>>()?,
                )
            }
            crate::hir::HirExprKindFor::EnumVariant(enum_name, variant_name, args, location) => {
                Self::EnumVariant(
                    enum_name.clone(),
                    variant_name.clone(),
                    encode_exprs(args, encoder)?,
                    location.clone(),
                )
            }
            crate::hir::HirExprKindFor::If {
                condition,
                then_branch,
                else_branch,
            } => Self::If {
                condition: Box::new(SerializedHirExpr::encode(condition, encoder)?),
                then_branch: SerializedHirBlock::encode(then_branch, encoder)?,
                else_branch: else_branch
                    .as_ref()
                    .map(|block| SerializedHirBlock::encode(block, encoder))
                    .transpose()?,
            },
            crate::hir::HirExprKindFor::Match { scrutinee, arms } => Self::Match {
                scrutinee: Box::new(SerializedHirExpr::encode(scrutinee, encoder)?),
                arms: arms
                    .iter()
                    .map(|arm| SerializedHirMatchArm::encode(arm, encoder))
                    .collect::<Result<_, _>>()?,
            },
            crate::hir::HirExprKindFor::While { condition, body } => Self::While {
                condition: Box::new(SerializedHirExpr::encode(condition, encoder)?),
                body: SerializedHirBlock::encode(body, encoder)?,
            },
            crate::hir::HirExprKindFor::For {
                var,
                local_id,
                iter,
                body,
            } => Self::For {
                var: var.clone(),
                local_id: *local_id,
                iter: Box::new(SerializedHirExpr::encode(iter, encoder)?),
                body: SerializedHirBlock::encode(body, encoder)?,
            },
            crate::hir::HirExprKindFor::Loop(body) => {
                Self::Loop(SerializedHirBlock::encode(body, encoder)?)
            }
            crate::hir::HirExprKindFor::Block(body) => {
                Self::Block(SerializedHirBlock::encode(body, encoder)?)
            }
            crate::hir::HirExprKindFor::UnsafeBlock(body) => {
                Self::Block(SerializedHirBlock::encode(body, encoder)?)
            }
            crate::hir::HirExprKindFor::Lambda {
                params,
                body,
                captures,
            } => Self::Lambda {
                params: params
                    .iter()
                    .map(|param| SerializedHirParam::encode(param, encoder))
                    .collect::<Result<_, _>>()?,
                body: SerializedHirBlock::encode(body, encoder)?,
                captures: captures
                    .iter()
                    .map(|capture| SerializedHirClosureCapture::encode(capture, encoder))
                    .collect::<Result<_, _>>()?,
            },
            crate::hir::HirExprKindFor::Ref(mutable, inner) => Self::Ref(
                *mutable,
                Box::new(SerializedHirExpr::encode(inner, encoder)?),
            ),
            crate::hir::HirExprKindFor::Deref(inner) => {
                Self::Deref(Box::new(SerializedHirExpr::encode(inner, encoder)?))
            }
            crate::hir::HirExprKindFor::Cast(inner, ty) => Self::Cast(
                Box::new(SerializedHirExpr::encode(inner, encoder)?),
                encoder.encode_type(ty)?,
            ),
            crate::hir::HirExprKindFor::Assign(lhs, rhs) => Self::Assign(
                Box::new(SerializedHirExpr::encode(lhs, encoder)?),
                Box::new(SerializedHirExpr::encode(rhs, encoder)?),
            ),
            crate::hir::HirExprKindFor::Intrinsic { name, args } => Self::Intrinsic {
                name: name.clone(),
                args: encode_exprs(args, encoder)?,
            },
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirExprKindFor<crate::hir::AcceptedHir>, String> {
        Ok(match self {
            Self::IntLiteral(value) => crate::hir::HirExprKindFor::IntLiteral(value),
            Self::ConstParam(param) => crate::hir::HirExprKindFor::ConstParam(param),
            Self::FloatLiteral(value) => crate::hir::HirExprKindFor::FloatLiteral(value),
            Self::BoolLiteral(value) => crate::hir::HirExprKindFor::BoolLiteral(value),
            Self::StringLiteral(value) => crate::hir::HirExprKindFor::StringLiteral(value),
            Self::CharLiteral(value) => crate::hir::HirExprKindFor::CharLiteral(value),
            Self::ArrayLiteral(elems) => {
                crate::hir::HirExprKindFor::ArrayLiteral(decode_exprs(elems, decoder)?)
            }
            Self::ArrayRepeat(value, len) => {
                crate::hir::HirExprKindFor::ArrayRepeat(Box::new(value.decode(decoder)?), len)
            }
            Self::TupleLiteral(elems) => {
                crate::hir::HirExprKindFor::TupleLiteral(decode_exprs(elems, decoder)?)
            }
            Self::Unit => crate::hir::HirExprKindFor::Unit,
            Self::Var(name) => crate::hir::HirExprKindFor::Var(name),
            Self::ResolvedVar(reference) => {
                crate::hir::HirExprKindFor::ResolvedVar(reference.into_hir())
            }
            Self::FieldAccess(inner, name, location) => crate::hir::HirExprKindFor::FieldAccess(
                Box::new(inner.decode(decoder)?),
                name,
                location,
            ),
            Self::TupleIndex(inner, index) => {
                crate::hir::HirExprKindFor::TupleIndex(Box::new(inner.decode(decoder)?), index)
            }
            Self::BinOp(op, lhs, rhs) => crate::hir::HirExprKindFor::BinOp(
                op,
                Box::new(lhs.decode(decoder)?),
                Box::new(rhs.decode(decoder)?),
            ),
            Self::UnaryOp(op, inner) => {
                crate::hir::HirExprKindFor::UnaryOp(op, Box::new(inner.decode(decoder)?))
            }
            Self::Call(func, args, target) => crate::hir::HirExprKindFor::Call(
                Box::new(func.decode(decoder)?),
                decode_exprs(args, decoder)?,
                target.map(|target| target.decode(decoder)).transpose()?,
            ),
            Self::MethodCall(func, name, args, receiver, target) => {
                crate::hir::HirExprKindFor::MethodCall(
                    Box::new(func.decode(decoder)?),
                    name,
                    decode_exprs(args, decoder)?,
                    receiver,
                    target
                        .map(|target| target.decode(decoder))
                        .transpose()?
                        .ok_or_else(|| {
                            "serialized accepted method call lacks authority".to_string()
                        })?,
                )
            }
            Self::Try {
                expr,
                branch_method,
                branch_self_receiver,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                control_flow_enum,
                break_variant,
                continue_variant,
            } => crate::hir::HirExprKindFor::Try {
                expr: Box::new(expr.decode(decoder)?),
                branch_method: branch_method
                    .map(|target| target.decode(decoder))
                    .transpose()?
                    .ok_or_else(|| "serialized accepted Try lacks branch authority".to_string())?,
                branch_target: None,
                branch_self_receiver,
                from_residual_target: from_residual_target
                    .map(|target| target.decode(decoder))
                    .transpose()?
                    .ok_or_else(|| {
                        "serialized accepted Try lacks residual authority".to_string()
                    })?,
                output_ty: decoder.decode_type(output_ty)?,
                residual_ty: decoder.decode_type(residual_ty)?,
                return_ty: decoder.decode_type(return_ty)?,
                control_flow_enum,
                break_variant,
                continue_variant,
            },
            Self::StructLiteral(name, struct_id, fields) => {
                crate::hir::HirExprKindFor::StructLiteral(
                    name,
                    struct_id,
                    fields
                        .into_iter()
                        .map(|field| field.decode(decoder))
                        .collect::<Result<_, _>>()?,
                )
            }
            Self::EnumVariant(enum_name, variant_name, args, location) => {
                crate::hir::HirExprKindFor::EnumVariant(
                    enum_name,
                    variant_name,
                    decode_exprs(args, decoder)?,
                    location,
                )
            }
            Self::If {
                condition,
                then_branch,
                else_branch,
            } => crate::hir::HirExprKindFor::If {
                condition: Box::new(condition.decode(decoder)?),
                then_branch: then_branch.decode(decoder)?,
                else_branch: else_branch.map(|block| block.decode(decoder)).transpose()?,
            },
            Self::Match { scrutinee, arms } => crate::hir::HirExprKindFor::Match {
                scrutinee: Box::new(scrutinee.decode(decoder)?),
                arms: arms
                    .into_iter()
                    .map(|arm| arm.decode(decoder))
                    .collect::<Result<_, _>>()?,
            },
            Self::While { condition, body } => crate::hir::HirExprKindFor::While {
                condition: Box::new(condition.decode(decoder)?),
                body: body.decode(decoder)?,
            },
            Self::For {
                var,
                local_id,
                iter,
                body,
            } => crate::hir::HirExprKindFor::For {
                var,
                local_id,
                iter: Box::new(iter.decode(decoder)?),
                body: body.decode(decoder)?,
            },
            Self::Loop(body) => crate::hir::HirExprKindFor::Loop(body.decode(decoder)?),
            Self::Block(body) => crate::hir::HirExprKindFor::Block(body.decode(decoder)?),
            Self::Lambda {
                params,
                body,
                captures,
            } => crate::hir::HirExprKindFor::Lambda {
                params: params
                    .into_iter()
                    .map(|param| param.decode(decoder))
                    .collect::<Result<_, _>>()?,
                body: body.decode(decoder)?,
                captures: captures
                    .into_iter()
                    .map(|capture| capture.decode(decoder))
                    .collect::<Result<_, _>>()?,
            },
            Self::Ref(mutable, inner) => {
                crate::hir::HirExprKindFor::Ref(mutable, Box::new(inner.decode(decoder)?))
            }
            Self::Deref(inner) => {
                crate::hir::HirExprKindFor::Deref(Box::new(inner.decode(decoder)?))
            }
            Self::Cast(inner, ty) => crate::hir::HirExprKindFor::Cast(
                Box::new(inner.decode(decoder)?),
                decoder.decode_type(ty)?,
            ),
            Self::Assign(lhs, rhs) => crate::hir::HirExprKindFor::Assign(
                Box::new(lhs.decode(decoder)?),
                Box::new(rhs.decode(decoder)?),
            ),
            Self::Intrinsic { name, args } => crate::hir::HirExprKindFor::Intrinsic {
                name,
                args: decode_exprs(args, decoder)?,
            },
        })
    }
}
