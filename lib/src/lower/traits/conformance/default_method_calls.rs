//! Retargeting of generated default-method calls.

use crate::hir::*;
use crate::ids::DefId;
use crate::types::Type;
use std::collections::HashMap;

pub(super) fn retarget_generated_default_trait_method_calls(func: &mut HirFunction, imp: &HirImpl) {
    let Some(trait_id) = imp.trait_id else {
        return;
    };

    retarget_generated_default_trait_method_calls_in_block(
        &mut func.body,
        trait_id,
        imp.id,
        &imp.methods,
    );
}

fn retarget_generated_default_trait_method_calls_in_block(
    block: &mut HirBlock,
    trait_id: DefId,
    impl_id: DefId,
    impl_methods: &HashMap<String, HirFunction>,
) {
    for stmt in &mut block.stmts {
        match stmt {
            HirStmt::Let { value, .. } | HirStmt::Expr(value) => {
                retarget_generated_default_trait_method_calls_in_expr(
                    value,
                    trait_id,
                    impl_id,
                    impl_methods,
                );
            }
            HirStmt::Return(Some(value)) | HirStmt::Break(Some(value)) => {
                retarget_generated_default_trait_method_calls_in_expr(
                    value,
                    trait_id,
                    impl_id,
                    impl_methods,
                );
            }
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
        }
    }
}

fn retarget_generated_default_trait_method_calls_in_expr(
    expr: &mut HirExpr,
    trait_id: DefId,
    impl_id: DefId,
    impl_methods: &HashMap<String, HirFunction>,
) {
    match &mut expr.kind {
        HirExprKind::Call(func, args, _) => {
            retarget_generated_default_trait_method_calls_in_expr(
                func,
                trait_id,
                impl_id,
                impl_methods,
            );
            for arg in args {
                retarget_generated_default_trait_method_calls_in_expr(
                    arg,
                    trait_id,
                    impl_id,
                    impl_methods,
                );
            }
        }
        HirExprKind::MethodCall(recv, method_name, args, _, target) => {
            let receiver_is_generated_self = generated_default_method_receiver_is_self(recv);
            retarget_generated_default_trait_method_calls_in_expr(
                recv,
                trait_id,
                impl_id,
                impl_methods,
            );
            for arg in args {
                retarget_generated_default_trait_method_calls_in_expr(
                    arg,
                    trait_id,
                    impl_id,
                    impl_methods,
                );
            }
            if let Some(target) = target {
                if receiver_is_generated_self
                    && target.impl_id().is_none()
                    && target.trait_id() == Some(trait_id)
                {
                    if let Some(method) = impl_methods.get(method_name) {
                        let selected_trait = match &target.target {
                            crate::hir::HirSelectedMethodTarget::TraitMethod {
                                member_id,
                                trait_args,
                                ..
                            } => Some(crate::hir::HirSelectedTraitMember {
                                trait_id,
                                member_id: *member_id,
                                trait_args: trait_args.clone(),
                            }),
                            _ => None,
                        };
                        target.set_impl_method(impl_id, method.id, selected_trait);
                        let mut owner_params = std::collections::HashSet::new();
                        recv.ty.collect_generic_params(&mut owner_params);
                        let mut owner_params = owner_params
                            .into_iter()
                            .filter(|param| param.owner == impl_id)
                            .collect::<Vec<_>>();
                        owner_params.sort_by_key(|param| param.index);
                        target.owner_substitution = owner_params
                            .into_iter()
                            .map(|param| crate::hir::HirTypeBinding {
                                param,
                                ty: Type::Generic(param),
                            })
                            .collect();
                    }
                }
            }
        }
        HirExprKind::Try { expr, .. } => {
            retarget_generated_default_trait_method_calls_in_expr(
                expr,
                trait_id,
                impl_id,
                impl_methods,
            );
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            retarget_generated_default_trait_method_calls_in_expr(
                condition,
                trait_id,
                impl_id,
                impl_methods,
            );
            retarget_generated_default_trait_method_calls_in_block(
                then_branch,
                trait_id,
                impl_id,
                impl_methods,
            );
            if let Some(else_branch) = else_branch {
                retarget_generated_default_trait_method_calls_in_block(
                    else_branch,
                    trait_id,
                    impl_id,
                    impl_methods,
                );
            }
        }
        HirExprKind::Block(block) | HirExprKind::Loop(block) | HirExprKind::UnsafeBlock(block) => {
            retarget_generated_default_trait_method_calls_in_block(
                block,
                trait_id,
                impl_id,
                impl_methods,
            );
        }
        HirExprKind::Lambda { body, .. } => {
            retarget_generated_default_trait_method_calls_in_block(
                body,
                trait_id,
                impl_id,
                impl_methods,
            );
        }
        HirExprKind::While { condition, body } => {
            retarget_generated_default_trait_method_calls_in_expr(
                condition,
                trait_id,
                impl_id,
                impl_methods,
            );
            retarget_generated_default_trait_method_calls_in_block(
                body,
                trait_id,
                impl_id,
                impl_methods,
            );
        }
        HirExprKind::For { iter, body, .. } => {
            retarget_generated_default_trait_method_calls_in_expr(
                iter,
                trait_id,
                impl_id,
                impl_methods,
            );
            retarget_generated_default_trait_method_calls_in_block(
                body,
                trait_id,
                impl_id,
                impl_methods,
            );
        }
        HirExprKind::Assign(lhs, rhs) | HirExprKind::BinOp(_, lhs, rhs) => {
            retarget_generated_default_trait_method_calls_in_expr(
                lhs,
                trait_id,
                impl_id,
                impl_methods,
            );
            retarget_generated_default_trait_method_calls_in_expr(
                rhs,
                trait_id,
                impl_id,
                impl_methods,
            );
        }
        HirExprKind::UnaryOp(_, inner)
        | HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::TupleIndex(inner, _)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Deref(inner)
        | HirExprKind::Cast(inner, _) => {
            retarget_generated_default_trait_method_calls_in_expr(
                inner,
                trait_id,
                impl_id,
                impl_methods,
            );
        }
        HirExprKind::Intrinsic { args, .. }
        | HirExprKind::ArrayLiteral(args)
        | HirExprKind::TupleLiteral(args)
        | HirExprKind::EnumVariant(_, _, args, _) => {
            for arg in args {
                retarget_generated_default_trait_method_calls_in_expr(
                    arg,
                    trait_id,
                    impl_id,
                    impl_methods,
                );
            }
        }
        HirExprKind::ArrayRepeat(value, _) => {
            retarget_generated_default_trait_method_calls_in_expr(
                value,
                trait_id,
                impl_id,
                impl_methods,
            );
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                retarget_generated_default_trait_method_calls_in_expr(
                    &mut field.value,
                    trait_id,
                    impl_id,
                    impl_methods,
                );
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            retarget_generated_default_trait_method_calls_in_expr(
                scrutinee,
                trait_id,
                impl_id,
                impl_methods,
            );
            for arm in arms {
                if let Some(guard) = &mut arm.guard {
                    retarget_generated_default_trait_method_calls_in_expr(
                        guard,
                        trait_id,
                        impl_id,
                        impl_methods,
                    );
                }
                retarget_generated_default_trait_method_calls_in_block(
                    &mut arm.body,
                    trait_id,
                    impl_id,
                    impl_methods,
                );
            }
        }
        HirExprKind::Var(_)
        | HirExprKind::ResolvedVar(_)
        | HirExprKind::IntLiteral(_) | HirExprKind::ConstParam(_)
        | HirExprKind::FloatLiteral(_)
        | HirExprKind::BoolLiteral(_)
        | HirExprKind::StringLiteral(_)
        | HirExprKind::CharLiteral(_)
        | HirExprKind::Unit => {}
    }
}

fn generated_default_method_receiver_is_self(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::Var(name) => name == "self",
        HirExprKind::ResolvedVar(reference) => reference.name == "self",
        HirExprKind::Deref(inner) | HirExprKind::Ref(_, inner) | HirExprKind::Cast(inner, _) => {
            generated_default_method_receiver_is_self(inner)
        }
        _ => false,
    }
}
