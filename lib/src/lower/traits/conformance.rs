//! Trait conformance checking

use std::collections::{BTreeSet, HashMap};

use crate::collect::resolver::ResolverTables;
use crate::hir::*;
use crate::ids::{AssocTypeId, CrateId, DefId, IdGen, LocalDefId};
use crate::infer::InferenceEngine;
use crate::lower::items::LowerItems;
use crate::type_services::visit::remap_generic_params_in_place;
use crate::types::{GenericParamId, TraitBound, Type};

use crate::lower::{Lowerer, ResolveError};

pub(crate) struct DefaultMethodInjectionContext<'a> {
    trait_name: &'a str,
}

pub(crate) struct TraitDefaultMethodInjector<'a> {
    engine: &'a mut InferenceEngine,
    root_crate_id: CrateId,
    local_def_ids: &'a mut IdGen<LocalDefId>,
    current_def_ids: &'a mut BTreeSet<DefId>,
    source_map: &'a crate::source_map::SemanticSourceMap,
    diagnostics: &'a mut Vec<ResolveError>,
}

impl<'a> TraitDefaultMethodInjector<'a> {
    pub(crate) fn new(
        engine: &'a mut InferenceEngine,
        root_crate_id: CrateId,
        local_def_ids: &'a mut IdGen<LocalDefId>,
        current_def_ids: &'a mut BTreeSet<DefId>,
        source_map: &'a crate::source_map::SemanticSourceMap,
        diagnostics: &'a mut Vec<ResolveError>,
    ) -> Self {
        Self {
            engine,
            root_crate_id,
            local_def_ids,
            current_def_ids,
            source_map,
            diagnostics,
        }
    }

    pub(crate) fn prepare_missing_default_method(
        &mut self,
        imp: &mut HirImpl,
        method_name: &str,
        default_func: &HirFunction,
        self_type: &Type,
        context: DefaultMethodInjectionContext<'_>,
        prepare_func: impl FnOnce(&mut HirFunction),
    ) -> HirFunction {
        let mut func = default_func.clone();
        let default_method_id = func.id;
        let generated_method_id = DefId::new(self.root_crate_id, self.local_def_ids.fresh());
        self.current_def_ids.insert(generated_method_id);
        func.id = generated_method_id;
        remap_function_generic_owner(&mut func, default_method_id, generated_method_id);

        prepare_func(&mut func);
        retarget_generated_default_trait_method_calls(&mut func, imp);

        if let Err(err) = self.engine.unify(&func.body.ty, &func.ret_type) {
            let message = format!(
                "default method '{}.{}' return type mismatch: {}",
                context.trait_name,
                method_name,
                err.render(&self.engine)
            );
            if let Some(span) = self
                .source_map
                .definition_declaration_span(default_method_id)
                .cloned()
            {
                self.diagnostics.push(ResolveError::with_span_code(
                    message,
                    span,
                    crate::diagnostic::DiagnosticCode::Type,
                ));
            } else {
                self.diagnostics.push(ResolveError::non_source(message));
            }
        }
        resolve_generated_default_types(self.engine, &mut func);
        apply_generated_default_self_type(&mut func, self_type);

        func
    }
}

fn retarget_generated_default_trait_method_calls(func: &mut HirFunction, imp: &HirImpl) {
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
        | HirExprKind::IntLiteral(_)
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

fn apply_generated_default_self_type(func: &mut HirFunction, self_type: &Type) {
    if func.self_receiver.is_none() {
        return;
    }
    let Some(existing_param_ty) = func.params.first().map(|param| param.ty.clone()) else {
        return;
    };

    let param_ty = match func.self_receiver {
        Some(crate::types::ReceiverMode::Move) => self_type.clone(),
        Some(crate::types::ReceiverMode::Shared) => Type::Reference {
            mutable: false,
            inner: Box::new(self_type.clone()),
        },
        Some(crate::types::ReceiverMode::Mut) => Type::Reference {
            mutable: true,
            inner: Box::new(self_type.clone()),
        },
        None => match existing_param_ty {
            Type::Reference { mutable, .. } => Type::Reference {
                mutable,
                inner: Box::new(self_type.clone()),
            },
            _ => self_type.clone(),
        },
    };

    if let Some(param) = func.params.first_mut() {
        if param.name == "self" {
            param.ty = param_ty.clone();
        }
    }
    apply_generated_default_self_type_in_block(&mut func.body, &param_ty, self_type);
}

fn apply_generated_default_self_type_in_block(
    block: &mut HirBlock,
    param_ty: &Type,
    self_type: &Type,
) {
    for stmt in &mut block.stmts {
        match stmt {
            HirStmt::Let { value, .. } | HirStmt::Expr(value) => {
                apply_generated_default_self_type_in_expr(value, param_ty, self_type);
            }
            HirStmt::Return(Some(value)) | HirStmt::Break(Some(value)) => {
                apply_generated_default_self_type_in_expr(value, param_ty, self_type);
            }
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
        }
    }
}

fn apply_generated_default_self_type_in_expr(
    expr: &mut HirExpr,
    param_ty: &Type,
    self_type: &Type,
) {
    match &mut expr.kind {
        HirExprKind::Var(name) if name == "self" => {
            expr.ty = param_ty.clone();
        }
        HirExprKind::Call(func, args, _) => {
            apply_generated_default_self_type_in_expr(func, param_ty, self_type);
            for arg in args {
                apply_generated_default_self_type_in_expr(arg, param_ty, self_type);
            }
        }
        HirExprKind::MethodCall(recv, _, args, _, _) => {
            apply_generated_default_self_type_in_expr(recv, param_ty, self_type);
            for arg in args {
                apply_generated_default_self_type_in_expr(arg, param_ty, self_type);
            }
        }
        HirExprKind::Try { expr, .. } => {
            apply_generated_default_self_type_in_expr(expr, param_ty, self_type);
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            apply_generated_default_self_type_in_expr(condition, param_ty, self_type);
            apply_generated_default_self_type_in_block(then_branch, param_ty, self_type);
            if let Some(else_branch) = else_branch {
                apply_generated_default_self_type_in_block(else_branch, param_ty, self_type);
            }
        }
        HirExprKind::Block(block) | HirExprKind::Loop(block) | HirExprKind::UnsafeBlock(block) => {
            apply_generated_default_self_type_in_block(block, param_ty, self_type);
        }
        HirExprKind::Lambda { body, .. } => {
            apply_generated_default_self_type_in_block(body, param_ty, self_type);
        }
        HirExprKind::While { condition, body } => {
            apply_generated_default_self_type_in_expr(condition, param_ty, self_type);
            apply_generated_default_self_type_in_block(body, param_ty, self_type);
        }
        HirExprKind::For { iter, body, .. } => {
            apply_generated_default_self_type_in_expr(iter, param_ty, self_type);
            apply_generated_default_self_type_in_block(body, param_ty, self_type);
        }
        HirExprKind::Assign(lhs, rhs) | HirExprKind::BinOp(_, lhs, rhs) => {
            apply_generated_default_self_type_in_expr(lhs, param_ty, self_type);
            apply_generated_default_self_type_in_expr(rhs, param_ty, self_type);
        }
        HirExprKind::Deref(inner) => {
            apply_generated_default_self_type_in_expr(inner, param_ty, self_type);
            if inner.ty == *param_ty {
                expr.ty = self_type.clone();
            }
        }
        HirExprKind::UnaryOp(_, inner)
        | HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::TupleIndex(inner, _)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Cast(inner, _) => {
            apply_generated_default_self_type_in_expr(inner, param_ty, self_type);
        }
        HirExprKind::Intrinsic { args, .. }
        | HirExprKind::ArrayLiteral(args)
        | HirExprKind::TupleLiteral(args)
        | HirExprKind::EnumVariant(_, _, args, _) => {
            for arg in args {
                apply_generated_default_self_type_in_expr(arg, param_ty, self_type);
            }
        }
        HirExprKind::ArrayRepeat(value, _) => {
            apply_generated_default_self_type_in_expr(value, param_ty, self_type);
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                apply_generated_default_self_type_in_expr(&mut field.value, param_ty, self_type);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            apply_generated_default_self_type_in_expr(scrutinee, param_ty, self_type);
            for arm in arms {
                if let Some(guard) = &mut arm.guard {
                    apply_generated_default_self_type_in_expr(guard, param_ty, self_type);
                }
                apply_generated_default_self_type_in_block(&mut arm.body, param_ty, self_type);
            }
        }
        HirExprKind::Var(_)
        | HirExprKind::ResolvedVar(_)
        | HirExprKind::IntLiteral(_)
        | HirExprKind::FloatLiteral(_)
        | HirExprKind::BoolLiteral(_)
        | HirExprKind::StringLiteral(_)
        | HirExprKind::CharLiteral(_)
        | HirExprKind::Unit => {}
    }
}

fn resolve_generated_default_types(engine: &mut InferenceEngine, func: &mut HirFunction) {
    for param in &mut func.params {
        param.ty = engine.resolve(&param.ty);
    }
    func.ret_type = engine.resolve(&func.ret_type);
    resolve_generated_default_block_types(engine, &mut func.body);
    if !generated_default_uses_generic_type(func) {
        func.generic_params.clear();
        func.generic_bounds.clear();
    }
}

fn generated_default_uses_generic_type(func: &HirFunction) -> bool {
    func.params.iter().any(|param| type_uses_generic(&param.ty))
        || type_uses_generic(&func.ret_type)
        || type_uses_generic(&func.body.ty)
        || bounds_use_generic_type(func)
}

fn bounds_use_generic_type(func: &HirFunction) -> bool {
    func.generic_bounds.iter().any(|(param, bounds)| {
        param.owner == func.id
            || bounds
                .iter()
                .any(|bound| bound.type_args.iter().any(type_uses_generic))
    })
}

fn type_uses_generic(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| {
        matches!(nested, Type::Generic(_) | Type::Projection { .. })
    })
}

fn resolve_generated_default_block_types(engine: &mut InferenceEngine, block: &mut HirBlock) {
    block.ty = engine.resolve(&block.ty);
    for stmt in &mut block.stmts {
        resolve_generated_default_stmt_types(engine, stmt);
    }
    if let Some(HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Break(Some(expr))) =
        block.stmts.last()
    {
        block.ty = expr.ty.clone();
    }
}

fn resolve_generated_default_stmt_types(engine: &mut InferenceEngine, stmt: &mut HirStmt) {
    match stmt {
        HirStmt::Let { ty, value, .. } => {
            *ty = engine.resolve(ty);
            resolve_generated_default_expr_types(engine, value);
        }
        HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Break(Some(expr)) => {
            resolve_generated_default_expr_types(engine, expr);
        }
        HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
    }
}

fn resolve_generated_default_pattern_types(engine: &mut InferenceEngine, pattern: &mut HirPattern) {
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                resolve_generated_default_pattern_types(engine, pattern);
            }
        }
        HirPattern::Struct(_, _, type_args, field_patterns) => {
            for ty in type_args {
                *ty = engine.resolve(ty);
            }
            for field in field_patterns {
                resolve_generated_default_pattern_types(engine, &mut field.pattern);
            }
        }
        HirPattern::Enum(_, _, _, patterns) => {
            for pattern in patterns {
                resolve_generated_default_pattern_types(engine, pattern);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

fn resolve_generated_default_expr_types(engine: &mut InferenceEngine, expr: &mut HirExpr) {
    expr.ty = engine.resolve(&expr.ty);
    match &mut expr.kind {
        HirExprKind::Call(func_expr, args, target) => {
            resolve_generated_default_expr_types(engine, func_expr);
            for arg in args {
                resolve_generated_default_expr_types(engine, arg);
            }
            if let Some(HirCallTarget::StaticMethod(target)) = target {
                target.owner_ty = engine.resolve(&target.owner_ty);
                target
                    .method
                    .for_each_type_mut(|ty| *ty = engine.resolve(ty));
            }
        }
        HirExprKind::MethodCall(recv, _, args, _, target) => {
            resolve_generated_default_expr_types(engine, recv);
            for arg in args {
                resolve_generated_default_expr_types(engine, arg);
            }
            if let Some(target) = target {
                target.for_each_type_mut(|ty| *ty = engine.resolve(ty));
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
            resolve_generated_default_expr_types(engine, expr);
            if let Some(target) = branch_method {
                target.for_each_type_mut(|ty| *ty = engine.resolve(ty));
            }
            if let Some(HirCallTarget::StaticMethod(target)) = from_residual_target {
                target.owner_ty = engine.resolve(&target.owner_ty);
                target
                    .method
                    .for_each_type_mut(|ty| *ty = engine.resolve(ty));
            }
            *output_ty = engine.resolve(output_ty);
            *residual_ty = engine.resolve(residual_ty);
            *return_ty = engine.resolve(return_ty);
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            resolve_generated_default_expr_types(engine, condition);
            resolve_generated_default_block_types(engine, then_branch);
            if let Some(else_branch) = else_branch {
                resolve_generated_default_block_types(engine, else_branch);
            }
        }
        HirExprKind::Block(block) | HirExprKind::Loop(block) | HirExprKind::UnsafeBlock(block) => {
            resolve_generated_default_block_types(engine, block);
        }
        HirExprKind::Lambda {
            params,
            body,
            captures,
        } => {
            for param in params {
                param.ty = engine.resolve(&param.ty);
            }
            for capture in captures {
                capture.ty = engine.resolve(&capture.ty);
            }
            resolve_generated_default_block_types(engine, body);
        }
        HirExprKind::While { condition, body } => {
            resolve_generated_default_expr_types(engine, condition);
            resolve_generated_default_block_types(engine, body);
        }
        HirExprKind::For { iter, body, .. } => {
            resolve_generated_default_expr_types(engine, iter);
            resolve_generated_default_block_types(engine, body);
        }
        HirExprKind::Assign(lhs, rhs) | HirExprKind::BinOp(_, lhs, rhs) => {
            resolve_generated_default_expr_types(engine, lhs);
            resolve_generated_default_expr_types(engine, rhs);
        }
        HirExprKind::UnaryOp(_, inner)
        | HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::TupleIndex(inner, _)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Deref(inner) => {
            resolve_generated_default_expr_types(engine, inner);
        }
        HirExprKind::Cast(inner, ty) => {
            resolve_generated_default_expr_types(engine, inner);
            *ty = engine.resolve(ty);
        }
        HirExprKind::Intrinsic { args, .. }
        | HirExprKind::ArrayLiteral(args)
        | HirExprKind::TupleLiteral(args)
        | HirExprKind::EnumVariant(_, _, args, _) => {
            for arg in args {
                resolve_generated_default_expr_types(engine, arg);
            }
        }
        HirExprKind::ArrayRepeat(value, _) => {
            resolve_generated_default_expr_types(engine, value);
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                resolve_generated_default_expr_types(engine, &mut field.value);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            resolve_generated_default_expr_types(engine, scrutinee);
            for arm in arms {
                resolve_generated_default_pattern_types(engine, &mut arm.pattern);
                if let Some(guard) = &mut arm.guard {
                    resolve_generated_default_expr_types(engine, guard);
                }
                resolve_generated_default_block_types(engine, &mut arm.body);
            }
        }
        HirExprKind::Var(_)
        | HirExprKind::ResolvedVar(_)
        | HirExprKind::IntLiteral(_)
        | HirExprKind::FloatLiteral(_)
        | HirExprKind::BoolLiteral(_)
        | HirExprKind::StringLiteral(_)
        | HirExprKind::CharLiteral(_)
        | HirExprKind::Unit => {}
    }
}

#[derive(Default)]
pub(crate) struct TraitConformanceOutput {
    pub(crate) diagnostics: Vec<ResolveError>,
    pub(crate) effective_trait_methods: HashMap<(DefId, DefId), DefId>,
}

fn collect_effective_trait_methods(
    items: &LowerItems,
    root_crate_id: CrateId,
    output: &mut TraitConformanceOutput,
) {
    for imp in items
        .impl_defs_in_order()
        .map(|(_, imp)| imp)
        .filter(|imp| imp.id.crate_id == root_crate_id)
    {
        let Some(trait_id) = imp.trait_id else {
            continue;
        };
        let Some(trait_def) = items.trait_def(trait_id) else {
            continue;
        };

        for (member_name, member_id) in trait_def
            .methods
            .iter()
            .map(|(name, method)| (name, method.id))
            .chain(
                trait_def
                    .signatures
                    .iter()
                    .filter(|(name, _)| !trait_def.methods.contains_key(*name))
                    .map(|(name, signature)| (name, signature.id)),
            )
        {
            let Some(method_id) = imp.methods.get(member_name).map(|method| method.id) else {
                continue;
            };
            let key = (imp.id, member_id);
            if let Some(previous) = output.effective_trait_methods.insert(key, method_id) {
                if previous != method_id {
                    output.diagnostics.push(ResolveError::non_source(format!(
                        "trait implementation {:?} has conflicting effective bodies {:?} and {:?} for member {:?}",
                        imp.id, previous, method_id, member_id
                    )));
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct IndexProtocolIds {
    index_trait: DefId,
    index_output: AssocTypeId,
    index_mut_trait: DefId,
    index_mut_output: AssocTypeId,
}

pub(crate) struct TraitConformanceContext<'a> {
    items: &'a mut LowerItems,
    engine: &'a mut InferenceEngine,
    resolver: &'a ResolverTables,
    dependency_resolvers: &'a HashMap<String, ResolverTables>,
    root_crate_id: CrateId,
    local_def_ids: &'a mut IdGen<LocalDefId>,
    current_def_ids: &'a mut BTreeSet<DefId>,
    source_map: &'a crate::source_map::SemanticSourceMap,
    index_protocol_ids: Option<IndexProtocolIds>,
}

pub(crate) struct TraitConformanceService<'a> {
    items: &'a mut LowerItems,
    engine: &'a mut InferenceEngine,
    resolver: &'a ResolverTables,
    dependency_resolvers: &'a HashMap<String, ResolverTables>,
    root_crate_id: CrateId,
    local_def_ids: &'a mut IdGen<LocalDefId>,
    current_def_ids: &'a mut BTreeSet<DefId>,
    source_map: &'a crate::source_map::SemanticSourceMap,
    index_protocol_ids: Option<IndexProtocolIds>,
    output: TraitConformanceOutput,
}

pub(crate) struct TraitConformancePhase;

impl<'a> TraitConformanceService<'a> {
    pub(crate) fn new(context: TraitConformanceContext<'a>) -> Self {
        Self {
            items: context.items,
            engine: context.engine,
            resolver: context.resolver,
            dependency_resolvers: context.dependency_resolvers,
            root_crate_id: context.root_crate_id,
            local_def_ids: context.local_def_ids,
            current_def_ids: context.current_def_ids,
            source_map: context.source_map,
            index_protocol_ids: context.index_protocol_ids,
            output: TraitConformanceOutput::default(),
        }
    }

    pub(crate) fn finish(self) -> TraitConformanceOutput {
        self.output
    }
}

impl TraitConformancePhase {
    pub(crate) fn prepare_explicit_method_selection(lowerer: &mut Lowerer) {
        // Default bodies can call concrete trait implementations before defaults
        // are injected. Publish their exact member bindings, not name fallbacks.
        let mut output = TraitConformanceOutput::default();
        collect_effective_trait_methods(&lowerer.items, lowerer.root_crate_id, &mut output);
        lowerer
            .imported_effective_trait_methods
            .extend(output.effective_trait_methods);
        lowerer.diagnostics.extend(output.diagnostics);
    }

    fn annotate_callable_impl_patterns(lowerer: &mut Lowerer) {
        let callable_traits = [
            lowerer
                .language_items
                .fn_once
                .as_ref()
                .map(|items| (items.trait_id, crate::types::CallableKind::FnOnce)),
            lowerer
                .language_items
                .fn_mut
                .as_ref()
                .map(|items| (items.trait_id, crate::types::CallableKind::FnMut)),
            lowerer
                .language_items
                .fn_trait
                .as_ref()
                .map(|items| (items.trait_id, crate::types::CallableKind::Fn)),
        ]
        .into_iter()
        .flatten()
        .collect::<HashMap<_, _>>();
        if callable_traits.is_empty() {
            return;
        }

        let impl_ids = lowerer
            .items
            .impl_defs_in_order()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        for impl_id in impl_ids {
            let Some(imp) = lowerer.items.impl_def_mut(impl_id) else {
                continue;
            };
            let Some(callable_kind) = imp
                .trait_id
                .and_then(|trait_id| callable_traits.get(&trait_id).copied())
            else {
                continue;
            };
            if let HirImplReceiverPattern::Exact(Type::Function {
                callable_kind: pattern_kind,
                ..
            }) = &mut imp.receiver_pattern
            {
                *pattern_kind = callable_kind;
            }
        }
    }

    /// Auto-implement the marked Sized trait for all structs and enums.
    fn auto_impl_sized(lowerer: &mut Lowerer) {
        let Some(sized_trait_id) = lowerer
            .language_items
            .sized
            .as_ref()
            .map(|items| items.trait_id)
        else {
            return;
        };
        let Some(sized_trait_name) = lowerer
            .items
            .trait_def(sized_trait_id)
            .map(|trait_def| trait_def.name.clone())
        else {
            return;
        };

        let mut owner_ids: Vec<DefId> = lowerer
            .items
            .structures()
            .map(|(id, _)| id)
            .chain(lowerer.items.enumerations().map(|(id, _)| id))
            .collect();
        owner_ids.sort();
        for owner_id in owner_ids {
            let Some(name) = lowerer
                .canonical_name_for_def_id(owner_id)
                .map(str::to_string)
            else {
                continue;
            };
            // Check if already has a Sized impl
            let already_has = lowerer
                .items
                .impl_defs_in_order()
                .any(|(_, imp)| imp.type_name == name && imp.trait_id == Some(sized_trait_id));
            if !already_has {
                if !lowerer.current_def_ids.contains(&owner_id) {
                    continue;
                }

                if let Some(owner_path) = lowerer.try_canonical_owner_path(&name) {
                    let id = DefId::new(lowerer.root_crate_id, lowerer.local_def_ids.fresh());
                    lowerer.current_def_ids.insert(id);
                    let (owner_generics, is_struct) =
                        if let Some(structure) = lowerer.items.structure(owner_id) {
                            (structure.generic_params.clone(), true)
                        } else {
                            (
                                lowerer
                                    .items
                                    .enumeration(owner_id)
                                    .expect("Sized owner ID must resolve to a struct or enum")
                                    .generic_params
                                    .clone(),
                                false,
                            )
                        };
                    let type_generics = owner_generics
                        .into_iter()
                        .enumerate()
                        .map(|(index, generic)| {
                            crate::types::GenericParamDecl::new(
                                GenericParamId {
                                    owner: id,
                                    index: index as u32,
                                },
                                generic.name,
                                generic.kind,
                            )
                        })
                        .collect::<Vec<_>>();
                    let type_args: Vec<_> = type_generics
                        .iter()
                        .enumerate()
                        .map(|(index, _)| {
                            Type::Generic(GenericParamId {
                                owner: id,
                                index: index as u32,
                            })
                        })
                        .collect();
                    let receiver_pattern = if is_struct {
                        HirImplReceiverPattern::Exact(Type::Struct {
                            id: owner_id,
                            args: type_args,
                        })
                    } else {
                        HirImplReceiverPattern::Exact(Type::Enum {
                            id: owner_id,
                            args: type_args,
                        })
                    };

                    lowerer
                        .items
                        .insert_impl(HirImpl {
                            id,
                            owner: HirImplOwner::Named(owner_path),
                            type_name: name.to_string(),
                            type_generics,
                            receiver_pattern,
                            trait_name: Some(sized_trait_name.clone()),
                            trait_id: Some(sized_trait_id),
                            trait_generics: vec![],
                            trait_arg_types: vec![],
                            associated_types: vec![],
                            bounds: crate::hir::HirGenericBounds::new(),
                            methods: HashMap::new(),
                        })
                        .expect("fresh automatic Sized impl ID must not collide");
                }
            }
        }
    }

    /// Check that trait impls provide all required methods, and inject defaults
    pub(crate) fn run(lowerer: &mut Lowerer) {
        Self::annotate_callable_impl_patterns(lowerer);
        Self::auto_impl_sized(lowerer);
        let index_protocol_ids = lowerer
            .language_items
            .index
            .as_ref()
            .zip(lowerer.language_items.index_mut.as_ref())
            .map(|(index, index_mut)| IndexProtocolIds {
                index_trait: index.trait_id,
                index_output: index.output_id,
                index_mut_trait: index_mut.trait_id,
                index_mut_output: index_mut.output_id,
            });
        let output = {
            let mut service = TraitConformanceService::new(TraitConformanceContext {
                items: &mut lowerer.items,
                engine: &mut lowerer.engine,
                resolver: &lowerer.resolver,
                dependency_resolvers: &lowerer.dependency_resolvers,
                root_crate_id: lowerer.root_crate_id,
                local_def_ids: &mut lowerer.local_def_ids,
                current_def_ids: &mut lowerer.current_def_ids,
                source_map: &lowerer.source_map,
                index_protocol_ids,
            });
            service.check_trait_conformance();
            service.finish()
        };
        lowerer
            .imported_effective_trait_methods
            .extend(output.effective_trait_methods);
        lowerer.diagnostics.extend(output.diagnostics);
    }
}

#[cfg(test)]
impl Lowerer {
    pub(crate) fn auto_impl_sized(&mut self) {
        TraitConformancePhase::auto_impl_sized(self);
    }

    pub(crate) fn check_trait_conformance(&mut self) {
        TraitConformancePhase::run(self);
    }
}

impl TraitConformanceService<'_> {
    fn display_type(&self, ty: &Type) -> String {
        let mut context =
            crate::type_services::display::TypeDisplayContext::from_resolver(self.resolver);
        for resolver in self.dependency_resolvers.values() {
            context.extend_resolver(resolver);
        }
        for (_, function) in self.items.functions() {
            context.insert_generic_names(&function.generic_params);
        }
        for (_, signature) in self.items.function_sigs() {
            context.insert_generic_names(&signature.generic_params);
        }
        for (_, structure) in self.items.structures() {
            context.insert_generic_names(&structure.generic_params);
        }
        for (_, enumeration) in self.items.enumerations() {
            context.insert_generic_names(&enumeration.generic_params);
        }
        for (_, alias) in self.items.type_aliases() {
            context.insert_generic_names(&alias.generic_params);
        }
        for (_, trait_def) in self.items.trait_defs() {
            context.insert_generic_names(&trait_def.generic_params);
            if let Some(target) = &trait_def.target {
                context.insert_generic_name(target.id, target.name.clone());
            }
            for associated in &trait_def.associated_types {
                context.insert_associated_name(
                    crate::types::AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: associated.id,
                    },
                    associated.name.clone(),
                );
            }
            for function in trait_def.methods.values() {
                context.insert_generic_names(&function.generic_params);
            }
            for signature in trait_def.signatures.values() {
                context.insert_generic_names(&signature.generic_params);
            }
        }
        for (_, impl_def) in self.items.impl_defs() {
            context.insert_generic_names(&impl_def.type_generics);
            context.insert_generic_names(&impl_def.trait_generics);
            for associated in &impl_def.associated_types {
                context.insert_associated_name(
                    crate::types::AssociatedTypeKey {
                        owner: impl_def.id,
                        assoc_type_id: associated.id,
                    },
                    associated.name.clone(),
                );
            }
            for function in impl_def.methods.values() {
                context.insert_generic_names(&function.generic_params);
            }
        }
        crate::type_services::display::display_type_with_context(ty, &context).to_string()
    }

    fn resolve_item_id(&self, name: &str) -> Option<DefId> {
        self.resolver.resolve_item_or_alias(name).or_else(|| {
            name.contains("::").then(|| {
                self.dependency_resolvers
                    .values()
                    .find_map(|resolver| resolver.resolve_item_or_alias(name))
            })?
        })
    }

    fn resolve_module_alias_or_item_id(&self, name: &str) -> Option<DefId> {
        self.resolver
            .module_aliases
            .get(name)
            .copied()
            .or_else(|| self.resolve_item_id(name))
    }

    fn resolve_trait_id(&self, name: &str) -> Option<DefId> {
        self.resolve_trait_type_id(name)
    }

    fn resolve_trait_type_id(&self, name: &str) -> Option<DefId> {
        self.resolve_module_alias_or_item_id(name)
            .and_then(|id| self.items.trait_def(id).map(|trait_def| trait_def.id))
    }

    pub(crate) fn check_trait_conformance(&mut self) {
        fn substitute_self(
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

        fn substitute_trait_impl_types_in_function(
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
                HirExprKind::Loop(body)
                | HirExprKind::Block(body)
                | HirExprKind::UnsafeBlock(body) => {
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
                HirExprKind::IntLiteral(_)
                | HirExprKind::FloatLiteral(_)
                | HirExprKind::BoolLiteral(_)
                | HirExprKind::StringLiteral(_)
                | HirExprKind::CharLiteral(_)
                | HirExprKind::Unit
                | HirExprKind::Var(_)
                | HirExprKind::ResolvedVar(_) => {}
            }
        }

        fn prepare_default_method(
            func: &mut HirFunction,
            self_type: &Type,
            impl_trait_id: Option<DefId>,
            trait_arg_types: &[Type],
            impl_associated_types: &[HirAssociatedTypeDef],
            trait_generic_subst: &HashMap<GenericParamId, Type>,
        ) {
            // Only substitute the self-related TypeVars from the default body.
            if func.self_receiver.is_some() {
                if let Some(source_self_ty) = func.params.first().map(|param| param.ty.clone()) {
                    crate::hir::substitute_typevars_in_function(func, &source_self_ty, self_type);
                }
            }
            substitute_trait_impl_types_in_function(
                func,
                self_type,
                impl_trait_id,
                trait_arg_types,
                impl_associated_types,
                trait_generic_subst,
            );
            apply_generated_default_self_type(func, self_type);
        }

        let impl_ids: Vec<_> = self
            .items
            .impl_defs_in_order()
            .map(|(impl_id, _)| impl_id)
            .collect();
        for impl_id in impl_ids {
            let trait_snapshot = self
                .items
                .impl_def(impl_id)
                .unwrap()
                .trait_id
                .or_else(|| {
                    self.items
                        .impl_def(impl_id)
                        .unwrap()
                        .trait_name
                        .as_deref()
                        .and_then(|name| self.resolve_trait_id(name))
                })
                .and_then(|trait_id| self.items.trait_def(trait_id))
                .cloned();
            let impl_span = self
                .source_map
                .definition_declaration_span(impl_id)
                .cloned();
            let imp = self.items.impl_def_mut(impl_id).unwrap();
            if imp.id.crate_id != self.root_crate_id {
                continue;
            }

            if let Some(trait_name) = imp.trait_name.clone() {
                let trait_def = trait_snapshot;

                if let Some(trait_def) = trait_def.as_ref() {
                    if imp.trait_id.is_none() {
                        imp.trait_id = Some(trait_def.id);
                    }

                    let self_type = match &imp.receiver_pattern {
                        HirImplReceiverPattern::Exact(ty)
                        | HirImplReceiverPattern::Constructor(ty) => ty.clone(),
                        HirImplReceiverPattern::SliceFamily { element } => {
                            Type::Slice(Box::new(element.clone()))
                        }
                    };
                    let impl_trait_id = Some(trait_def.id);
                    let impl_associated_types = imp.associated_types.clone();
                    let mut trait_generic_subst: HashMap<GenericParamId, Type> = trait_def
                        .generic_params
                        .iter()
                        .enumerate()
                        .filter_map(|(index, _)| {
                            imp.trait_arg_types.get(index).cloned().map(|actual| {
                                (
                                    GenericParamId {
                                        owner: trait_def.id,
                                        index: index as u32,
                                    },
                                    actual,
                                )
                            })
                        })
                        .collect();
                    trait_generic_subst.insert(
                        GenericParamId {
                            owner: trait_def.id,
                            index: trait_def.generic_params.len() as u32,
                        },
                        self_type.clone(),
                    );

                    for assoc in &trait_def.associated_types {
                        if !imp.associated_types.iter().any(|item| item.id == assoc.id) {
                            self.output.diagnostics.push(
                                crate::lower::ResolveError::from_optional_span(
                                    format!(
                                        "Type '{}' does not define required associated type '{}' from trait '{}'",
                                        imp.type_name, assoc.name, trait_name
                                    ),
                                    impl_span.clone(),
                                ),
                            );
                        }
                    }

                    for assoc in &imp.associated_types {
                        if !trait_def
                            .associated_types
                            .iter()
                            .any(|decl| decl.id == assoc.id)
                        {
                            let span = self
                                .source_map
                                .symbol(&crate::source_map::SourceSymbol::AssociatedType {
                                    owner: imp.id,
                                    associated: assoc.id,
                                })
                                .map(|source| source.name_span.clone())
                                .or_else(|| impl_span.clone());
                            self.output.diagnostics.push(
                                crate::lower::ResolveError::from_optional_span(
                                    format!(
                                    "Type '{}' defines unknown associated type '{}' for trait '{}'",
                                    imp.type_name, assoc.name, trait_name
                                ),
                                    span,
                                ),
                            );
                        }
                    }

                    // Check required signatures are implemented and unify types
                    for (sig_name, sig) in &trait_def.signatures {
                        if !imp.methods.contains_key(sig_name)
                            && !trait_def.methods.contains_key(sig_name)
                        {
                            // Required method not implemented and no default
                            self.output.diagnostics.push(
                                crate::lower::ResolveError::from_optional_span(
                                    format!(
                                        "Type '{}' does not implement required method '{}' from trait '{}'",
                                        imp.type_name, sig_name, trait_name
                                    ),
                                    impl_span.clone(),
                                ),
                            );
                        } else if let Some(method) = imp.methods.get_mut(sig_name) {
                            let method_span = self
                                .source_map
                                .definition_declaration_span(method.id)
                                .cloned();
                            let signature_span = method_span.clone();
                            let signature_method_generics = sig
                                .generic_params
                                .iter()
                                .filter_map(|signature_param| {
                                    method
                                        .generic_params
                                        .iter()
                                        .find(|method_param| {
                                            method_param.name == signature_param.name
                                                && method_param.kind == signature_param.kind
                                        })
                                        .map(|method_param| (signature_param.id, method_param.id))
                                })
                                .collect::<HashMap<_, _>>();
                            // Unify impl method types with trait signature (after substituting Self)
                            // First, handle curried function types: Self -> Self -> Self
                            // The trait signature params are [Self, Self] for a binary op
                            // We need to unify each parameter with the substituted type

                            if method.params.len() != sig.params.len() {
                                self.output.diagnostics.push(crate::lower::ResolveError::from_optional_span_code(format!(
                                        "Type '{}' method '{}' parameter count mismatch for trait '{}': expected {}, found {}",
                                        imp.type_name,
                                        sig_name,
                                        trait_name,
                                        sig.params.len(),
                                        method.params.len()
                                    ), signature_span.clone(), crate::diagnostic::DiagnosticCode::Type));
                                continue;
                            }

                            for (i, param) in method.params.iter_mut().enumerate() {
                                param.ty = substitute_self(
                                    &param.ty,
                                    &self_type,
                                    impl_trait_id,
                                    &imp.trait_arg_types,
                                    &impl_associated_types,
                                    &trait_generic_subst,
                                );
                                let mut expected_ty = substitute_self(
                                    &sig.params[i],
                                    &self_type,
                                    impl_trait_id,
                                    &imp.trait_arg_types,
                                    &impl_associated_types,
                                    &trait_generic_subst,
                                );
                                remap_generic_params_in_place(&mut expected_ty, &mut |param| {
                                    signature_method_generics
                                        .get(&param)
                                        .copied()
                                        .unwrap_or(param)
                                });
                                if let Err(err) = self.engine.unify(&param.ty, &expected_ty) {
                                    self.output.diagnostics.push(crate::lower::ResolveError::from_optional_span_code(format!(
                                            "Type '{}' method '{}' parameter {} type mismatch for trait '{}': {}",
                                            imp.type_name,
                                            sig_name,
                                            i,
                                            trait_name,
                                            err.render(&self.engine)
                                        ), signature_span.clone(), crate::diagnostic::DiagnosticCode::Type));
                                } else {
                                    let resolved = self.engine.resolve(&param.ty);
                                    // Update to resolved type
                                    param.ty = resolved;
                                }
                            }

                            // Unify return type
                            method.ret_type = substitute_self(
                                &method.ret_type,
                                &self_type,
                                impl_trait_id,
                                &imp.trait_arg_types,
                                &impl_associated_types,
                                &trait_generic_subst,
                            );
                            let mut expected_ret = substitute_self(
                                &sig.ret,
                                &self_type,
                                impl_trait_id,
                                &imp.trait_arg_types,
                                &impl_associated_types,
                                &trait_generic_subst,
                            );
                            remap_generic_params_in_place(&mut expected_ret, &mut |param| {
                                signature_method_generics
                                    .get(&param)
                                    .copied()
                                    .unwrap_or(param)
                            });
                            if let Err(err) = self.engine.unify(&method.ret_type, &expected_ret) {
                                self.output.diagnostics.push(crate::lower::ResolveError::from_optional_span_code(format!(
                                        "Type '{}' method '{}' return type mismatch for trait '{}': {}",
                                        imp.type_name, sig_name, trait_name, err.render(&self.engine)
                                    ), method_span, crate::diagnostic::DiagnosticCode::Type));
                            } else {
                                // Update to resolved type
                                method.ret_type = self.engine.resolve(&method.ret_type);
                            }
                        }
                    }

                    // Inject default methods from trait that aren't overridden.
                    for (method_name, default_func) in &trait_def.methods {
                        if !imp.methods.contains_key(method_name) {
                            let default_trait_name = trait_name.clone();
                            let trait_arg_types = imp.trait_arg_types.clone();
                            let func = TraitDefaultMethodInjector::new(
                                self.engine,
                                self.root_crate_id,
                                self.local_def_ids,
                                self.current_def_ids,
                                self.source_map,
                                &mut self.output.diagnostics,
                            )
                            .prepare_missing_default_method(
                                imp,
                                method_name,
                                default_func,
                                &self_type,
                                DefaultMethodInjectionContext {
                                    trait_name: &default_trait_name,
                                },
                                |func| {
                                    prepare_default_method(
                                        func,
                                        &self_type,
                                        impl_trait_id,
                                        &trait_arg_types,
                                        &impl_associated_types,
                                        &trait_generic_subst,
                                    );
                                },
                            );
                            imp.methods.insert(method_name.to_string(), func);
                        }
                    }
                } else {
                    self.output
                        .diagnostics
                        .push(crate::lower::ResolveError::from_optional_span(
                            format!("unknown trait '{}' in impl", trait_name),
                            impl_span.clone(),
                        ));
                }
            }
        }

        self.check_supertrait_obligations();

        self.validate_index_mut_pairs();

        collect_effective_trait_methods(self.items, self.root_crate_id, &mut self.output);
    }

    fn check_supertrait_obligations(&mut self) {
        let traits = self
            .items
            .trait_defs()
            .map(|(id, trait_def)| (id, trait_def.clone()))
            .collect::<HashMap<_, _>>();
        let impls = self
            .items
            .impl_defs_in_order()
            .map(|(id, imp)| (id, imp.clone()))
            .collect::<HashMap<_, _>>();
        let mut impl_ids = impls.keys().copied().collect::<Vec<_>>();
        impl_ids.sort();

        for impl_id in impl_ids {
            let Some(imp) = impls.get(&impl_id) else {
                continue;
            };
            if imp.id.crate_id != self.root_crate_id {
                continue;
            }
            let Some(trait_id) = imp.trait_id else {
                continue;
            };
            let Some(trait_def) = traits.get(&trait_id) else {
                continue;
            };
            let subject = match &imp.receiver_pattern {
                HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                    ty.clone()
                }
                HirImplReceiverPattern::SliceFamily { element } => {
                    Type::Slice(Box::new(element.clone()))
                }
            };
            let mut subst = crate::selection::generic_substitution_for_owner(
                &trait_def.generic_params,
                &imp.trait_arg_types,
            );
            let target_id =
                trait_def
                    .target
                    .as_ref()
                    .map(|target| target.id)
                    .unwrap_or(GenericParamId {
                        owner: trait_def.id,
                        index: trait_def.generic_params.len() as u32,
                    });
            subst.insert(target_id, subject);
            let selection = crate::selection::SelectionService::new(
                &traits,
                &impls,
                None,
                Some(trait_id),
                &imp.bounds,
            );
            for predicate in &trait_def.predicates {
                let crate::types::Predicate::Trait {
                    subject,
                    trait_id: required_trait,
                    args,
                } = predicate.substitute_generics(&subst);
                let bound = TraitBound {
                    trait_id: required_trait,
                    type_args: args,
                };
                if !selection.trait_bound_satisfied(&subject, &bound) {
                    let required_name = traits
                        .get(&required_trait)
                        .map(|required| required.name.as_str())
                        .unwrap_or("<unknown>");
                    let message = format!(
                        "implementation of trait '{}' for '{}' does not satisfy supertrait obligation '{}: {} ({})'",
                        trait_def.name,
                        imp.type_name,
                        self.display_type(&subject),
                        required_name,
                        bound
                            .type_args
                            .iter()
                            .map(|arg| self.display_type(arg))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                    let span = self.source_map.definition_declaration_span(imp.id).cloned();
                    self.output
                        .diagnostics
                        .push(ResolveError::from_optional_span_code(
                            message,
                            span,
                            crate::diagnostic::DiagnosticCode::Type,
                        ));
                }
            }
        }
    }

    fn validate_index_mut_pairs(&mut self) {
        let Some(protocols) = self.index_protocol_ids else {
            return;
        };

        let mut index_mut_impls = self
            .items
            .impl_defs_in_order()
            .filter(|(_, imp)| {
                imp.id.crate_id == self.root_crate_id
                    && imp.trait_id == Some(protocols.index_mut_trait)
            })
            .map(|(_, imp)| imp.clone())
            .collect::<Vec<_>>();
        index_mut_impls.sort_by_key(|imp| imp.id);

        let mut index_impls = self
            .items
            .impl_defs_in_order()
            .filter(|(_, imp)| {
                imp.id.crate_id == self.root_crate_id && imp.trait_id == Some(protocols.index_trait)
            })
            .map(|(_, imp)| imp.clone())
            .collect::<Vec<_>>();
        index_impls.sort_by_key(|imp| imp.id);

        for index_mut in index_mut_impls {
            let index_mut_span = self
                .source_map
                .definition_declaration_span(index_mut.id)
                .cloned();
            let mut matches = index_impls
                .iter()
                .filter(|index| index_impls_are_alpha_equivalent(&index_mut, index))
                .collect::<Vec<_>>();
            matches.sort_by_key(|imp| imp.id);

            let key = index_mut
                .trait_arg_types
                .first()
                .map(|ty| self.display_type(ty))
                .unwrap_or_else(|| "<missing>".to_string());
            let context = format!(
                "IndexMut implementation for {} with key {}",
                index_mut.type_name, key
            );

            let Some(index) = (match matches.as_slice() {
                [] => {
                    self.output
                        .diagnostics
                        .push(ResolveError::from_optional_span(
                            format!("{context} requires a matching Index implementation"),
                            index_mut_span.clone(),
                        ));
                    None
                }
                [index] => Some(*index),
                _matches => {
                    self.output
                        .diagnostics
                        .push(ResolveError::from_optional_span(
                            format!("{context} has multiple matching Index implementations"),
                            index_mut_span.clone(),
                        ));
                    None
                }
            }) else {
                continue;
            };

            let Some(index_output) = associated_type_for_index(&index, protocols.index_output)
            else {
                continue;
            };
            let Some(index_mut_output) =
                associated_type_for_index(&index_mut, protocols.index_mut_output)
            else {
                continue;
            };
            if !alpha_equivalent_type(
                &index_output.ty,
                index.id,
                &index_mut_output.ty,
                index_mut.id,
            ) {
                self.output
                    .diagnostics
                    .push(ResolveError::from_optional_span(
                        format!(
                            "IndexMut implementation output {} does not match Index output {}",
                            self.display_type(&index_mut_output.ty),
                            self.display_type(&index_output.ty)
                        ),
                        index_mut_span,
                    ));
            }
        }
    }
}

fn associated_type_for_index(imp: &HirImpl, id: AssocTypeId) -> Option<&HirAssociatedTypeDef> {
    imp.associated_types
        .iter()
        .find(|associated| associated.id == id)
}

fn index_impls_are_alpha_equivalent(left: &HirImpl, right: &HirImpl) -> bool {
    alpha_equivalent_receiver_pattern(
        &left.receiver_pattern,
        left.id,
        &right.receiver_pattern,
        right.id,
    ) && left.type_generics.len() == right.type_generics.len()
        && alpha_equivalent_type_lists(
            &left.trait_arg_types,
            left.id,
            &right.trait_arg_types,
            right.id,
        )
        && alpha_equivalent_bounds(&left.bounds, left.id, &right.bounds, right.id)
}

fn alpha_equivalent_receiver_pattern(
    left: &HirImplReceiverPattern,
    left_impl: DefId,
    right: &HirImplReceiverPattern,
    right_impl: DefId,
) -> bool {
    match (left, right) {
        (
            HirImplReceiverPattern::Exact(Type::Reference {
                mutable: true,
                inner: left_inner,
            }),
            HirImplReceiverPattern::Exact(Type::Reference {
                mutable: false,
                inner: right_inner,
            }),
        ) => alpha_equivalent_type(left_inner, left_impl, right_inner, right_impl),
        (HirImplReceiverPattern::Exact(left), HirImplReceiverPattern::Exact(right))
            if !matches!(left, Type::Reference { .. })
                && !matches!(right, Type::Reference { .. }) =>
        {
            alpha_equivalent_type(left, left_impl, right, right_impl)
        }
        (
            HirImplReceiverPattern::SliceFamily { element: left },
            HirImplReceiverPattern::SliceFamily { element: right },
        ) => alpha_equivalent_type(left, left_impl, right, right_impl),
        _ => false,
    }
}

fn alpha_equivalent_type_lists(
    left: &[Type],
    left_impl: DefId,
    right: &[Type],
    right_impl: DefId,
) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| alpha_equivalent_type(left, left_impl, right, right_impl))
}

fn alpha_equivalent_type(left: &Type, left_impl: DefId, right: &Type, right_impl: DefId) -> bool {
    match (left, right) {
        (Type::Generic(left), Type::Generic(right)) => {
            if left.owner == left_impl || right.owner == right_impl {
                left.owner == left_impl && right.owner == right_impl && left.index == right.index
            } else {
                left == right
            }
        }
        (Type::Slice(left), Type::Slice(right)) | (Type::Pointer(left), Type::Pointer(right)) => {
            alpha_equivalent_type(left, left_impl, right, right_impl)
        }
        (Type::Array(left, left_len), Type::Array(right, right_len)) => {
            left_len == right_len && alpha_equivalent_type(left, left_impl, right, right_impl)
        }
        (Type::Tuple(left), Type::Tuple(right)) => {
            alpha_equivalent_type_lists(left, left_impl, right, right_impl)
        }
        (
            Type::Function {
                params: left_params,
                ret: left_ret,
                safety: left_safety,
                callable_kind: left_kind,
                captures: left_captures,
            },
            Type::Function {
                params: right_params,
                ret: right_ret,
                safety: right_safety,
                callable_kind: right_kind,
                captures: right_captures,
            },
        ) => {
            left_safety == right_safety
                && left_kind == right_kind
                && left_captures.len() == right_captures.len()
                && left_captures
                    .iter()
                    .zip(right_captures)
                    .all(|(left, right)| {
                        left.kind == right.kind
                            && alpha_equivalent_type(&left.ty, left_impl, &right.ty, right_impl)
                    })
                && alpha_equivalent_type_lists(left_params, left_impl, right_params, right_impl)
                && alpha_equivalent_type(left_ret, left_impl, right_ret, right_impl)
        }
        (
            Type::Struct {
                id: left_id,
                args: left_args,
            },
            Type::Struct {
                id: right_id,
                args: right_args,
            },
        ) => {
            left_id == right_id
                && alpha_equivalent_type_lists(left_args, left_impl, right_args, right_impl)
        }
        (
            Type::Enum {
                id: left_id,
                args: left_args,
            },
            Type::Enum {
                id: right_id,
                args: right_args,
            },
        ) => {
            left_id == right_id
                && alpha_equivalent_type_lists(left_args, left_impl, right_args, right_impl)
        }
        (
            Type::Reference {
                mutable: left_mutable,
                inner: left_inner,
            },
            Type::Reference {
                mutable: right_mutable,
                inner: right_inner,
            },
        ) => {
            left_mutable == right_mutable
                && alpha_equivalent_type(left_inner, left_impl, right_inner, right_impl)
        }
        (
            Type::Projection {
                ty: left_ty,
                trait_id: left_trait,
                assoc_type: left_assoc,
                trait_args: left_args,
            },
            Type::Projection {
                ty: right_ty,
                trait_id: right_trait,
                assoc_type: right_assoc,
                trait_args: right_args,
            },
        ) => {
            left_trait == right_trait
                && left_assoc == right_assoc
                && alpha_equivalent_type(left_ty, left_impl, right_ty, right_impl)
                && alpha_equivalent_type_lists(left_args, left_impl, right_args, right_impl)
        }
        _ => left == right,
    }
}

fn alpha_equivalent_generic_param(
    left: GenericParamId,
    left_impl: DefId,
    right: GenericParamId,
    right_impl: DefId,
) -> bool {
    if left.owner == left_impl || right.owner == right_impl {
        left.owner == left_impl && right.owner == right_impl && left.index == right.index
    } else {
        left == right
    }
}

fn alpha_equivalent_bounds(
    left: &crate::hir::HirGenericBounds,
    left_impl: DefId,
    right: &crate::hir::HirGenericBounds,
    right_impl: DefId,
) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut left_entries = left.iter().collect::<Vec<_>>();
    left_entries.sort_by_key(|(param, _)| (param.owner, param.index));
    let mut right_entries = right.iter().collect::<Vec<_>>();
    right_entries.sort_by_key(|(param, _)| (param.owner, param.index));
    left_entries.into_iter().all(|(left_param, left_bounds)| {
        let Some((_, right_bounds)) = right_entries.iter().find(|(right_param, _)| {
            alpha_equivalent_generic_param(*left_param, left_impl, **right_param, right_impl)
        }) else {
            return false;
        };
        alpha_equivalent_bound_multiset(left_bounds, left_impl, right_bounds, right_impl)
    })
}

fn alpha_equivalent_bound_multiset(
    left: &[TraitBound],
    left_impl: DefId,
    right: &[TraitBound],
    right_impl: DefId,
) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut matched = vec![false; right.len()];
    left.iter().all(|left| {
        let Some(index) = right.iter().enumerate().position(|(index, right)| {
            !matched[index]
                && left.trait_id == right.trait_id
                && alpha_equivalent_type_lists(
                    &left.type_args,
                    left_impl,
                    &right.type_args,
                    right_impl,
                )
        }) else {
            return false;
        };
        matched[index] = true;
        true
    })
}

fn remap_type_generic_owner(ty: &mut Type, old_owner: DefId, new_owner: DefId) {
    ty.remap_def_ids(&mut |id| if id == old_owner { new_owner } else { id });
}

fn remap_generic_param_owner(param: &mut GenericParamId, old_owner: DefId, new_owner: DefId) {
    if param.owner == old_owner {
        param.owner = new_owner;
    }
}

fn remap_generic_bounds_owner(bounds: &mut HirGenericBounds, old_owner: DefId, new_owner: DefId) {
    let old_bounds = std::mem::take(bounds);
    for (mut param, mut trait_bounds) in old_bounds {
        remap_generic_param_owner(&mut param, old_owner, new_owner);
        for bound in &mut trait_bounds {
            for type_arg in &mut bound.type_args {
                remap_type_generic_owner(type_arg, old_owner, new_owner);
            }
        }
        bounds.entry(param).or_default().extend(trait_bounds);
    }
}

fn remap_function_generic_owner(func: &mut HirFunction, old_owner: DefId, new_owner: DefId) {
    if old_owner == new_owner {
        return;
    }

    for param in &mut func.generic_params {
        remap_generic_param_owner(&mut param.id, old_owner, new_owner);
    }
    remap_generic_bounds_owner(&mut func.generic_bounds, old_owner, new_owner);
    for param in &mut func.params {
        remap_type_generic_owner(&mut param.ty, old_owner, new_owner);
    }
    remap_type_generic_owner(&mut func.ret_type, old_owner, new_owner);
    remap_block_generic_owner(&mut func.body, old_owner, new_owner);
}

fn remap_block_generic_owner(block: &mut HirBlock, old_owner: DefId, new_owner: DefId) {
    remap_type_generic_owner(&mut block.ty, old_owner, new_owner);
    for stmt in &mut block.stmts {
        remap_stmt_generic_owner(stmt, old_owner, new_owner);
    }
}

fn remap_stmt_generic_owner(stmt: &mut HirStmt, old_owner: DefId, new_owner: DefId) {
    match stmt {
        HirStmt::Let { ty, value, .. } => {
            remap_type_generic_owner(ty, old_owner, new_owner);
            remap_expr_generic_owner(value, old_owner, new_owner);
        }
        HirStmt::Expr(expr) => remap_expr_generic_owner(expr, old_owner, new_owner),
        HirStmt::Return(value) | HirStmt::Break(value) => {
            if let Some(value) = value {
                remap_expr_generic_owner(value, old_owner, new_owner);
            }
        }
        HirStmt::Continue => {}
    }
}

fn remap_pattern_generic_owner(pattern: &mut HirPattern, old_owner: DefId, new_owner: DefId) {
    match pattern {
        HirPattern::Struct(_, _, type_args, fields) => {
            for type_arg in type_args {
                remap_type_generic_owner(type_arg, old_owner, new_owner);
            }
            for field in fields {
                remap_pattern_generic_owner(&mut field.pattern, old_owner, new_owner);
            }
        }
        HirPattern::Tuple(items) | HirPattern::Or(items) => {
            for nested in items {
                remap_pattern_generic_owner(nested, old_owner, new_owner);
            }
        }
        HirPattern::Enum(_, _, _, items) => {
            for nested in items {
                remap_pattern_generic_owner(nested, old_owner, new_owner);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

fn remap_expr_generic_owner(expr: &mut HirExpr, old_owner: DefId, new_owner: DefId) {
    remap_type_generic_owner(&mut expr.ty, old_owner, new_owner);

    match &mut expr.kind {
        HirExprKind::ArrayLiteral(elems) | HirExprKind::TupleLiteral(elems) => {
            for elem in elems {
                remap_expr_generic_owner(elem, old_owner, new_owner);
            }
        }
        HirExprKind::ArrayRepeat(value, _) => remap_expr_generic_owner(value, old_owner, new_owner),
        HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::TupleIndex(inner, _)
        | HirExprKind::UnaryOp(_, inner)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Deref(inner) => remap_expr_generic_owner(inner, old_owner, new_owner),
        HirExprKind::Cast(inner, ty) => {
            remap_expr_generic_owner(inner, old_owner, new_owner);
            remap_type_generic_owner(ty, old_owner, new_owner);
        }
        HirExprKind::BinOp(_, lhs, rhs) | HirExprKind::Assign(lhs, rhs) => {
            remap_expr_generic_owner(lhs, old_owner, new_owner);
            remap_expr_generic_owner(rhs, old_owner, new_owner);
        }
        HirExprKind::Call(func, args, _) => {
            remap_expr_generic_owner(func, old_owner, new_owner);
            for arg in args {
                remap_expr_generic_owner(arg, old_owner, new_owner);
            }
        }
        HirExprKind::MethodCall(func, _, args, _, target) => {
            remap_expr_generic_owner(func, old_owner, new_owner);
            for arg in args {
                remap_expr_generic_owner(arg, old_owner, new_owner);
            }
            if let Some(target) = target {
                target.for_each_type_mut(|ty| remap_type_generic_owner(ty, old_owner, new_owner));
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
            remap_expr_generic_owner(expr, old_owner, new_owner);
            if let Some(target) = branch_method {
                target.for_each_type_mut(|ty| remap_type_generic_owner(ty, old_owner, new_owner));
            }
            if let Some(HirCallTarget::StaticMethod(target)) = from_residual_target {
                remap_type_generic_owner(&mut target.owner_ty, old_owner, new_owner);
                target
                    .method
                    .for_each_type_mut(|ty| remap_type_generic_owner(ty, old_owner, new_owner));
            }
            remap_type_generic_owner(output_ty, old_owner, new_owner);
            remap_type_generic_owner(residual_ty, old_owner, new_owner);
            remap_type_generic_owner(return_ty, old_owner, new_owner);
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                remap_expr_generic_owner(&mut field.value, old_owner, new_owner);
            }
        }
        HirExprKind::EnumVariant(_, _, args, _) | HirExprKind::Intrinsic { args, .. } => {
            for arg in args {
                remap_expr_generic_owner(arg, old_owner, new_owner);
            }
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            remap_expr_generic_owner(condition, old_owner, new_owner);
            remap_block_generic_owner(then_branch, old_owner, new_owner);
            if let Some(else_branch) = else_branch {
                remap_block_generic_owner(else_branch, old_owner, new_owner);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            remap_expr_generic_owner(scrutinee, old_owner, new_owner);
            for arm in arms {
                remap_pattern_generic_owner(&mut arm.pattern, old_owner, new_owner);
                if let Some(guard) = &mut arm.guard {
                    remap_expr_generic_owner(guard, old_owner, new_owner);
                }
                remap_block_generic_owner(&mut arm.body, old_owner, new_owner);
            }
        }
        HirExprKind::While { condition, body } => {
            remap_expr_generic_owner(condition, old_owner, new_owner);
            remap_block_generic_owner(body, old_owner, new_owner);
        }
        HirExprKind::For { iter, body, .. } => {
            remap_expr_generic_owner(iter, old_owner, new_owner);
            remap_block_generic_owner(body, old_owner, new_owner);
        }
        HirExprKind::Loop(body) | HirExprKind::Block(body) | HirExprKind::UnsafeBlock(body) => {
            remap_block_generic_owner(body, old_owner, new_owner);
        }
        HirExprKind::Lambda {
            params,
            body,
            captures,
        } => {
            for param in params {
                remap_type_generic_owner(&mut param.ty, old_owner, new_owner);
            }
            for capture in captures {
                remap_type_generic_owner(&mut capture.ty, old_owner, new_owner);
            }
            remap_block_generic_owner(body, old_owner, new_owner);
        }
        HirExprKind::IntLiteral(_)
        | HirExprKind::FloatLiteral(_)
        | HirExprKind::BoolLiteral(_)
        | HirExprKind::StringLiteral(_)
        | HirExprKind::CharLiteral(_)
        | HirExprKind::Unit
        | HirExprKind::Var(_)
        | HirExprKind::ResolvedVar(_) => {}
    }
}

#[cfg(test)]
mod tests;
