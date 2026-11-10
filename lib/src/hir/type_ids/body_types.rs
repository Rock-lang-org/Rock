use super::*;

fn child_path(path: &[usize], segment: usize) -> Vec<usize> {
    let mut child = path.to_vec();
    child.push(segment);
    child
}

pub(super) fn collect_block<P: HirPhase>(
    owner: DefId,
    block: &HirBlockFor<P>,
    path: Vec<usize>,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    record_type(
        context,
        ids,
        HirTypeLocation::Block {
            owner,
            path: path.clone(),
        },
        &block.ty,
    );

    for (index, stmt) in block.stmts.iter().enumerate() {
        let stmt_path = child_path(&path, index);
        collect_stmt(owner, stmt, stmt_path, context, ids);
    }
}

fn collect_stmt<P: HirPhase>(
    owner: DefId,
    stmt: &HirStmtFor<P>,
    path: Vec<usize>,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    match stmt {
        HirStmtFor::Let {
            name, ty, value, ..
        } => {
            record_type(
                context,
                ids,
                HirTypeLocation::LetStmt {
                    owner,
                    path: path.clone(),
                    name: name.clone(),
                },
                ty,
            );
            collect_expr(owner, value, child_path(&path, 0), context, ids);
        }
        HirStmtFor::Expr(expr) => collect_expr(owner, expr, child_path(&path, 0), context, ids),
        HirStmtFor::Return(Some(expr)) => {
            collect_expr(owner, expr, child_path(&path, 0), context, ids)
        }
        HirStmtFor::Break(Some(expr)) => {
            collect_expr(owner, expr, child_path(&path, 0), context, ids)
        }
        HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => {}
    }
}

pub(crate) fn collect_pattern_type_ids(
    owner: DefId,
    pattern: &HirPattern,
    path: Vec<usize>,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    match pattern {
        HirPattern::Struct(_, _, type_args, fields) => {
            for (index, ty) in type_args.iter().enumerate() {
                record_type(
                    context,
                    ids,
                    HirTypeLocation::StructPatternArg {
                        owner,
                        path: path.clone(),
                        index,
                    },
                    ty,
                );
            }
            for (index, field) in fields.iter().enumerate() {
                collect_pattern_type_ids(
                    owner,
                    &field.pattern,
                    child_path(&path, index),
                    context,
                    ids,
                );
            }
        }
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for (index, pattern) in patterns.iter().enumerate() {
                collect_pattern_type_ids(owner, pattern, child_path(&path, index), context, ids);
            }
        }
        HirPattern::Enum(_, _, _, patterns) => {
            for (index, pattern) in patterns.iter().enumerate() {
                collect_pattern_type_ids(owner, pattern, child_path(&path, index), context, ids);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

fn collect_expr<P: HirPhase>(
    owner: DefId,
    expr: &HirExprFor<P>,
    path: Vec<usize>,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    record_type(
        context,
        ids,
        HirTypeLocation::Expr {
            owner,
            path: path.clone(),
        },
        &expr.ty,
    );

    match &expr.kind {
        HirExprKind::ArrayLiteral(elems) | HirExprKind::TupleLiteral(elems) => {
            for (index, elem) in elems.iter().enumerate() {
                collect_expr(owner, elem, child_path(&path, index), context, ids);
            }
        }
        HirExprKind::ArrayRepeat(value, _) => {
            collect_expr(owner, value, child_path(&path, 0), context, ids);
        }
        HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::TupleIndex(inner, _)
        | HirExprKind::UnaryOp(_, inner)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Deref(inner) => {
            collect_expr(owner, inner, child_path(&path, 0), context, ids);
        }
        HirExprKind::Cast(inner, target_ty) => {
            record_type(
                context,
                ids,
                HirTypeLocation::CastTarget {
                    owner,
                    path: path.clone(),
                },
                target_ty,
            );
            collect_expr(owner, inner, child_path(&path, 0), context, ids);
        }
        HirExprKind::BinOp(_, receiver, index) => {
            collect_expr(owner, receiver, child_path(&path, 0), context, ids);
            collect_expr(owner, index, child_path(&path, 1), context, ids);
        }
        HirExprKind::Call(function, args, target) => {
            if let Some(crate::hir::HirCallTarget::StaticMethod(target)) = target {
                record_type(
                    context,
                    ids,
                    HirTypeLocation::StaticCallOwner {
                        owner,
                        path: path.clone(),
                    },
                    &target.owner_ty,
                );
                collect_method_target_types(
                    owner,
                    &path,
                    &target.method,
                    context,
                    ids,
                    |index| HirTypeLocation::StaticCallTraitArg {
                        owner,
                        path: path.clone(),
                        index,
                    },
                    |index| HirTypeLocation::StaticCallOwnerSubstitution {
                        owner,
                        path: path.clone(),
                        index,
                    },
                    |index| HirTypeLocation::StaticCallMethodSubstitution {
                        owner,
                        path: path.clone(),
                        index,
                    },
                );
            }
            collect_expr(owner, function, child_path(&path, 0), context, ids);
            for (index, arg) in args.iter().enumerate() {
                collect_expr(owner, arg, child_path(&path, index + 1), context, ids);
            }
        }
        HirExprKind::MethodCall(receiver, _, args, _, target) => {
            if let Some(target) = P::method_authority(target) {
                collect_method_target_types(
                    owner,
                    &path,
                    target,
                    context,
                    ids,
                    |index| HirTypeLocation::MethodCallTraitArg {
                        owner,
                        path: path.clone(),
                        index,
                    },
                    |index| HirTypeLocation::MethodCallOwnerSubstitution {
                        owner,
                        path: path.clone(),
                        index,
                    },
                    |index| HirTypeLocation::MethodCallMethodSubstitution {
                        owner,
                        path: path.clone(),
                        index,
                    },
                );
            }
            collect_expr(owner, receiver, child_path(&path, 0), context, ids);
            for (index, arg) in args.iter().enumerate() {
                collect_expr(owner, arg, child_path(&path, index + 1), context, ids);
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
            record_type(
                context,
                ids,
                HirTypeLocation::TryOutput {
                    owner,
                    path: path.clone(),
                },
                output_ty,
            );
            record_type(
                context,
                ids,
                HirTypeLocation::TryResidual {
                    owner,
                    path: path.clone(),
                },
                residual_ty,
            );
            record_type(
                context,
                ids,
                HirTypeLocation::TryReturn {
                    owner,
                    path: path.clone(),
                },
                return_ty,
            );
            if let Some(target) = P::method_authority(branch_method) {
                collect_method_target_types(
                    owner,
                    &path,
                    target,
                    context,
                    ids,
                    |index| HirTypeLocation::TryBranchTraitArg {
                        owner,
                        path: path.clone(),
                        index,
                    },
                    |index| HirTypeLocation::TryBranchOwnerSubstitution {
                        owner,
                        path: path.clone(),
                        index,
                    },
                    |index| HirTypeLocation::TryBranchMethodSubstitution {
                        owner,
                        path: path.clone(),
                        index,
                    },
                );
            }
            if let Some(crate::hir::HirCallTarget::StaticMethod(target)) =
                P::residual_authority(from_residual_target)
            {
                record_type(
                    context,
                    ids,
                    HirTypeLocation::TryFromResidualStaticCallOwner {
                        owner,
                        path: path.clone(),
                    },
                    &target.owner_ty,
                );
                collect_method_target_types(
                    owner,
                    &path,
                    &target.method,
                    context,
                    ids,
                    |index| HirTypeLocation::TryFromResidualStaticCallTraitArg {
                        owner,
                        path: path.clone(),
                        index,
                    },
                    |index| HirTypeLocation::TryFromResidualStaticCallOwnerSubstitution {
                        owner,
                        path: path.clone(),
                        index,
                    },
                    |index| HirTypeLocation::TryFromResidualStaticCallMethodSubstitution {
                        owner,
                        path: path.clone(),
                        index,
                    },
                );
            }
            collect_expr(owner, expr, child_path(&path, 0), context, ids);
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for (index, field) in fields.iter().enumerate() {
                collect_expr(owner, &field.value, child_path(&path, index), context, ids);
            }
        }
        HirExprKind::EnumVariant(_, _, args, _) | HirExprKind::Intrinsic { args, .. } => {
            for (index, arg) in args.iter().enumerate() {
                collect_expr(owner, arg, child_path(&path, index), context, ids);
            }
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_expr(owner, condition, child_path(&path, 0), context, ids);
            collect_block(owner, then_branch, child_path(&path, 1), context, ids);
            if let Some(else_branch) = else_branch {
                collect_block(owner, else_branch, child_path(&path, 2), context, ids);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            collect_expr(owner, scrutinee, child_path(&path, 0), context, ids);
            for (index, arm) in arms.iter().enumerate() {
                collect_pattern_type_ids(
                    owner,
                    &arm.pattern,
                    child_path(&path, 1000 + index),
                    context,
                    ids,
                );
                if let Some(guard) = &arm.guard {
                    collect_expr(owner, guard, child_path(&path, 1 + index * 2), context, ids);
                }
                collect_block(
                    owner,
                    &arm.body,
                    child_path(&path, 2 + index * 2),
                    context,
                    ids,
                );
            }
        }
        HirExprKind::While { condition, body } => {
            collect_expr(owner, condition, child_path(&path, 0), context, ids);
            collect_block(owner, body, child_path(&path, 1), context, ids);
        }
        HirExprKind::For { iter, body, .. } => {
            collect_expr(owner, iter, child_path(&path, 0), context, ids);
            collect_block(owner, body, child_path(&path, 1), context, ids);
        }
        HirExprKind::Loop(body) | HirExprKind::Block(body) | HirExprKind::UnsafeBlock(body) => {
            collect_block(owner, body, child_path(&path, 0), context, ids);
        }
        HirExprKind::Lambda {
            params,
            body,
            captures,
        } => {
            for (index, param) in params.iter().enumerate() {
                record_type(
                    context,
                    ids,
                    HirTypeLocation::ClosureParam {
                        owner,
                        path: path.clone(),
                        index,
                    },
                    &param.ty,
                );
            }
            for capture in captures {
                record_type(
                    context,
                    ids,
                    HirTypeLocation::ClosureCapture {
                        owner,
                        path: path.clone(),
                        name: capture.name.clone(),
                    },
                    &capture.ty,
                );
            }
            collect_block(owner, body, child_path(&path, 1), context, ids);
        }
        HirExprKind::Assign(lhs, rhs) => {
            collect_expr(owner, lhs, child_path(&path, 0), context, ids);
            collect_expr(owner, rhs, child_path(&path, 1), context, ids);
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

pub(super) fn collect_generic_bound_args(
    generic_bounds: &HashMap<GenericParamId, Vec<TraitBound>>,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
    location: impl Fn(GenericParamId, usize, usize) -> HirTypeLocation,
) {
    let mut bounds: Vec<_> = generic_bounds.iter().collect();
    bounds.sort_by_key(|(param, _)| (param.owner, param.index));
    for (param, bounds) in bounds {
        for (bound_index, bound) in bounds.iter().enumerate() {
            collect_bound_type_args(bound, context, ids, |arg_index| {
                location(*param, bound_index, arg_index)
            });
        }
    }
}

fn collect_method_target_types(
    _owner: DefId,
    _path: &[usize],
    target: &crate::hir::HirMethodCallTarget,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
    trait_location: impl Fn(usize) -> HirTypeLocation,
    owner_substitution_location: impl Fn(usize) -> HirTypeLocation,
    method_substitution_location: impl Fn(usize) -> HirTypeLocation,
) {
    for (index, ty) in target.trait_args().iter().enumerate() {
        record_type(context, ids, trait_location(index), ty);
    }
    for (index, binding) in target.owner_substitution.iter().enumerate() {
        record_type(
            context,
            ids,
            owner_substitution_location(index),
            &binding.ty,
        );
    }
    for (index, binding) in target.method_substitution.iter().enumerate() {
        record_type(
            context,
            ids,
            method_substitution_location(index),
            &binding.ty,
        );
    }
}

pub(super) fn collect_bound_type_args(
    bound: &TraitBound,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
    location: impl Fn(usize) -> HirTypeLocation,
) {
    for (arg_index, ty) in bound.type_args.iter().enumerate() {
        record_type(context, ids, location(arg_index), ty);
    }
}
