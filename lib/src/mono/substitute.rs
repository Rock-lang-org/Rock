use std::collections::HashMap;

use super::hir_types::{
    HirBlock, HirCallTarget, HirExpr, HirExprKind, HirMatchArm, HirParam, HirPattern, HirStmt,
    HirStructLiteralField, HirStructPatternField,
};
use crate::ids::TypeId;
use crate::types::{GenericParamId, Type};

use super::Monomorphizer;

impl Monomorphizer {
    /// Substitute generic types in a Type using a substitution map
    pub(super) fn substitute_type_with_map(
        &mut self,
        ty: &Type,
        substitution: &HashMap<GenericParamId, TypeId>,
    ) -> Type {
        let substitution = substitution
            .iter()
            .map(|(param, ty)| (*param, self.type_for(*ty)))
            .collect();
        self.normalize_type(&ty.substitute_generics(&substitution))
    }

    fn substitute_call_target(
        &mut self,
        target: &Option<HirCallTarget>,
        substitution: &HashMap<GenericParamId, TypeId>,
    ) -> Option<HirCallTarget> {
        match target {
            Some(HirCallTarget::StaticMethod(target)) => {
                let mut target = target.clone();
                target.owner_ty = self.substitute_type_with_map(&target.owner_ty, substitution);
                target
                    .method
                    .for_each_type_mut(|ty| *ty = self.substitute_type_with_map(ty, substitution));
                Some(HirCallTarget::StaticMethod(target))
            }
            other => other.clone(),
        }
    }

    fn substitute_required_call_target(
        &mut self,
        target: &HirCallTarget,
        substitution: &HashMap<GenericParamId, TypeId>,
    ) -> HirCallTarget {
        self.substitute_call_target(&Some(target.clone()), substitution)
            .expect("required accepted call authority must remain present")
    }

    pub(super) fn substitute_block(
        &mut self,
        block: &HirBlock,
        substitution: &HashMap<GenericParamId, TypeId>,
    ) -> HirBlock {
        let stmts = block
            .stmts
            .iter()
            .map(|s| self.substitute_stmt(s, substitution))
            .collect();
        let ty = self.substitute_type_with_map(&block.ty, substitution);
        HirBlock { stmts, ty }
    }

    fn substitute_pattern(
        &mut self,
        pattern: &HirPattern,
        substitution: &HashMap<GenericParamId, TypeId>,
    ) -> HirPattern {
        match pattern {
            HirPattern::Tuple(patterns) => HirPattern::Tuple(
                patterns
                    .iter()
                    .map(|pattern| self.substitute_pattern(pattern, substitution))
                    .collect(),
            ),
            HirPattern::Struct(name, struct_id, type_args, field_patterns) => HirPattern::Struct(
                name.clone(),
                *struct_id,
                type_args
                    .iter()
                    .map(|ty| self.substitute_type_with_map(ty, substitution))
                    .collect(),
                field_patterns
                    .iter()
                    .map(|field| HirStructPatternField {
                        name: field.name.clone(),
                        field: field.field.clone(),
                        pattern: self.substitute_pattern(&field.pattern, substitution),
                    })
                    .collect(),
            ),
            HirPattern::Enum(enum_name, variant_name, location, patterns) => HirPattern::Enum(
                enum_name.clone(),
                variant_name.clone(),
                location.clone(),
                patterns
                    .iter()
                    .map(|pattern| self.substitute_pattern(pattern, substitution))
                    .collect(),
            ),
            HirPattern::Or(patterns) => HirPattern::Or(
                patterns
                    .iter()
                    .map(|pattern| self.substitute_pattern(pattern, substitution))
                    .collect(),
            ),
            HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {
                pattern.clone()
            }
        }
    }

    fn substitute_stmt(
        &mut self,
        stmt: &HirStmt,
        substitution: &HashMap<GenericParamId, TypeId>,
    ) -> HirStmt {
        match stmt {
            HirStmt::Let {
                name,
                local_id,
                ty,
                value,
                mutable,
            } => HirStmt::Let {
                name: name.clone(),
                local_id: *local_id,
                ty: self.substitute_type_with_map(ty, substitution),
                value: self.substitute_expr(value, substitution),
                mutable: *mutable,
            },
            HirStmt::Expr(expr) => HirStmt::Expr(self.substitute_expr(expr, substitution)),
            HirStmt::Return(Some(expr)) => {
                HirStmt::Return(Some(self.substitute_expr(expr, substitution)))
            }
            HirStmt::Break(Some(expr)) => {
                HirStmt::Break(Some(self.substitute_expr(expr, substitution)))
            }
            _ => stmt.clone(),
        }
    }

    pub(super) fn substitute_expr(
        &mut self,
        expr: &HirExpr,
        substitution: &HashMap<GenericParamId, TypeId>,
    ) -> HirExpr {
        let ty = self.substitute_type_with_map(&expr.ty, substitution);

        let kind = match &expr.kind {
            HirExprKind::Var(name) => HirExprKind::Var(name.clone()),
            HirExprKind::ResolvedVar(reference) => HirExprKind::ResolvedVar(reference.clone()),
            HirExprKind::BinOp(op, lhs, rhs) => HirExprKind::BinOp(
                op.clone(),
                Box::new(self.substitute_expr(lhs, substitution)),
                Box::new(self.substitute_expr(rhs, substitution)),
            ),
            HirExprKind::UnaryOp(op, inner) => HirExprKind::UnaryOp(
                op.clone(),
                Box::new(self.substitute_expr(inner, substitution)),
            ),
            HirExprKind::Call(func, args, target) => HirExprKind::Call(
                Box::new(self.substitute_expr(func, substitution)),
                args.iter()
                    .map(|a| self.substitute_expr(a, substitution))
                    .collect(),
                self.substitute_call_target(target, substitution),
            ),
            HirExprKind::MethodCall(recv, method, args, mode, target) => {
                let mut target = target.clone();
                target
                    .for_each_type_mut(|ty| *ty = self.substitute_type_with_map(ty, substitution));

                HirExprKind::MethodCall(
                    Box::new(self.substitute_expr(recv, substitution)),
                    method.clone(),
                    args.iter()
                        .map(|a| self.substitute_expr(a, substitution))
                        .collect(),
                    *mode,
                    target,
                )
            }
            HirExprKind::Try {
                expr,
                branch_method,
                branch_target,
                branch_self_receiver,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                control_flow_enum,
                break_variant,
                continue_variant,
            } => HirExprKind::Try {
                expr: Box::new(self.substitute_expr(expr, substitution)),
                branch_method: {
                    let mut target = branch_method.clone();
                    target.for_each_type_mut(|ty| {
                        *ty = self.substitute_type_with_map(ty, substitution)
                    });
                    target
                },
                branch_target: self.substitute_call_target(branch_target, substitution),
                branch_self_receiver: *branch_self_receiver,
                from_residual_target: self
                    .substitute_required_call_target(from_residual_target, substitution),
                output_ty: self.substitute_type_with_map(output_ty, substitution),
                residual_ty: self.substitute_type_with_map(residual_ty, substitution),
                return_ty: self.substitute_type_with_map(return_ty, substitution),
                control_flow_enum: *control_flow_enum,
                break_variant: break_variant.clone(),
                continue_variant: continue_variant.clone(),
            },
            HirExprKind::FieldAccess(inner, field, location) => {
                let new_inner = self.substitute_expr(inner, substitution);
                HirExprKind::FieldAccess(Box::new(new_inner), field.clone(), location.clone())
            }
            HirExprKind::TupleIndex(inner, idx) => {
                HirExprKind::TupleIndex(Box::new(self.substitute_expr(inner, substitution)), *idx)
            }
            HirExprKind::ArrayLiteral(elems) => HirExprKind::ArrayLiteral(
                elems
                    .iter()
                    .map(|e| self.substitute_expr(e, substitution))
                    .collect(),
            ),
            HirExprKind::ArrayRepeat(value, len) => {
                HirExprKind::ArrayRepeat(Box::new(self.substitute_expr(value, substitution)), *len)
            }
            HirExprKind::TupleLiteral(elems) => HirExprKind::TupleLiteral(
                elems
                    .iter()
                    .map(|e| self.substitute_expr(e, substitution))
                    .collect(),
            ),
            HirExprKind::StructLiteral(name, struct_id, fields) => HirExprKind::StructLiteral(
                name.clone(),
                *struct_id,
                fields
                    .iter()
                    .map(|field| HirStructLiteralField {
                        name: field.name.clone(),
                        value: self.substitute_expr(&field.value, substitution),
                        field: field.field.clone(),
                    })
                    .collect(),
            ),
            HirExprKind::EnumVariant(name1, name2, args, location) => HirExprKind::EnumVariant(
                name1.clone(),
                name2.clone(),
                args.iter()
                    .map(|e| self.substitute_expr(e, substitution))
                    .collect(),
                location.clone(),
            ),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => HirExprKind::If {
                condition: Box::new(self.substitute_expr(condition, substitution)),
                then_branch: self.substitute_block(then_branch, substitution),
                else_branch: else_branch
                    .as_ref()
                    .map(|e| self.substitute_block(e, substitution)),
            },
            HirExprKind::Match { scrutinee, arms } => HirExprKind::Match {
                scrutinee: Box::new(self.substitute_expr(scrutinee, substitution)),
                arms: arms
                    .iter()
                    .map(|arm| HirMatchArm {
                        pattern: self.substitute_pattern(&arm.pattern, substitution),
                        guard: arm
                            .guard
                            .as_ref()
                            .map(|g| self.substitute_expr(g, substitution)),
                        body: self.substitute_block(&arm.body, substitution),
                    })
                    .collect(),
            },
            HirExprKind::While { condition, body } => HirExprKind::While {
                condition: Box::new(self.substitute_expr(condition, substitution)),
                body: self.substitute_block(body, substitution),
            },
            HirExprKind::For {
                var,
                local_id,
                iter,
                body,
            } => HirExprKind::For {
                var: var.clone(),
                local_id: *local_id,
                iter: Box::new(self.substitute_expr(iter, substitution)),
                body: self.substitute_block(body, substitution),
            },
            HirExprKind::Loop(body) => HirExprKind::Loop(self.substitute_block(body, substitution)),
            HirExprKind::Block(body) => {
                HirExprKind::Block(self.substitute_block(body, substitution))
            }
            HirExprKind::UnsafeBlock(body) => {
                HirExprKind::UnsafeBlock(self.substitute_block(body, substitution))
            }
            HirExprKind::Lambda {
                params,
                body,
                captures,
            } => {
                let new_params = params
                    .iter()
                    .map(|p| HirParam {
                        name: p.name.clone(),
                        local_id: p.local_id,
                        ty: self.substitute_type_with_map(&p.ty, substitution),
                        mutable: p.mutable,
                        is_ref: p.is_ref,
                    })
                    .collect();
                let new_captures = captures
                    .iter()
                    .map(|capture| crate::hir::HirClosureCapture {
                        name: capture.name.clone(),
                        local_id: capture.local_id,
                        kind: capture.kind,
                        mutable: capture.mutable,
                        ty: self.substitute_type_with_map(&capture.ty, substitution),
                    })
                    .collect();
                HirExprKind::Lambda {
                    params: new_params,
                    body: self.substitute_block(body, substitution),
                    captures: new_captures,
                }
            }
            HirExprKind::Ref(mutbl, inner) => HirExprKind::Ref(
                mutbl.clone(),
                Box::new(self.substitute_expr(inner, substitution)),
            ),
            HirExprKind::Deref(inner) => {
                HirExprKind::Deref(Box::new(self.substitute_expr(inner, substitution)))
            }
            HirExprKind::Cast(inner, ty) => HirExprKind::Cast(
                Box::new(self.substitute_expr(inner, substitution)),
                self.substitute_type_with_map(ty, substitution),
            ),
            HirExprKind::OwnedObjectCall { owner, args, call } => {
                let mut call = call.clone();
                call.visit_types_mut(&mut |ty| {
                    *ty = self.substitute_type_with_map(ty, substitution)
                });
                HirExprKind::OwnedObjectCall {
                    owner: Box::new(self.substitute_expr(owner, substitution)),
                    args: args
                        .iter()
                        .map(|arg| self.substitute_expr(arg, substitution))
                        .collect(),
                    call,
                }
            }
            HirExprKind::ObjectCoercion(inner, coercion) => {
                let mut coercion = coercion.clone();
                coercion.visit_types_mut(&mut |ty| {
                    *ty = self.substitute_type_with_map(ty, substitution)
                });
                HirExprKind::ObjectCoercion(
                    Box::new(self.substitute_expr(inner, substitution)),
                    coercion,
                )
            }
            HirExprKind::Assign(lhs, rhs) => HirExprKind::Assign(
                Box::new(self.substitute_expr(lhs, substitution)),
                Box::new(self.substitute_expr(rhs, substitution)),
            ),
            HirExprKind::Intrinsic { name, args } => HirExprKind::Intrinsic {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| self.substitute_expr(a, substitution))
                    .collect(),
            },
            _ => expr.kind.clone(),
        };
        HirExpr {
            kind,
            ty,
            span: expr.span.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
    use crate::type_context::{Ty, TypeContext};
    use crate::types::{AssociatedTypeKey, GenericParamId, Type};

    fn def_id(local: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(local))
    }

    #[test]
    fn type_id_substitution_rewrites_generic_projection_args() {
        let owner = def_id(90);
        let trait_id = def_id(91);
        let generic = GenericParamId { owner, index: 0 };
        let assoc = AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: AssocTypeId(0),
        };
        let mut context = TypeContext::new();
        let base = context.intern_ty(Ty::Generic(generic));
        let projection = context.intern_ty(Ty::Projection {
            ty: base,
            trait_id,
            assoc_type: assoc,
            trait_args: vec![base],
        });
        let replacement = context.intern_type(&Type::Struct {
            id: def_id(92),
            args: Vec::new(),
        });
        let mut subst = HashMap::new();
        subst.insert(generic, replacement);

        let rewritten = context.substitute_generics(projection, &subst);

        assert_eq!(
            context.type_for(rewritten),
            Type::Projection {
                ty: Box::new(Type::Struct {
                    id: def_id(92),
                    args: Vec::new(),
                }),
                trait_id,
                assoc_type: assoc,
                trait_args: vec![Type::Struct {
                    id: def_id(92),
                    args: Vec::new(),
                }],
            }
        );
    }
}
