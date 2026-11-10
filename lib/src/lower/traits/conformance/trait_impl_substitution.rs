//! Substitution of trait-impl types into default method bodies.

use crate::hir::*;
use crate::ids::DefId;
use crate::types::{GenericParamId, Type};
use std::collections::HashMap;

pub(super) fn substitute_self(
    ty: &Type,
    impl_type: &Type,
    impl_trait_id: Option<DefId>,
    impl_trait_arg_types: &[Type],
    impl_associated_types: &[HirAssociatedTypeDef],
    generic_subst: &HashMap<GenericParamId, Type>,
) -> Type {
    struct SelfSubstituter<'a> {
        impl_type: &'a Type,
        impl_trait_id: Option<DefId>,
        impl_trait_arg_types: &'a [Type],
        impl_associated_types: &'a [HirAssociatedTypeDef],
        generic_subst: &'a HashMap<GenericParamId, Type>,
    }

    impl crate::type_services::visit::TypeFolder for SelfSubstituter<'_> {
        fn fold_type(&mut self, ty: Type) -> Type {
            match ty {
                Type::Generic(param) => self
                    .generic_subst
                    .get(&param)
                    .cloned()
                    .unwrap_or(Type::Generic(param)),
                Type::Projection {
                    ty,
                    trait_id,
                    assoc_type,
                    trait_args,
                } => {
                    let substituted_ty = self.fold_type(*ty);
                    let substituted_trait_args = trait_args
                        .into_iter()
                        .map(|arg| self.fold_type(arg))
                        .collect::<Vec<_>>();

                    if substituted_ty == *self.impl_type
                        && self.impl_trait_id == Some(trait_id)
                        && assoc_type.owner == trait_id
                        && substituted_trait_args == self.impl_trait_arg_types
                    {
                        if let Some(assoc) = self
                            .impl_associated_types
                            .iter()
                            .find(|item| item.id == assoc_type.assoc_type_id)
                        {
                            return self.fold_type(assoc.ty.clone());
                        }
                    }

                    Type::Projection {
                        ty: Box::new(substituted_ty),
                        trait_id,
                        assoc_type,
                        trait_args: substituted_trait_args,
                    }
                }
                other => crate::type_services::visit::fold_type_children(other, self),
            }
        }
    }

    crate::type_services::visit::fold_type(
        ty.clone(),
        &mut SelfSubstituter {
            impl_type,
            impl_trait_id,
            impl_trait_arg_types,
            impl_associated_types,
            generic_subst,
        },
    )
}

pub(super) fn substitute_trait_impl_types_in_function(
    func: &mut HirFunction,
    impl_type: &Type,
    impl_trait_id: Option<DefId>,
    impl_trait_arg_types: &[Type],
    impl_associated_types: &[HirAssociatedTypeDef],
    generic_subst: &HashMap<GenericParamId, Type>,
) {
    for bounds in func.generic_bounds.values_mut() {
        for bound in bounds {
            for type_arg in &mut bound.type_args {
                *type_arg = substitute_self(
                    type_arg,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
        }
    }
    for predicate in &mut func.generic_bounds.predicates {
        match predicate {
            crate::types::Predicate::Trait { subject, args, .. } => {
                *subject = substitute_self(
                    subject,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
                for arg in args {
                    *arg = substitute_self(
                        arg,
                        impl_type,
                        impl_trait_id,
                        impl_trait_arg_types,
                        impl_associated_types,
                        generic_subst,
                    );
                }
            }
        }
    }

    for param in &mut func.params {
        param.ty = substitute_self(
            &param.ty,
            impl_type,
            impl_trait_id,
            impl_trait_arg_types,
            impl_associated_types,
            generic_subst,
        );
    }
    func.ret_type = substitute_self(
        &func.ret_type,
        impl_type,
        impl_trait_id,
        impl_trait_arg_types,
        impl_associated_types,
        generic_subst,
    );
    substitute_trait_impl_types_in_block(
        &mut func.body,
        impl_type,
        impl_trait_id,
        impl_trait_arg_types,
        impl_associated_types,
        generic_subst,
    );
}

fn substitute_trait_impl_types_in_block(
    block: &mut HirBlock,
    impl_type: &Type,
    impl_trait_id: Option<DefId>,
    impl_trait_arg_types: &[Type],
    impl_associated_types: &[HirAssociatedTypeDef],
    generic_subst: &HashMap<GenericParamId, Type>,
) {
    for stmt in &mut block.stmts {
        substitute_trait_impl_types_in_stmt(
            stmt,
            impl_type,
            impl_trait_id,
            impl_trait_arg_types,
            impl_associated_types,
            generic_subst,
        );
    }
    block.ty = substitute_self(
        &block.ty,
        impl_type,
        impl_trait_id,
        impl_trait_arg_types,
        impl_associated_types,
        generic_subst,
    );
}

fn substitute_trait_impl_types_in_stmt(
    stmt: &mut HirStmt,
    impl_type: &Type,
    impl_trait_id: Option<DefId>,
    impl_trait_arg_types: &[Type],
    impl_associated_types: &[HirAssociatedTypeDef],
    generic_subst: &HashMap<GenericParamId, Type>,
) {
    match stmt {
        HirStmt::Let { ty, value, .. } => {
            *ty = substitute_self(
                ty,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            substitute_trait_impl_types_in_expr(
                value,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
        }
        HirStmt::Expr(value) => substitute_trait_impl_types_in_expr(
            value,
            impl_type,
            impl_trait_id,
            impl_trait_arg_types,
            impl_associated_types,
            generic_subst,
        ),
        HirStmt::Return(value) | HirStmt::Break(value) => {
            if let Some(value) = value {
                substitute_trait_impl_types_in_expr(
                    value,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
        }
        HirStmt::Continue => {}
    }
}

fn substitute_trait_impl_types_in_pattern(
    pattern: &mut HirPattern,
    impl_type: &Type,
    impl_trait_id: Option<DefId>,
    impl_trait_arg_types: &[Type],
    impl_associated_types: &[HirAssociatedTypeDef],
    generic_subst: &HashMap<GenericParamId, Type>,
) {
    match pattern {
        HirPattern::Struct(_, _, type_args, fields) => {
            for type_arg in type_args {
                *type_arg = substitute_self(
                    type_arg,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
            for field in fields {
                substitute_trait_impl_types_in_pattern(
                    &mut field.pattern,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
        }
        HirPattern::Tuple(items) | HirPattern::Or(items) => {
            for nested in items {
                substitute_trait_impl_types_in_pattern(
                    nested,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
        }
        HirPattern::Enum(_, _, _, items) => {
            for nested in items {
                substitute_trait_impl_types_in_pattern(
                    nested,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

fn substitute_trait_impl_types_in_expr(
    expr: &mut HirExpr,
    impl_type: &Type,
    impl_trait_id: Option<DefId>,
    impl_trait_arg_types: &[Type],
    impl_associated_types: &[HirAssociatedTypeDef],
    generic_subst: &HashMap<GenericParamId, Type>,
) {
    expr.ty = substitute_self(
        &expr.ty,
        impl_type,
        impl_trait_id,
        impl_trait_arg_types,
        impl_associated_types,
        generic_subst,
    );

    match &mut expr.kind {
        HirExprKind::ArrayLiteral(elems) | HirExprKind::TupleLiteral(elems) => {
            for elem in elems {
                substitute_trait_impl_types_in_expr(
                    elem,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
        }
        HirExprKind::ArrayRepeat(value, _) => {
            substitute_trait_impl_types_in_expr(
                value,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
        }
        HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::TupleIndex(inner, _)
        | HirExprKind::UnaryOp(_, inner)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Deref(inner) => substitute_trait_impl_types_in_expr(
            inner,
            impl_type,
            impl_trait_id,
            impl_trait_arg_types,
            impl_associated_types,
            generic_subst,
        ),
        HirExprKind::Cast(inner, ty) => {
            substitute_trait_impl_types_in_expr(
                inner,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            *ty = substitute_self(
                ty,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
        }
        HirExprKind::BinOp(_, lhs, rhs) | HirExprKind::Assign(lhs, rhs) => {
            substitute_trait_impl_types_in_expr(
                lhs,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            substitute_trait_impl_types_in_expr(
                rhs,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
        }
        HirExprKind::Call(func, args, _) => {
            substitute_trait_impl_types_in_expr(
                func,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            for arg in args {
                substitute_trait_impl_types_in_expr(
                    arg,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
        }
        HirExprKind::MethodCall(func, _, args, _, target) => {
            substitute_trait_impl_types_in_expr(
                func,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            for arg in args {
                substitute_trait_impl_types_in_expr(
                    arg,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
            if let Some(target) = target {
                target.for_each_type_mut(|ty| {
                    *ty = substitute_self(
                        ty,
                        impl_type,
                        impl_trait_id,
                        impl_trait_arg_types,
                        impl_associated_types,
                        generic_subst,
                    );
                });
            }
        }
        HirExprKind::Try {
            expr,
            branch_method,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            ..
        } => {
            substitute_trait_impl_types_in_expr(
                expr,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            if let Some(target) = branch_method {
                target.for_each_type_mut(|ty| {
                    *ty = substitute_self(
                        ty,
                        impl_type,
                        impl_trait_id,
                        impl_trait_arg_types,
                        impl_associated_types,
                        generic_subst,
                    );
                });
            }
            if let Some(HirCallTarget::StaticMethod(target)) = from_residual_target {
                target.owner_ty = substitute_self(
                    &target.owner_ty,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
                target.method.for_each_type_mut(|ty| {
                    *ty = substitute_self(
                        ty,
                        impl_type,
                        impl_trait_id,
                        impl_trait_arg_types,
                        impl_associated_types,
                        generic_subst,
                    );
                });
            }
            *output_ty = substitute_self(
                output_ty,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            *residual_ty = substitute_self(
                residual_ty,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            *return_ty = substitute_self(
                return_ty,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                substitute_trait_impl_types_in_expr(
                    &mut field.value,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
        }
        HirExprKind::EnumVariant(_, _, args, _) | HirExprKind::Intrinsic { args, .. } => {
            for arg in args {
                substitute_trait_impl_types_in_expr(
                    arg,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            substitute_trait_impl_types_in_expr(
                condition,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            substitute_trait_impl_types_in_block(
                then_branch,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            if let Some(else_branch) = else_branch {
                substitute_trait_impl_types_in_block(
                    else_branch,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            substitute_trait_impl_types_in_expr(
                scrutinee,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            for arm in arms {
                substitute_trait_impl_types_in_pattern(
                    &mut arm.pattern,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
                if let Some(guard) = &mut arm.guard {
                    substitute_trait_impl_types_in_expr(
                        guard,
                        impl_type,
                        impl_trait_id,
                        impl_trait_arg_types,
                        impl_associated_types,
                        generic_subst,
                    );
                }
                substitute_trait_impl_types_in_block(
                    &mut arm.body,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
        }
        HirExprKind::While { condition, body } => {
            substitute_trait_impl_types_in_expr(
                condition,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            substitute_trait_impl_types_in_block(
                body,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
        }
        HirExprKind::For { iter, body, .. } => {
            substitute_trait_impl_types_in_expr(
                iter,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
            substitute_trait_impl_types_in_block(
                body,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
        }
        HirExprKind::Loop(body) | HirExprKind::Block(body) | HirExprKind::UnsafeBlock(body) => {
            substitute_trait_impl_types_in_block(
                body,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
        }
        HirExprKind::Lambda {
            params,
            body,
            captures,
        } => {
            for param in params {
                param.ty = substitute_self(
                    &param.ty,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
            for capture in captures {
                capture.ty = substitute_self(
                    &capture.ty,
                    impl_type,
                    impl_trait_id,
                    impl_trait_arg_types,
                    impl_associated_types,
                    generic_subst,
                );
            }
            substitute_trait_impl_types_in_block(
                body,
                impl_type,
                impl_trait_id,
                impl_trait_arg_types,
                impl_associated_types,
                generic_subst,
            );
        }
        HirExprKind::IntLiteral(_) | HirExprKind::ConstParam(_)
        | HirExprKind::FloatLiteral(_)
        | HirExprKind::BoolLiteral(_)
        | HirExprKind::StringLiteral(_)
        | HirExprKind::CharLiteral(_)
        | HirExprKind::Unit
        | HirExprKind::Var(_)
        | HirExprKind::ResolvedVar(_) => {}
    }
}
