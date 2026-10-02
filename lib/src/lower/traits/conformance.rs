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
        HirExprKind::Open { source, body, .. } => {
            retarget_generated_default_trait_method_calls_in_expr(source, trait_id, impl_id, impl_methods);
            retarget_generated_default_trait_method_calls_in_block(body, trait_id, impl_id, impl_methods);
        }
        HirExprKind::OwnedObjectCall { owner, args, .. } => {
            retarget_generated_default_trait_method_calls_in_expr(
                owner,
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
        | HirExprKind::Cast(inner, _)
        | HirExprKind::ObjectCoercion(inner, _) => {
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
        HirExprKind::Open { source, body, .. } => {
            apply_generated_default_self_type_in_expr(source, param_ty, self_type);
            apply_generated_default_self_type_in_block(body, param_ty, self_type);
        }
        HirExprKind::OwnedObjectCall { owner, args, .. } => {
            apply_generated_default_self_type_in_expr(owner, param_ty, self_type);
            for arg in args {
                apply_generated_default_self_type_in_expr(arg, param_ty, self_type);
            }
        }
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
        | HirExprKind::Cast(inner, _)
        | HirExprKind::ObjectCoercion(inner, _) => {
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
        func.generic_bounds = HirGenericBounds::new();
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
        HirExprKind::Open { source, binding, body } => {
            resolve_generated_default_expr_types(engine, source);
            binding.visit_types_mut(&mut |ty| *ty = engine.resolve(ty));
            resolve_generated_default_block_types(engine, body);
        }
        HirExprKind::OwnedObjectCall { owner, args, call } => {
            resolve_generated_default_expr_types(engine, owner);
            for arg in args {
                resolve_generated_default_expr_types(engine, arg);
            }
            call.visit_types_mut(&mut |ty| *ty = engine.resolve(ty));
        }
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
        HirExprKind::ObjectCoercion(inner, coercion) => {
            resolve_generated_default_expr_types(engine, inner);
            coercion.visit_types_mut(&mut |ty| *ty = engine.resolve(ty));
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct IndexProtocolIds {
    index_trait: DefId,
    index_output: AssocTypeId,
    index_mut_trait: DefId,
    index_mut_output: AssocTypeId,
}

pub(crate) struct TraitConformanceContext<'a> {
    language_items: &'a crate::hir::HirLanguageItems,
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
    language_items: &'a crate::hir::HirLanguageItems,
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
            language_items: context.language_items,
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
                language_items: &lowerer.language_items,
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
                HirExprKind::Open { source, binding, body } => {
                    substitute_trait_impl_types_in_expr(source, impl_type, impl_trait_id, impl_trait_arg_types, impl_associated_types, generic_subst);
                    binding.visit_types_mut(&mut |ty| *ty = substitute_self(ty, impl_type, impl_trait_id, impl_trait_arg_types, impl_associated_types, generic_subst));
                    substitute_trait_impl_types_in_block(body, impl_type, impl_trait_id, impl_trait_arg_types, impl_associated_types, generic_subst);
                }
                HirExprKind::OwnedObjectCall { owner, args, call } => {
                    substitute_trait_impl_types_in_expr(
                        owner,
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
                    call.visit_types_mut(&mut |ty| {
                        *ty = substitute_self(
                            ty,
                            impl_type,
                            impl_trait_id,
                            impl_trait_arg_types,
                            impl_associated_types,
                            generic_subst,
                        )
                    });
                }
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
                HirExprKind::ObjectCoercion(inner, coercion) => {
                    substitute_trait_impl_types_in_expr(
                        inner,
                        impl_type,
                        impl_trait_id,
                        impl_trait_arg_types,
                        impl_associated_types,
                        generic_subst,
                    );
                    coercion.visit_types_mut(&mut |ty| {
                        *ty = substitute_self(
                            ty,
                            impl_type,
                            impl_trait_id,
                            impl_trait_arg_types,
                            impl_associated_types,
                            generic_subst,
                        )
                    });
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

        for imp in self
            .items
            .impl_defs_in_order()
            .map(|(_, imp)| imp)
            .filter(|imp| imp.id.crate_id == self.root_crate_id)
        {
            let Some(trait_id) = imp.trait_id else {
                continue;
            };
            let Some(trait_def) = self.items.trait_def(trait_id) else {
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
                if let Some(previous) = self.output.effective_trait_methods.insert(key, method_id) {
                    if previous != method_id {
                        self.output.diagnostics.push(ResolveError::non_source(format!(
                                "trait implementation {:?} has conflicting effective bodies {:?} and {:?} for member {:?}",
                                imp.id, previous, method_id, member_id
                            )));
                    }
                }
            }
        }
    }

    fn check_supertrait_obligations(&mut self) {
        let structs = self
            .items
            .structures()
            .map(|(id, definition)| (id, definition.clone()))
            .collect();
        let enums = self
            .items
            .enumerations()
            .map(|(id, definition)| (id, definition.clone()))
            .collect();
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
                self.language_items.sized.as_ref().map(|item| item.trait_id),
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
                let protocol = crate::infer::solve::object_protocol_origin(
                    &subject,
                    &bound,
                    &impls,
                    &structs,
                    &enums,
                    self.language_items,
                );
                if protocol.is_none() && !selection.trait_bound_satisfied(&subject, &bound) {
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
    if left.relaxed_sized.len() != right.relaxed_sized.len()
        || !left.relaxed_sized.iter().all(|left| {
            right
                .relaxed_sized
                .iter()
                .any(|right| alpha_equivalent_generic_param(*left, left_impl, *right, right_impl))
        })
    {
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
    let mut old_bounds = std::mem::take(bounds);
    bounds.relaxed_sized = std::mem::take(&mut old_bounds.relaxed_sized)
        .into_iter()
        .map(|mut parameter| {
            remap_generic_param_owner(&mut parameter, old_owner, new_owner);
            parameter
        })
        .collect();
    bounds.predicates = std::mem::take(&mut old_bounds.predicates);
    for predicate in &mut bounds.predicates {
        let crate::types::Predicate::Trait { subject, args, .. } = predicate;
        remap_type_generic_owner(subject, old_owner, new_owner);
        for argument in args {
            remap_type_generic_owner(argument, old_owner, new_owner);
        }
    }
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
        HirExprKind::Open { source, binding, body } => {
            remap_expr_generic_owner(source, old_owner, new_owner);
            binding.visit_types_mut(&mut |ty| remap_type_generic_owner(ty, old_owner, new_owner));
            if binding.witness.owner == old_owner { binding.witness.owner = new_owner; }
            remap_block_generic_owner(body, old_owner, new_owner);
        }
        HirExprKind::OwnedObjectCall { owner, args, call } => {
            remap_expr_generic_owner(owner, old_owner, new_owner);
            for arg in args {
                remap_expr_generic_owner(arg, old_owner, new_owner);
            }
            call.visit_types_mut(&mut |ty| remap_type_generic_owner(ty, old_owner, new_owner));
            remap_opened_dispatch_owner(&mut call.method, old_owner, new_owner);
        }
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
        HirExprKind::ObjectCoercion(inner, coercion) => {
            remap_expr_generic_owner(inner, old_owner, new_owner);
            coercion.visit_types_mut(&mut |ty| remap_type_generic_owner(ty, old_owner, new_owner));
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
                remap_opened_dispatch_owner(target, old_owner, new_owner);
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

fn remap_opened_dispatch_owner(target: &mut crate::hir::HirMethodCallTarget, old_owner: DefId, new_owner: DefId) {
    if let crate::hir::HirSelectedMethodTarget::TraitMethod { dispatch: crate::hir::HirTraitDispatchKind::Opened(witness), .. } = &mut target.target {
        if witness.owner == old_owner { witness.owner = new_owner; }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::collect::resolver::ResolverTables;
    use crate::hir::{
        HirAssociatedTypeDecl, HirAssociatedTypeDef, HirBlock, HirClosureCapture,
        HirClosureCaptureKind, HirEnum, HirExpr, HirExprKind, HirFunction, HirFunctionSig, HirImpl,
        HirImplOwner, HirImplReceiverPattern, HirMatchArm, HirMethodCallTarget, HirParam,
        HirPattern, HirStmt, HirStruct, HirStructPatternField, HirTrait,
    };
    use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
    use crate::lexer::Span;
    use crate::lower::Lowerer;
    use crate::types::ReceiverMode;
    use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, TraitBound, Type};

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn empty_trait(id: DefId, name: &str) -> HirTrait {
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id,
            name: name.to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        }
    }

    fn point_struct(id: DefId) -> HirStruct {
        HirStruct {
            id,
            name: "Point".to_string(),
            generic_params: vec![],
            fields: vec![],
        }
    }

    fn register_point(lowerer: &mut Lowerer, point_id: DefId) {
        lowerer.items.insert_structure(point_struct(point_id));
        lowerer
            .resolver
            .item_paths
            .insert("Point".to_string(), point_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(point_id, "Point".to_string());
        lowerer.current_def_ids.insert(point_id);
    }

    #[test]
    fn trait_conformance_display_context_renders_generic_projection_names() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(80);
        let generic = GenericParamDecl::type_param(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "T",
        );
        lowerer
            .resolver
            .item_names_by_id
            .insert(trait_id, "Render".to_string());
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Render".to_string(),
            generic_params: vec![generic],
            associated_types: vec![HirAssociatedTypeDecl {
                id: AssocTypeId(0),
                name: "Item".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        });

        let service = super::TraitConformanceService::new(super::TraitConformanceContext {
            language_items: &lowerer.language_items,
            items: &mut lowerer.items,
            engine: &mut lowerer.engine,
            resolver: &lowerer.resolver,
            dependency_resolvers: &lowerer.dependency_resolvers,
            root_crate_id: lowerer.root_crate_id,
            local_def_ids: &mut lowerer.local_def_ids,
            current_def_ids: &mut lowerer.current_def_ids,
            source_map: &lowerer.source_map,
            index_protocol_ids: None,
        });
        let ty = Type::Projection {
            ty: Box::new(Type::Generic(GenericParamId {
                owner: trait_id,
                index: 0,
            })),
            trait_id,
            assoc_type: AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![Type::Generic(GenericParamId {
                owner: trait_id,
                index: 0,
            })],
        };

        assert_eq!(service.display_type(&ty), "<T as Render T>::Item");
    }

    #[test]
    fn index_mut_pair_diagnostic_uses_index_mut_method_span() {
        let mut lowerer = Lowerer::new_for_test();
        let index_trait_id = def_id(40);
        let index_mut_trait_id = def_id(41);
        let index_mut_method_id = def_id(42);
        let impl_id = def_id(43);
        let point_id = def_id(44);
        let span = Span {
            file_path: PathBuf::from("cell.rk"),
            start: 12,
            end: 24,
        };

        lowerer.language_items.index = Some(crate::language_items::IndexLanguageItems {
            trait_id: index_trait_id,
            output_id: AssocTypeId(1),
            method_id: def_id(45),
        });
        lowerer.language_items.index_mut = Some(crate::language_items::IndexMutLanguageItems {
            trait_id: index_mut_trait_id,
            output_id: AssocTypeId(2),
            method_id: index_mut_method_id,
        });
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Point".to_string()),
                type_name: "Point".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: point_id,
                    args: Vec::new(),
                }),
                trait_name: Some("IndexMut".to_string()),
                trait_id: Some(index_mut_trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: vec![Type::I64],
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([(
                    "write_at".to_string(),
                    HirFunction {
                        id: index_mut_method_id,
                        name: "write_at".to_string(),
                        generic_params: Vec::new(),
                        generic_bounds: HashMap::new().into(),
                        params: Vec::new(),
                        ret_type: Type::Reference {
                            mutable: true,
                            inner: Box::new(Type::I64),
                        },
                        body: HirBlock {
                            stmts: vec![HirStmt::Expr(HirExpr {
                                kind: HirExprKind::Unit,
                                ty: Type::Unit,
                                span: span.clone(),
                            })],
                            ty: Type::Unit,
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(ReceiverMode::Move),
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();
        lowerer.source_map.insert_symbol(
            crate::source_map::SourceSymbol::Definition(impl_id),
            span.clone(),
            Some(span.clone()),
            None,
        );

        let output = {
            let mut service = super::TraitConformanceService::new(super::TraitConformanceContext {
                language_items: &lowerer.language_items,
                items: &mut lowerer.items,
                engine: &mut lowerer.engine,
                resolver: &lowerer.resolver,
                dependency_resolvers: &lowerer.dependency_resolvers,
                root_crate_id: lowerer.root_crate_id,
                local_def_ids: &mut lowerer.local_def_ids,
                current_def_ids: &mut lowerer.current_def_ids,
                source_map: &lowerer.source_map,
                index_protocol_ids: Some(super::IndexProtocolIds {
                    index_trait: index_trait_id,
                    index_output: AssocTypeId(1),
                    index_mut_trait: index_mut_trait_id,
                    index_mut_output: AssocTypeId(2),
                }),
            });
            service.validate_index_mut_pairs();
            service.finish()
        };

        let diagnostic = output
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.message
                    == "IndexMut implementation for Point with key I64 requires a matching Index implementation"
            })
            .expect("missing IndexMut pairing diagnostic");
        assert!(!diagnostic.message.contains("DefId"));
        assert!(!diagnostic.message.contains("TypeVarId"));
        assert_eq!(diagnostic.code, crate::diagnostic::DiagnosticCode::Resolve);
        let diagnostic_span = diagnostic.span().expect("diagnostic should have a span");
        assert_eq!(diagnostic_span.start, span.start);
        assert_eq!(diagnostic_span.end, span.end);
    }

    #[test]
    fn conformance_does_not_resolve_dependency_export_alias_without_import() {
        let mut lowerer = Lowerer::new_for_test();
        let point_id = def_id(30);
        let trait_id = DefId::new(CrateId(7), LocalDefId(31));
        register_point(&mut lowerer, point_id);
        lowerer
            .items
            .insert_trait_def(empty_trait(trait_id, "dep::Show"));

        let mut dep_resolver = ResolverTables::default();
        dep_resolver.item_paths.insert("Show".to_string(), trait_id);
        dep_resolver
            .item_paths
            .insert("dep::Show".to_string(), trait_id);
        dep_resolver
            .item_names_by_id
            .insert(trait_id, "dep::Show".to_string());
        dep_resolver.insert_export_alias_with_name(
            "Show".to_string(),
            "dep::Show".to_string(),
            trait_id,
        );
        lowerer
            .dependency_resolvers
            .insert("dep".to_string(), dep_resolver);
        lowerer
            .items
            .insert_impl(HirImpl {
                id: def_id(32),
                owner: HirImplOwner::Named("Point".to_string()),
                type_name: "Point".to_string(),
                type_generics: vec![],
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: point_id,
                    args: Vec::new(),
                }),
                trait_name: Some("Show".to_string()),
                trait_id: None,
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        assert_eq!(lowerer.items.impl_def(def_id(32)).unwrap().trait_id, None);
    }

    #[test]
    fn conformance_requires_resolver_id_for_trait_name() {
        let mut lowerer = Lowerer::new_for_test();
        let point_id = def_id(33);
        let trait_id = def_id(34);
        register_point(&mut lowerer, point_id);
        lowerer
            .items
            .insert_trait_def(empty_trait(trait_id, "Show"));
        lowerer
            .items
            .insert_impl(HirImpl {
                id: def_id(35),
                owner: HirImplOwner::Named("Point".to_string()),
                type_name: "Point".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Show".to_string()),
                trait_id: None,
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        assert_eq!(lowerer.items.impl_def(def_id(35)).unwrap().trait_id, None);
    }

    #[test]
    fn auto_impl_sized_ignores_local_trait_named_sized() {
        let mut lowerer = Lowerer::new_for_test();
        let sized_id = def_id(10);
        let point_id = def_id(11);
        lowerer
            .items
            .insert_trait_def(empty_trait(sized_id, "Sized"));
        register_point(&mut lowerer, point_id);

        lowerer.auto_impl_sized();

        assert!(lowerer
            .items
            .impl_defs()
            .all(|(_, imp)| imp.trait_id != Some(sized_id)));
    }

    #[test]
    fn auto_impl_sized_uses_marked_trait_id() {
        let mut lowerer = Lowerer::new_for_test();
        let sized_id = def_id(20);
        let point_id = def_id(21);
        lowerer
            .items
            .insert_trait_def(empty_trait(sized_id, "Sized"));
        lowerer.language_items.sized =
            Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
        register_point(&mut lowerer, point_id);

        lowerer.auto_impl_sized();

        assert!(lowerer
            .items
            .impl_defs()
            .any(|(_, imp)| imp.type_name == "Point" && imp.trait_id == Some(sized_id)));
    }

    #[test]
    fn auto_impl_sized_uses_marked_renamed_trait_id() {
        let mut lowerer = Lowerer::new_for_test();
        let sized_id = def_id(22);
        let point_id = def_id(23);
        lowerer
            .items
            .insert_trait_def(empty_trait(sized_id, "StaticLayout"));
        lowerer.language_items.sized =
            Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
        register_point(&mut lowerer, point_id);

        lowerer.auto_impl_sized();

        let generated = lowerer
            .items
            .impl_defs()
            .find_map(|(_, imp)| (imp.type_name == "Point").then_some(imp))
            .expect("marked trait should receive automatic implementations");
        assert_eq!(generated.trait_id, Some(sized_id));
        assert_eq!(generated.trait_name.as_deref(), Some("StaticLayout"));
    }

    #[test]
    fn auto_impl_sized_does_not_run_without_marked_bundle() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(24);
        let point_id = def_id(25);
        lowerer
            .items
            .insert_trait_def(empty_trait(trait_id, "StaticLayout"));
        register_point(&mut lowerer, point_id);

        lowerer.auto_impl_sized();

        assert!(lowerer.items.impl_defs().next().is_none());
    }

    #[test]
    fn auto_impl_sized_preserves_generic_struct_shape() {
        let mut lowerer = Lowerer::new_for_test();
        let sized_id = def_id(5);
        let struct_id = def_id(6);
        lowerer
            .items
            .insert_trait_def(empty_trait(sized_id, "Sized"));
        lowerer.language_items.sized =
            Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
        lowerer.items.insert_structure(HirStruct {
            id: struct_id,
            name: "Box".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: struct_id,
                    index: 0,
                },
                "T",
            )],
            fields: vec![],
        });
        lowerer
            .resolver
            .item_paths
            .insert("Box".to_string(), struct_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(struct_id, "Box".to_string());
        lowerer.current_def_ids.insert(struct_id);

        lowerer.auto_impl_sized();

        let impl_def = lowerer
            .items
            .impl_defs()
            .find_map(|(_, imp)| (imp.type_name == "Box").then_some(imp))
            .expect("generic Box should receive Sized");
        assert_eq!(impl_def.type_generics[0].name, "T");
        assert_eq!(impl_def.type_generics[0].id.owner, impl_def.id);
        assert_eq!(
            impl_def.receiver_pattern,
            HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(GenericParamId {
                    owner: impl_def.id,
                    index: 0,
                })],
            })
        );
    }

    #[test]
    fn auto_impl_sized_preserves_generic_enum_shape() {
        let mut lowerer = Lowerer::new_for_test();
        let sized_id = def_id(7);
        let enum_id = def_id(8);
        lowerer
            .items
            .insert_trait_def(empty_trait(sized_id, "Sized"));
        lowerer.language_items.sized =
            Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
        lowerer.items.insert_enumeration(HirEnum {
            id: enum_id,
            name: "Option".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: enum_id,
                    index: 0,
                },
                "T",
            )],
            variants: vec![],
        });
        lowerer
            .resolver
            .item_paths
            .insert("Option".to_string(), enum_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(enum_id, "Option".to_string());
        lowerer.current_def_ids.insert(enum_id);

        lowerer.auto_impl_sized();

        let impl_def = lowerer
            .items
            .impl_defs()
            .find_map(|(_, imp)| (imp.type_name == "Option").then_some(imp))
            .expect("generic Option should receive Sized");
        assert_eq!(impl_def.type_generics[0].name, "T");
        assert_eq!(impl_def.type_generics[0].id.owner, impl_def.id);
        assert_eq!(
            impl_def.receiver_pattern,
            HirImplReceiverPattern::Exact(Type::Enum {
                id: enum_id,
                args: vec![Type::Generic(GenericParamId {
                    owner: impl_def.id,
                    index: 0,
                })],
            })
        );
    }

    #[test]
    fn auto_impl_sized_generates_impls_in_owner_def_id_order() {
        let mut lowerer = Lowerer::new_for_test();
        let sized_id = def_id(5);
        let high_id = def_id(30);
        let low_id = def_id(10);
        lowerer
            .items
            .insert_trait_def(empty_trait(sized_id, "Sized"));
        lowerer.language_items.sized =
            Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
        lowerer.items.insert_structure(HirStruct {
            id: high_id,
            name: "High".to_string(),
            generic_params: vec![],
            fields: vec![],
        });
        lowerer.items.insert_enumeration(HirEnum {
            id: low_id,
            name: "Low".to_string(),
            generic_params: vec![],
            variants: vec![],
        });
        for (id, name) in [(high_id, "High"), (low_id, "Low")] {
            lowerer.resolver.item_paths.insert(name.to_string(), id);
            lowerer
                .resolver
                .item_names_by_id
                .insert(id, name.to_string());
            lowerer.current_def_ids.insert(id);
        }

        lowerer.auto_impl_sized();

        assert_eq!(
            lowerer
                .items
                .impl_defs_in_order()
                .map(|(_, imp)| imp.type_name.as_str())
                .collect::<Vec<_>>(),
            vec!["Low", "High"]
        );
    }

    #[test]
    fn auto_impl_sized_skips_foreign_structs_without_current_provenance() {
        let mut lowerer = Lowerer::new_for_test();
        let sized_id = def_id(20);
        let foreign_point_id = DefId::new(CrateId(7), LocalDefId(21));
        lowerer
            .items
            .insert_trait_def(empty_trait(sized_id, "Sized"));
        lowerer.language_items.sized =
            Some(crate::language_items::SizedLanguageItems { trait_id: sized_id });
        lowerer
            .items
            .insert_structure(point_struct(foreign_point_id));
        lowerer
            .resolver
            .item_paths
            .insert("dep::Point".to_string(), foreign_point_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(foreign_point_id, "dep::Point".to_string());

        lowerer.auto_impl_sized();

        assert!(lowerer
            .items
            .impl_defs()
            .all(|(_, imp)| !(imp.type_name == "dep::Point" && imp.trait_id == Some(sized_id))));
    }

    #[test]
    fn conformance_preserves_empty_override_without_default_injection() {
        let mut lowerer = Lowerer::new_for_test();

        let trait_id = def_id(6);
        let impl_id = def_id(7);
        let default_method_id = def_id(8);
        let override_method_id = def_id(9);

        let default_method = HirFunction {
            id: default_method_id,
            name: "print".to_string(),
            generic_params: vec![],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(GenericParamId {
                    owner: trait_id,
                    index: 0,
                }),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I32,
            body: HirBlock {
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKind::IntLiteral(42),
                    ty: Type::I32,
                    span: crate::lexer::Span::test(),
                })],
                ty: Type::I32,
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Shared),
            is_unsafe: false,
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Printable".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::from([("print".to_string(), default_method.clone())]),
            signatures: HashMap::from([(
                "print".to_string(),
                HirFunctionSig {
                    id: def_id(41),
                    name: "print".to_string(),
                    generic_params: vec![],
                    params: vec![Type::Generic(GenericParamId {
                        owner: trait_id,
                        index: 0,
                    })],
                    ret: Type::I32,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Printable".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "print".to_string(),
                    HirFunction {
                        id: override_method_id,
                        name: "print".to_string(),
                        generic_params: vec![],
                        generic_bounds: HashMap::new().into(),
                        params: vec![HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I64,
                            mutable: false,
                            is_ref: false,
                        }],
                        ret_type: Type::I32,
                        body: HirBlock {
                            stmts: vec![],
                            ty: Type::Unit,
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(ReceiverMode::Shared),
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let method = &lowerer.items.impl_def(impl_id).unwrap().methods["print"];
        assert_eq!(method.id, override_method_id);
        assert_eq!(method.params[0].ty, Type::I64);
        assert_eq!(method.ret_type, Type::I32);
        assert!(method.body.stmts.is_empty());
    }

    #[test]
    fn conformance_resolves_generic_impl_self_type_from_canonical_owner() {
        let mut lowerer = Lowerer::new_for_test();

        let trait_id = def_id(60);
        let impl_id = def_id(61);
        let default_method_id = def_id(62);
        let local_box_id = def_id(63);
        let local_enum_box_id = def_id(64);
        let dependency_box_id = DefId::new(CrateId(2), LocalDefId(10));
        let trait_self = Type::Generic(GenericParamId {
            owner: trait_id,
            index: 0,
        });

        lowerer.items.insert_structure(HirStruct {
            id: local_box_id,
            name: "Box".to_string(),
            generic_params: vec![],
            fields: vec![],
        });
        lowerer.items.insert_enumeration(HirEnum {
            id: local_enum_box_id,
            name: "Box".to_string(),
            generic_params: vec![],
            variants: vec![],
        });
        lowerer.items.insert_structure(HirStruct {
            id: dependency_box_id,
            name: "Box".to_string(),
            generic_params: vec![],
            fields: vec![],
        });
        lowerer
            .resolver
            .item_paths
            .insert("Box".to_string(), local_box_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(local_box_id, "Box".to_string());
        let mut dep_resolver = ResolverTables::default();
        dep_resolver
            .item_paths
            .insert("dep_b::Box".to_string(), dependency_box_id);
        dep_resolver
            .item_names_by_id
            .insert(dependency_box_id, "dep_b::Box".to_string());
        lowerer
            .dependency_resolvers
            .insert("dep_b".to_string(), dep_resolver);

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "CloneSelf".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::from([(
                "clone_self".to_string(),
                HirFunction {
                    id: default_method_id,
                    name: "clone_self".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: trait_self.clone(),
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: trait_self.clone(),
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::Var("self".to_string()),
                            ty: trait_self.clone(),
                            span: crate::lexer::Span::test(),
                        })],
                        ty: trait_self.clone(),
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::new(),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("dep_b::Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: vec![],
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: dependency_box_id,
                    args: Vec::new(),
                }),
                trait_name: Some("CloneSelf".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let method = &lowerer.items.impl_def(impl_id).unwrap().methods["clone_self"];
        let expected_self = Type::Struct {
            id: dependency_box_id,
            args: vec![],
        };
        assert_eq!(method.params[0].ty, expected_self);
        assert_eq!(method.ret_type, expected_self);
        assert!(
            lowerer.errors().is_empty(),
            "unexpected errors: {:?}",
            lowerer.errors()
        );
    }

    #[test]
    fn conformance_resolves_signatureless_default_method_return_type_from_body() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(50);
        let impl_id = def_id(51);
        let default_method_id = def_id(52);
        let ret_ty = lowerer.engine.fresh_type_var();

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Animal".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::from([(
                "legs".to_string(),
                HirFunction {
                    id: default_method_id,
                    name: "legs".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(GenericParamId {
                            owner: trait_id,
                            index: 0,
                        }),
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: ret_ty,
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::IntLiteral(4),
                            ty: Type::I64,
                            span: crate::lexer::Span::test(),
                        })],
                        ty: Type::I64,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::new(),
        });
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Dog".to_string()),
                type_name: "Dog".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Animal".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        assert_eq!(
            lowerer.items.impl_def(impl_id).unwrap().methods["legs"].ret_type,
            Type::I64
        );
    }

    #[test]
    fn trait_fixture_is_keyed_by_its_exact_id() {
        let expected_id = def_id(10);
        let wrong_id = def_id(11);
        let traits = HashMap::from([(
            wrong_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: wrong_id,
                name: "Debug".to_string(),
                generic_params: vec![],
                associated_types: vec![],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        )]);

        assert!(traits.get(&expected_id).is_none());
        assert_eq!(traits.get(&wrong_id).unwrap().id, wrong_id);
    }

    #[test]
    fn test_conformance_preserves_generic_borrowed_slice_self_param() {
        let mut lowerer = Lowerer::new_for_test();
        let self_ty = lowerer.engine.fresh_type_var();

        let trait_id = def_id(1);
        let impl_id = def_id(2);

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Show".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "show".to_string(),
                HirFunctionSig {
                    id: def_id(42),
                    name: "show".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::Generic(GenericParamId {
                        owner: trait_id,
                        index: 0,
                    })],
                    ret: Type::I32,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("&[T]".to_string()),
                type_name: "&[T]".to_string(),
                type_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: impl_id,
                        index: 0,
                    },
                    "T",
                )],
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Slice(Box::new(Type::Generic(GenericParamId {
                        owner: impl_id,
                        index: 0,
                    })))),
                }),
                trait_name: Some("Show".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "show".to_string(),
                    HirFunction {
                        id: def_id(3),
                        name: "show".to_string(),
                        generic_params: vec![],
                        generic_bounds: HashMap::new().into(),
                        params: vec![HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: self_ty,
                            mutable: false,
                            is_ref: false,
                        }],
                        ret_type: Type::I32,
                        body: HirBlock {
                            stmts: vec![],
                            ty: Type::I32,
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(ReceiverMode::Move),
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        assert_eq!(
            lowerer.items.impl_def(impl_id).unwrap().methods["show"].params[0].ty,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0
                })))),
            }
        );
    }

    #[test]
    fn conformance_substitutes_zero_based_declared_trait_generics() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(10);
        let impl_id = def_id(11);
        let method_id = def_id(12);
        let trait_generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 1,
        };
        let value_ty = lowerer.engine.fresh_type_var();
        let ret_ty = lowerer.engine.fresh_type_var();

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Sink".to_string(),
            generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "take".to_string(),
                HirFunctionSig {
                    id: def_id(43),
                    name: "take".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
                    params: vec![Type::Generic(trait_self), Type::Generic(trait_generic)],
                    ret: Type::Generic(trait_generic),
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: HirImplReceiverPattern::Exact(Type::I32),
                trait_name: Some("Sink".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "Bool",
                )],
                trait_arg_types: vec![Type::Bool],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "take".to_string(),
                    HirFunction {
                        id: method_id,
                        name: "take".to_string(),
                        generic_params: vec![],
                        generic_bounds: HashMap::new().into(),
                        params: vec![
                            HirParam {
                                name: "self".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                ty: Type::I32,
                                mutable: false,
                                is_ref: false,
                            },
                            HirParam {
                                name: "value".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                ty: value_ty,
                                mutable: false,
                                is_ref: false,
                            },
                        ],
                        ret_type: ret_ty,
                        body: HirBlock {
                            stmts: vec![],
                            ty: Type::Bool,
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(ReceiverMode::Move),
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let method = &lowerer.items.impl_def(impl_id).unwrap().methods["take"];
        assert_eq!(method.params[1].ty, Type::Bool);
        assert_eq!(method.ret_type, Type::Bool);
    }

    #[test]
    fn conformance_rejects_impl_method_missing_explicit_signature_arg() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(13);
        let impl_id = def_id(14);
        let method_id = def_id(15);
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 0,
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Sink".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "take".to_string(),
                HirFunctionSig {
                    id: def_id(16),
                    name: "take".to_string(),
                    generic_params: Vec::new(),
                    params: vec![
                        Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::Generic(trait_self)),
                        },
                        Type::I64,
                    ],
                    ret: Type::Bool,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: HirImplReceiverPattern::Exact(Type::I32),
                trait_name: Some("Sink".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "take".to_string(),
                    HirFunction {
                        id: method_id,
                        name: "take".to_string(),
                        generic_params: vec![],
                        generic_bounds: HashMap::new().into(),
                        params: vec![HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::Reference {
                                mutable: false,
                                inner: Box::new(Type::I32),
                            },
                            mutable: false,
                            is_ref: false,
                        }],
                        ret_type: Type::Bool,
                        body: HirBlock {
                            stmts: vec![],
                            ty: Type::Bool,
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(ReceiverMode::Shared),
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        assert!(lowerer.errors().iter().any(|error| {
            error
                .message
                .contains("method 'take' parameter count mismatch")
                && error.message.contains("expected 2")
                && error.message.contains("found 1")
        }));
    }

    #[test]
    fn conformance_rejects_impl_method_receiver_mode_mismatch() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(420);
        let impl_id = def_id(421);
        let method_id = def_id(422);
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 0,
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "DropLike".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "drop".to_string(),
                HirFunctionSig {
                    id: def_id(423),
                    name: "drop".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::Generic(trait_self)),
                    }],
                    ret: Type::Unit,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("DropLike".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "drop".to_string(),
                    HirFunction {
                        id: method_id,
                        name: "drop".to_string(),
                        generic_params: vec![],
                        generic_bounds: HashMap::new().into(),
                        params: vec![HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I32,
                            mutable: false,
                            is_ref: false,
                        }],
                        ret_type: Type::Unit,
                        body: HirBlock {
                            stmts: vec![],
                            ty: Type::Unit,
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(ReceiverMode::Move),
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        assert!(lowerer.errors().iter().any(|error| {
            error
                .message
                .contains("method 'drop' parameter 0 type mismatch")
                && error.message.contains("trait 'DropLike'")
        }));
    }

    #[test]
    fn conformance_rejects_impl_method_return_type_mismatch() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(424);
        let impl_id = def_id(425);
        let method_id = def_id(426);

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Flag".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "flag".to_string(),
                HirFunctionSig {
                    id: def_id(427),
                    name: "flag".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::I32],
                    ret: Type::Bool,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Flag".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "flag".to_string(),
                    HirFunction {
                        id: method_id,
                        name: "flag".to_string(),
                        generic_params: vec![],
                        generic_bounds: HashMap::new().into(),
                        params: vec![HirParam {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I32,
                            mutable: false,
                            is_ref: false,
                        }],
                        ret_type: Type::I64,
                        body: HirBlock {
                            stmts: vec![],
                            ty: Type::I64,
                        },
                        is_curried: false,
                        is_method: false,
                        self_receiver: None,
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        assert!(lowerer.errors().iter().any(|error| {
            error.message.contains("method 'flag' return type mismatch")
                && error.message.contains("trait 'Flag'")
        }));
    }

    #[test]
    fn conformance_does_not_recheck_dependency_impl_signatures() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(428);
        let impl_id = DefId::new(CrateId(7), LocalDefId(429));
        let method_id = DefId::new(CrateId(7), LocalDefId(430));

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Flag".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "flag".to_string(),
                HirFunctionSig {
                    id: def_id(431),
                    name: "flag".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::I32],
                    ret: Type::Bool,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Flag".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "flag".to_string(),
                    HirFunction {
                        id: method_id,
                        name: "flag".to_string(),
                        generic_params: vec![],
                        generic_bounds: HashMap::new().into(),
                        params: vec![HirParam {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I32,
                            mutable: false,
                            is_ref: false,
                        }],
                        ret_type: Type::I64,
                        body: HirBlock {
                            stmts: vec![],
                            ty: Type::I64,
                        },
                        is_curried: false,
                        is_method: false,
                        self_receiver: None,
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        assert!(lowerer.errors().is_empty());
    }

    #[test]
    fn conformance_substitutes_trait_arg_types_not_display_names() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(20);
        let impl_id = def_id(21);
        let method_id = def_id(22);
        let box_id = def_id(23);
        let trait_generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 1,
        };
        let boxed_i64 = Type::Struct {
            id: box_id,
            args: vec![Type::I64],
        };
        let value_ty = lowerer.engine.fresh_type_var();
        let ret_ty = lowerer.engine.fresh_type_var();

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Sink".to_string(),
            generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "take".to_string(),
                HirFunctionSig {
                    id: def_id(44),
                    name: "take".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
                    params: vec![Type::Generic(trait_self), Type::Generic(trait_generic)],
                    ret: Type::Generic(trait_generic),
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Sink".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "Box",
                )],
                trait_arg_types: vec![boxed_i64.clone()],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "take".to_string(),
                    HirFunction {
                        id: method_id,
                        name: "take".to_string(),
                        generic_params: vec![],
                        generic_bounds: HashMap::new().into(),
                        params: vec![
                            HirParam {
                                name: "self".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                ty: Type::I32,
                                mutable: false,
                                is_ref: false,
                            },
                            HirParam {
                                name: "value".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                ty: value_ty,
                                mutable: false,
                                is_ref: false,
                            },
                        ],
                        ret_type: ret_ty,
                        body: HirBlock {
                            stmts: vec![],
                            ty: boxed_i64.clone(),
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(ReceiverMode::Move),
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let method = &lowerer.items.impl_def(impl_id).unwrap().methods["take"];
        assert_eq!(method.params[1].ty, boxed_i64);
        assert_eq!(method.ret_type, boxed_i64);
    }

    #[test]
    fn conformance_uses_resolved_trait_id_for_projection_substitution() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(30);
        let impl_id = def_id(31);
        let method_id = def_id(32);
        let assoc_id = AssocTypeId(0);
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let ret_ty = lowerer.engine.fresh_type_var();

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Deref".to_string(),
            generic_params: vec![],
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_id,
                name: "Target".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "deref".to_string(),
                HirFunctionSig {
                    id: def_id(45),
                    name: "deref".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::Generic(trait_self)],
                    ret: Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::Projection {
                            ty: Box::new(Type::Generic(trait_self)),
                            trait_id,
                            assoc_type: AssociatedTypeKey {
                                owner: trait_id,
                                assoc_type_id: assoc_id,
                            },
                            trait_args: Vec::new(),
                        }),
                    },
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        });
        lowerer
            .resolver
            .item_paths
            .insert("Deref".to_string(), trait_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(trait_id, "Deref".to_string());

        let method = HirFunction {
            id: method_id,
            name: "deref".to_string(),
            generic_params: vec![],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::I64,
                mutable: false,
                is_ref: false,
            }],
            ret_type: ret_ty,
            body: HirBlock {
                stmts: vec![],
                ty: Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::I64),
                },
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Move),
            is_unsafe: false,
        };
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Deref".to_string()),
                trait_id: None,
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![HirAssociatedTypeDef {
                    id: assoc_id,
                    name: "Target".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: Type::I64,
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("deref".to_string(), method)]),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let imp = lowerer.items.impl_def(impl_id).unwrap();
        assert_eq!(imp.trait_id, Some(trait_id));
        assert_eq!(
            imp.methods["deref"].ret_type,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            }
        );
    }

    #[test]
    fn conformance_does_not_substitute_projection_owned_by_different_trait() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(50);
        let other_trait_id = def_id(51);
        let impl_id = def_id(52);
        let method_id = def_id(53);
        let assoc_id = AssocTypeId(0);
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let ret_ty = lowerer.engine.fresh_type_var();

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Deref".to_string(),
            generic_params: vec![],
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_id,
                name: "Target".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "deref".to_string(),
                HirFunctionSig {
                    id: def_id(54),
                    name: "deref".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::Generic(trait_self)],
                    ret: Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::Projection {
                            ty: Box::new(Type::Generic(trait_self)),
                            trait_id,
                            assoc_type: AssociatedTypeKey {
                                owner: other_trait_id,
                                assoc_type_id: assoc_id,
                            },
                            trait_args: Vec::new(),
                        }),
                    },
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Deref".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![HirAssociatedTypeDef {
                    id: assoc_id,
                    name: "Target".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: Type::I64,
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "deref".to_string(),
                    HirFunction {
                        id: method_id,
                        name: "deref".to_string(),
                        generic_params: vec![],
                        generic_bounds: HashMap::new().into(),
                        params: vec![HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I64,
                            mutable: false,
                            is_ref: false,
                        }],
                        ret_type: ret_ty,
                        body: HirBlock {
                            stmts: vec![],
                            ty: Type::Reference {
                                mutable: false,
                                inner: Box::new(Type::I64),
                            },
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(ReceiverMode::Move),
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        assert!(matches!(
            &lowerer.items.impl_def(impl_id).unwrap().methods["deref"].ret_type,
            Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::Projection { .. })
        ));
    }

    #[test]
    fn conformance_does_not_substitute_projection_with_different_trait_args() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(55);
        let impl_id = def_id(56);
        let method_id = def_id(57);
        let assoc_id = AssocTypeId(0);
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 1,
        };
        let ret_ty = lowerer.engine.fresh_type_var();

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Project".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "T",
            )],
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "project".to_string(),
                HirFunctionSig {
                    id: def_id(58),
                    name: "project".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::Generic(trait_self)],
                    ret: Type::Projection {
                        ty: Box::new(Type::Generic(trait_self)),
                        trait_id,
                        assoc_type: AssociatedTypeKey {
                            owner: trait_id,
                            assoc_type_id: assoc_id,
                        },
                        trait_args: vec![Type::Bool],
                    },
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Project".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "I64",
                )],
                trait_arg_types: vec![Type::I64],
                associated_types: vec![HirAssociatedTypeDef {
                    id: assoc_id,
                    name: "Output".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: Type::I64,
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "project".to_string(),
                    HirFunction {
                        id: method_id,
                        name: "project".to_string(),
                        generic_params: vec![],
                        generic_bounds: HashMap::new().into(),
                        params: vec![HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I64,
                            mutable: false,
                            is_ref: false,
                        }],
                        ret_type: ret_ty,
                        body: HirBlock {
                            stmts: vec![],
                            ty: Type::Projection {
                                ty: Box::new(Type::I64),
                                trait_id,
                                assoc_type: AssociatedTypeKey {
                                    owner: trait_id,
                                    assoc_type_id: assoc_id,
                                },
                                trait_args: vec![Type::Bool],
                            },
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(ReceiverMode::Move),
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        assert!(matches!(
            &lowerer.items.impl_def(impl_id).unwrap().methods["project"].ret_type,
            Type::Projection { trait_args, .. } if trait_args.as_slice() == [Type::Bool]
        ));
    }

    #[test]
    fn conformance_does_not_substitute_projection_on_non_self_generic_base() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(70);
        let impl_id = def_id(71);
        let method_id = def_id(72);
        let assoc_id = AssocTypeId(0);
        let trait_generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 1,
        };
        let non_self_projection = Type::Projection {
            ty: Box::new(Type::Generic(trait_generic)),
            trait_id,
            assoc_type: AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: assoc_id,
            },
            trait_args: vec![Type::Generic(trait_generic)],
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Project".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "T",
            )],
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::from([(
                "value".to_string(),
                HirFunction {
                    id: method_id,
                    name: "value".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![
                        HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::Generic(trait_self),
                            mutable: false,
                            is_ref: false,
                        },
                        HirParam {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: non_self_projection.clone(),
                            mutable: false,
                            is_ref: false,
                        },
                    ],
                    ret_type: non_self_projection.clone(),
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::Var("value".to_string()),
                            ty: non_self_projection.clone(),
                            span: crate::lexer::Span::test(),
                        })],
                        ty: non_self_projection.clone(),
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::new(),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Project".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "I64",
                )],
                trait_arg_types: vec![Type::I64],
                associated_types: vec![HirAssociatedTypeDef {
                    id: assoc_id,
                    name: "Output".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: Type::Bool,
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let method = &lowerer.items.impl_def(impl_id).unwrap().methods["value"];
        assert!(matches!(method.params[1].ty, Type::Projection { .. }));
        assert!(matches!(method.ret_type, Type::Projection { .. }));
    }

    #[test]
    fn conformance_substitutes_default_method_nested_struct_trait_generic_types() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(73);
        let impl_id = def_id(74);
        let method_id = def_id(75);
        let box_id = def_id(76);
        let trait_generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 1,
        };
        let boxed_trait_generic = Type::Struct {
            id: box_id,
            args: vec![Type::Generic(trait_generic)],
        };
        let boxed_i64 = Type::Struct {
            id: box_id,
            args: vec![Type::I64],
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "MakeBox".to_string(),
            generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
            associated_types: vec![],
            methods: HashMap::from([(
                "make".to_string(),
                HirFunction {
                    id: method_id,
                    name: "make".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: boxed_trait_generic.clone(),
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::Var("boxed".to_string()),
                            ty: boxed_trait_generic.clone(),
                            span: crate::lexer::Span::test(),
                        })],
                        ty: boxed_trait_generic.clone(),
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::new(),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("MakeBox".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "I64",
                )],
                trait_arg_types: vec![Type::I64],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let method = &lowerer.items.impl_def(impl_id).unwrap().methods["make"];
        assert_eq!(method.ret_type, boxed_i64);
        assert_eq!(method.body.ty, boxed_i64);
    }

    #[test]
    fn conformance_substitutes_default_method_nested_enum_trait_generic_types() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(77);
        let impl_id = def_id(78);
        let method_id = def_id(79);
        let maybe_id = def_id(80);
        let trait_generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 1,
        };
        let maybe_trait_generic = Type::Enum {
            id: maybe_id,
            args: vec![Type::Generic(trait_generic)],
        };
        let maybe_i64 = Type::Enum {
            id: maybe_id,
            args: vec![Type::I64],
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "MakeMaybe".to_string(),
            generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
            associated_types: vec![],
            methods: HashMap::from([(
                "make".to_string(),
                HirFunction {
                    id: method_id,
                    name: "make".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: maybe_trait_generic.clone(),
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::Var("maybe".to_string()),
                            ty: maybe_trait_generic.clone(),
                            span: crate::lexer::Span::test(),
                        })],
                        ty: maybe_trait_generic.clone(),
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::new(),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("MakeMaybe".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "I64",
                )],
                trait_arg_types: vec![Type::I64],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let method = &lowerer.items.impl_def(impl_id).unwrap().methods["make"];
        assert_eq!(method.ret_type, maybe_i64);
        assert_eq!(method.body.ty, maybe_i64);
    }

    #[test]
    fn conformance_substitutes_default_method_call_target_trait_args() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(81);
        let impl_id = def_id(82);
        let method_id = def_id(83);
        let target_method_id = def_id(84);
        let trait_generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 1,
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Caller".to_string(),
            generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
            associated_types: vec![],
            methods: HashMap::from([(
                "call".to_string(),
                HirFunction {
                    id: method_id,
                    name: "call".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::I64,
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::MethodCall(
                                Box::new(HirExpr {
                                    kind: HirExprKind::Var("self".to_string()),
                                    ty: Type::Generic(trait_self),
                                    span: crate::lexer::Span::test(),
                                }),
                                "target".to_string(),
                                Vec::new(),
                                Some(ReceiverMode::Shared),
                                Some(HirMethodCallTarget::impl_method(
                                    impl_id,
                                    target_method_id,
                                    Some(crate::hir::HirSelectedTraitMember {
                                        trait_id,
                                        member_id: target_method_id,
                                        trait_args: vec![Type::Generic(trait_generic)],
                                    }),
                                )),
                            ),
                            ty: Type::I64,
                            span: crate::lexer::Span::test(),
                        })],
                        ty: Type::I64,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::new(),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Caller".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "I64",
                )],
                trait_arg_types: vec![Type::I64],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let HirStmt::Expr(expr) = &lowerer.items.impl_def(impl_id).unwrap().methods["call"]
            .body
            .stmts[0]
        else {
            panic!("default body should contain method call");
        };
        let HirExprKind::MethodCall(_, _, _, _, Some(target)) = &expr.kind else {
            panic!("default body should contain method target");
        };
        assert_eq!(target.trait_args(), &[Type::I64]);
    }

    #[test]
    fn conformance_does_not_retarget_default_method_bound_non_self_receiver() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(113);
        let impl_id = def_id(114);
        let score_sig_id = def_id(115);
        let default_method_id = def_id(116);
        let impl_score_method_id = def_id(117);
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let method_generic = GenericParamId {
            owner: default_method_id,
            index: 0,
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Metric".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::from([(
                "score_other".to_string(),
                HirFunction {
                    id: default_method_id,
                    name: "score_other".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(method_generic, "T")],
                    generic_bounds: HashMap::from([(
                        method_generic,
                        vec![TraitBound {
                            trait_id,
                            type_args: vec![],
                        }],
                    )])
                    .into(),
                    params: vec![
                        HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::Generic(trait_self),
                            mutable: false,
                            is_ref: false,
                        },
                        HirParam {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(1),
                            ty: Type::Generic(method_generic),
                            mutable: false,
                            is_ref: false,
                        },
                    ],
                    ret_type: Type::I64,
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::MethodCall(
                                Box::new(HirExpr {
                                    kind: HirExprKind::Var("value".to_string()),
                                    ty: Type::Generic(method_generic),
                                    span: crate::lexer::Span::test(),
                                }),
                                "score".to_string(),
                                Vec::new(),
                                Some(ReceiverMode::Shared),
                                Some(HirMethodCallTarget::trait_method(
                                    trait_id,
                                    score_sig_id,
                                    Vec::new(),
                                    crate::hir::HirTraitDispatchKind::CurrentTrait,
                                )),
                            ),
                            ty: Type::I64,
                            span: crate::lexer::Span::test(),
                        })],
                        ty: Type::I64,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::from([(
                "score".to_string(),
                HirFunctionSig {
                    id: score_sig_id,
                    name: "score".to_string(),
                    generic_params: vec![],
                    params: vec![Type::Generic(trait_self)],
                    ret: Type::I64,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Metric".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "score".to_string(),
                    HirFunction {
                        id: impl_score_method_id,
                        name: "score".to_string(),
                        generic_params: vec![],
                        generic_bounds: HashMap::new().into(),
                        params: vec![HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::I32,
                            mutable: false,
                            is_ref: false,
                        }],
                        ret_type: Type::I64,
                        body: HirBlock {
                            stmts: vec![HirStmt::Expr(HirExpr {
                                kind: HirExprKind::IntLiteral(1),
                                ty: Type::I64,
                                span: crate::lexer::Span::test(),
                            })],
                            ty: Type::I64,
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(ReceiverMode::Shared),
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let HirStmt::Expr(expr) = &lowerer.items.impl_def(impl_id).unwrap().methods["score_other"]
            .body
            .stmts[0]
        else {
            panic!("default body should contain method call");
        };
        let HirExprKind::MethodCall(_, _, _, _, Some(target)) = &expr.kind else {
            panic!("default body should contain method target");
        };
        assert_eq!(target.impl_id(), None);
        assert_eq!(target.method_id(), Some(score_sig_id));
    }

    #[test]
    fn conformance_substitutes_default_method_match_guard_and_pattern_types() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(85);
        let impl_id = def_id(86);
        let method_id = def_id(87);
        let box_id = def_id(88);
        let trait_generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 1,
        };
        let boxed_trait_generic = Type::Struct {
            id: box_id,
            args: vec![Type::Generic(trait_generic)],
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Unwrap".to_string(),
            generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
            associated_types: vec![],
            methods: HashMap::from([(
                "unwrap".to_string(),
                HirFunction {
                    id: method_id,
                    name: "unwrap".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![
                        HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::Generic(trait_self),
                            mutable: false,
                            is_ref: false,
                        },
                        HirParam {
                            name: "boxed".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: boxed_trait_generic.clone(),
                            mutable: false,
                            is_ref: false,
                        },
                    ],
                    ret_type: Type::Generic(trait_generic),
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::Match {
                                scrutinee: Box::new(HirExpr {
                                    kind: HirExprKind::Var("boxed".to_string()),
                                    ty: boxed_trait_generic.clone(),
                                    span: crate::lexer::Span::test(),
                                }),
                                arms: vec![HirMatchArm {
                                    pattern: HirPattern::Struct(
                                        "Box".to_string(),
                                        None,
                                        vec![Type::Generic(trait_generic)],
                                        vec![HirStructPatternField {
                                            name: "value".to_string(),
                                            field: None,
                                            pattern: HirPattern::Binding {
                                                name: "value".to_string(),
                                                local_id: crate::ids::HirLocalId(0),
                                                mutable: false,
                                            },
                                        }],
                                    ),
                                    guard: Some(HirExpr {
                                        kind: HirExprKind::Var("value".to_string()),
                                        ty: Type::Generic(trait_generic),
                                        span: crate::lexer::Span::test(),
                                    }),
                                    body: HirBlock {
                                        stmts: vec![HirStmt::Expr(HirExpr {
                                            kind: HirExprKind::Var("value".to_string()),
                                            ty: Type::Generic(trait_generic),
                                            span: crate::lexer::Span::test(),
                                        })],
                                        ty: Type::Generic(trait_generic),
                                    },
                                }],
                            },
                            ty: Type::Generic(trait_generic),
                            span: crate::lexer::Span::test(),
                        })],
                        ty: Type::Generic(trait_generic),
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::new(),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Unwrap".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "I64",
                )],
                trait_arg_types: vec![Type::I64],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let HirStmt::Expr(expr) = &lowerer.items.impl_def(impl_id).unwrap().methods["unwrap"]
            .body
            .stmts[0]
        else {
            panic!("default body should contain match expression");
        };
        let HirExprKind::Match { arms, .. } = &expr.kind else {
            panic!("default body should contain match expression");
        };
        let HirPattern::Struct(_, _, type_args, _) = &arms[0].pattern else {
            panic!("match arm should contain struct pattern");
        };
        assert_eq!(type_args, &vec![Type::I64]);
        assert_eq!(arms[0].guard.as_ref().unwrap().ty, Type::I64);
    }

    #[test]
    fn conformance_substitutes_default_method_generic_bound_type_args() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(89);
        let impl_id = def_id(90);
        let method_id = def_id(91);
        let bound_trait_id = def_id(92);
        let method_generic = GenericParamId {
            owner: method_id,
            index: 0,
        };
        let trait_generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 1,
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Factory".to_string(),
            generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
            associated_types: vec![],
            methods: HashMap::from([(
                "make".to_string(),
                HirFunction {
                    id: method_id,
                    name: "make".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(method_generic, "U")],
                    generic_bounds: HashMap::from([(
                        method_generic,
                        vec![TraitBound {
                            trait_id: bound_trait_id,
                            type_args: vec![Type::Generic(trait_generic)],
                        }],
                    )])
                    .into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::Generic(trait_self),
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: Type::I64,
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::IntLiteral(1),
                            ty: Type::I64,
                            span: crate::lexer::Span::test(),
                        })],
                        ty: Type::I64,
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::new(),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Factory".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "I64",
                )],
                trait_arg_types: vec![Type::I64],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let method = &lowerer.items.impl_def(impl_id).unwrap().methods["make"];
        let remapped_method_generic = method.generic_params[0].id;
        assert_eq!(remapped_method_generic.owner, method.id);
        assert_eq!(
            method.generic_bounds[&remapped_method_generic][0].type_args,
            vec![Type::I64]
        );
    }

    #[test]
    fn conformance_default_body_generic_ids_match_trait_header_order() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(95);
        let impl_id = def_id(96);
        let method_id = def_id(97);
        let trait_generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 1,
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Identity".to_string(),
            generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
            associated_types: vec![],
            methods: HashMap::from([(
                "identity".to_string(),
                HirFunction {
                    id: method_id,
                    name: "identity".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![
                        HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::Generic(trait_self),
                            mutable: false,
                            is_ref: false,
                        },
                        HirParam {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::Generic(trait_generic),
                            mutable: false,
                            is_ref: false,
                        },
                    ],
                    ret_type: Type::Generic(trait_generic),
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::Var("value".to_string()),
                            ty: Type::Generic(trait_generic),
                            span: crate::lexer::Span::test(),
                        })],
                        ty: Type::Generic(trait_generic),
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::new(),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: HirImplReceiverPattern::Exact(Type::I32),
                trait_name: Some("Identity".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "I64",
                )],
                trait_arg_types: vec![Type::I64],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let method = &lowerer.items.impl_def(impl_id).unwrap().methods["identity"];
        assert_eq!(method.params[0].ty, Type::I32);
        assert_eq!(method.params[1].ty, Type::I64);
        assert_eq!(method.ret_type, Type::I64);
        assert_eq!(method.body.ty, Type::I64);
        let HirStmt::Expr(expr) = &method.body.stmts[0] else {
            panic!("default body should contain expression");
        };
        assert_eq!(expr.ty, Type::I64);
    }

    #[test]
    fn conformance_substitutes_default_method_lambda_capture_types() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(60);
        let impl_id = def_id(61);
        let default_method_id = def_id(62);
        let trait_generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 1,
        };
        let self_ty = Type::Generic(trait_self);
        let capture_ty = Type::Generic(trait_generic);
        let lambda_ty = Type::function(Vec::new(), capture_ty.clone());

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Factory".to_string(),
            generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
            associated_types: vec![],
            methods: HashMap::from([(
                "make".to_string(),
                HirFunction {
                    id: default_method_id,
                    name: "make".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: self_ty.clone(),
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: lambda_ty.clone(),
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::Lambda {
                                params: Vec::new(),
                                body: HirBlock {
                                    stmts: Vec::new(),
                                    ty: capture_ty.clone(),
                                },
                                captures: vec![HirClosureCapture {
                                    name: "captured".to_string(),
                                    local_id: crate::ids::HirLocalId(0),
                                    kind: HirClosureCaptureKind::Move,
                                    mutable: false,
                                    ty: capture_ty.clone(),
                                }],
                            },
                            ty: lambda_ty.clone(),
                            span: crate::lexer::Span::test(),
                        })],
                        ty: lambda_ty.clone(),
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::new(),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Factory".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "I64",
                )],
                trait_arg_types: vec![Type::I64],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let HirStmt::Expr(expr) = &lowerer.items.impl_def(impl_id).unwrap().methods["make"]
            .body
            .stmts[0]
        else {
            panic!("default method body should contain lambda expression");
        };
        let HirExprKind::Lambda { captures, .. } = &expr.kind else {
            panic!("default method body should contain lambda expression");
        };
        assert_eq!(captures[0].ty, Type::I64);
    }

    #[test]
    fn conformance_substitutes_default_method_lambda_projection_carriers() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(100);
        let impl_id = def_id(101);
        let default_method_id = def_id(102);
        let box_id = def_id(103);
        let trait_generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 1,
        };
        let self_ty = Type::Generic(trait_self);
        let projection_ty = Type::Projection {
            ty: Box::new(self_ty.clone()),
            trait_id,
            assoc_type: AssociatedTypeKey {
                assoc_type_id: AssocTypeId(0),
                owner: trait_id,
            },
            trait_args: vec![Type::Generic(trait_generic)],
        };
        let impl_output_ty = Type::Struct {
            id: box_id,
            args: vec![Type::I64],
        };
        let lambda_ty = Type::function(vec![projection_ty.clone()], projection_ty.clone());

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Factory".to_string(),
            generic_params: vec![GenericParamDecl::type_param(trait_generic, "T")],
            associated_types: vec![HirAssociatedTypeDecl {
                id: AssocTypeId(0),
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::from([(
                "make".to_string(),
                HirFunction {
                    id: default_method_id,
                    name: "make".to_string(),
                    generic_params: vec![],
                    generic_bounds: HashMap::new().into(),
                    params: vec![HirParam {
                        name: "self".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: self_ty.clone(),
                        mutable: false,
                        is_ref: false,
                    }],
                    ret_type: lambda_ty.clone(),
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::Lambda {
                                params: vec![HirParam {
                                    name: "value".to_string(),
                                    local_id: crate::ids::HirLocalId(0),
                                    ty: projection_ty.clone(),
                                    mutable: false,
                                    is_ref: false,
                                }],
                                body: HirBlock {
                                    stmts: Vec::new(),
                                    ty: projection_ty.clone(),
                                },
                                captures: vec![HirClosureCapture {
                                    name: "captured".to_string(),
                                    local_id: crate::ids::HirLocalId(0),
                                    kind: HirClosureCaptureKind::Move,
                                    mutable: false,
                                    ty: projection_ty.clone(),
                                }],
                            },
                            ty: lambda_ty.clone(),
                            span: crate::lexer::Span::test(),
                        })],
                        ty: lambda_ty.clone(),
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::new(),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I32".to_string()),
                type_name: "I32".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Factory".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "I64",
                )],
                trait_arg_types: vec![Type::I64],
                associated_types: vec![HirAssociatedTypeDef {
                    id: AssocTypeId(0),
                    name: "Output".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: impl_output_ty.clone(),
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let method = &lowerer.items.impl_def(impl_id).unwrap().methods["make"];
        let expected_lambda_ty =
            Type::function(vec![impl_output_ty.clone()], impl_output_ty.clone());
        assert_eq!(method.ret_type, expected_lambda_ty);

        let HirStmt::Expr(expr) = &method.body.stmts[0] else {
            panic!("default method body should contain lambda expression");
        };
        assert_eq!(expr.ty, expected_lambda_ty);

        let HirExprKind::Lambda {
            params,
            body,
            captures,
        } = &expr.kind
        else {
            panic!("default method body should contain lambda expression");
        };
        assert_eq!(params[0].ty, impl_output_ty);
        assert_eq!(body.ty, impl_output_ty);
        assert_eq!(captures[0].ty, impl_output_ty);
    }

    #[test]
    fn generated_default_method_is_attached_to_exact_impl_owner_and_remaps_generics() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(110);
        let impl_id = def_id(111);
        let default_method_id = def_id(112);
        let bound_trait_id = def_id(114);
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let default_method_generic = GenericParamId {
            owner: default_method_id,
            index: 0,
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Factory".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::from([(
                "id".to_string(),
                HirFunction {
                    id: default_method_id,
                    name: "id".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(default_method_generic, "U")],
                    generic_bounds: HashMap::from([(
                        default_method_generic,
                        vec![TraitBound {
                            trait_id: bound_trait_id,
                            type_args: vec![Type::Generic(default_method_generic)],
                        }],
                    )])
                    .into(),
                    params: vec![
                        HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::Generic(trait_self),
                            mutable: false,
                            is_ref: false,
                        },
                        HirParam {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: Type::Generic(default_method_generic),
                            mutable: false,
                            is_ref: false,
                        },
                    ],
                    ret_type: Type::Generic(default_method_generic),
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::Var("value".to_string()),
                            ty: Type::Generic(default_method_generic),
                            span: crate::lexer::Span::test(),
                        })],
                        ty: Type::Generic(default_method_generic),
                    },
                    is_curried: false,
                    is_method: true,
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
            signatures: HashMap::new(),
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Factory".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        let method = &lowerer.items.impl_def(impl_id).unwrap().methods["id"];
        assert_ne!(method.id, default_method_id);
        assert!(lowerer.current_def_ids.contains(&method.id));
        let remapped_method_generic = GenericParamId {
            owner: method.id,
            index: 0,
        };
        assert_eq!(method.generic_params[0].id, remapped_method_generic);
        assert!(method.generic_bounds.contains_key(&remapped_method_generic));
        assert!(!method.generic_bounds.contains_key(&default_method_generic));
        assert_eq!(method.params[1].ty, Type::Generic(remapped_method_generic));
        assert_eq!(method.ret_type, Type::Generic(remapped_method_generic));
        assert_eq!(method.body.ty, Type::Generic(remapped_method_generic));
        assert_eq!(
            method.generic_bounds[&remapped_method_generic][0].type_args,
            vec![Type::Generic(remapped_method_generic)]
        );
        let HirStmt::Expr(expr) = &method.body.stmts[0] else {
            panic!("default body should contain expression");
        };
        assert_eq!(expr.ty, Type::Generic(remapped_method_generic));
    }

    #[test]
    fn trait_conformance_service_uses_explicit_deps_for_selection_and_required_items() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(130);
        let impl_id = def_id(131);
        let assoc_id = AssocTypeId(0);
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 0,
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "module::Complete".to_string(),
            generic_params: vec![],
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "finish".to_string(),
                HirFunctionSig {
                    id: def_id(132),
                    name: "finish".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::Generic(trait_self)],
                    ret: Type::I64,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        });
        lowerer.resolver.insert_import_alias_with_name(
            "Complete".to_string(),
            "module::Complete".to_string(),
            trait_id,
        );
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Complete".to_string()),
                trait_id: None,
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        let output = {
            let mut service = super::TraitConformanceService::new(super::TraitConformanceContext {
                language_items: &lowerer.language_items,
                items: &mut lowerer.items,
                engine: &mut lowerer.engine,
                resolver: &lowerer.resolver,
                dependency_resolvers: &lowerer.dependency_resolvers,
                root_crate_id: lowerer.root_crate_id,
                local_def_ids: &mut lowerer.local_def_ids,
                current_def_ids: &mut lowerer.current_def_ids,
                source_map: &lowerer.source_map,
                index_protocol_ids: None,
            });
            service.check_trait_conformance();
            service.finish()
        };

        assert_eq!(
            lowerer.items.impl_def(impl_id).unwrap().trait_id,
            Some(trait_id)
        );
        assert!(output
            .diagnostics
            .iter()
            .any(|error| error.message.contains("required associated type 'Output'")));
        assert!(output
            .diagnostics
            .iter()
            .any(|error| error.message.contains("required method 'finish'")));
    }

    #[test]
    fn lowerer_conformance_delegate_reports_required_items() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(140);
        let impl_id = def_id(141);
        let trait_self = GenericParamId {
            owner: trait_id,
            index: 0,
        };

        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Complete".to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "finish".to_string(),
                HirFunctionSig {
                    id: def_id(142),
                    name: "finish".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::Generic(trait_self)],
                    ret: Type::I64,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        });
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("I64".to_string()),
                type_name: "I64".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![].into(),
                trait_name: Some("Complete".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        lowerer.check_trait_conformance();

        assert!(lowerer
            .errors()
            .iter()
            .any(|error| error.message.contains("required method 'finish'")));
    }

    #[test]
    fn conformance_rejects_missing_supertrait_implementation() {
        let mut lowerer = Lowerer::new_for_test();
        let parent_id = def_id(700);
        let child_id = def_id(701);
        let point_id = def_id(702);
        let impl_id = def_id(703);
        register_point(&mut lowerer, point_id);
        lowerer
            .items
            .insert_trait_def(empty_trait(parent_id, "Parent"));
        lowerer.items.insert_trait_def(HirTrait {
            id: child_id,
            name: "Child".to_string(),
            generic_params: Vec::new(),
            target: None,
            predicates: vec![crate::types::Predicate::Trait {
                subject: Type::Generic(GenericParamId {
                    owner: child_id,
                    index: 0,
                }),
                trait_id: parent_id,
                args: Vec::new(),
            }],
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        });
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Point".to_string()),
                type_name: "Point".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: point_id,
                    args: Vec::new(),
                }),
                trait_name: Some("Child".to_string()),
                trait_id: Some(child_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: crate::hir::HirGenericBounds::new(),
                methods: HashMap::new(),
            })
            .unwrap();
        lowerer.current_def_ids.insert(impl_id);

        lowerer.check_trait_conformance();

        assert!(
            lowerer.errors().iter().any(|error| {
                error
                    .message
                    .contains("does not satisfy supertrait obligation")
                    && error.message.contains("Parent")
            }),
            "unexpected diagnostics: {:?}",
            lowerer.errors()
        );
    }
}
