use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use crate::hir::{HirBlock, HirExpr, HirExprKind, HirGenericBounds, HirStmt};
use crate::infer::{ConstraintOwner, PartialHir};
use crate::lower::ResolveError;
use crate::types::Type;

use super::status::owner_selected;
use super::type_vars::collect_type_vars;
use super::{AuthoritySiteId, MethodAuthorityContext};

pub(super) fn collect_unresolved_authority_vars(
    block: &HirBlock,
    engine: &crate::infer::InferenceEngine,
    output: &mut HashSet<crate::ids::TypeVarId>,
) {
    fn expr(
        node: &HirExpr,
        engine: &crate::infer::InferenceEngine,
        output: &mut HashSet<crate::ids::TypeVarId>,
    ) {
        match &node.kind {
            HirExprKind::Call(callee, args, target) => {
                if target.is_none() {
                    if let HirExprKind::FieldAccess(receiver, _, _) = &callee.kind {
                        collect_type_vars(&engine.resolve(&receiver.ty), output);
                        // Overload selection can depend on arguments as well as
                        // the receiver; keep those variables linked to callers.
                        for arg in args {
                            collect_type_vars(&engine.resolve(&arg.ty), output);
                        }
                    }
                }
                if !matches!(callee.kind, HirExprKind::FieldAccess(..))
                    && matches!(callee.ty, Type::TypeVar(_))
                {
                    collect_type_vars(&engine.resolve(&callee.ty), output);
                    collect_type_vars(&engine.resolve(&node.ty), output);
                }
                expr(callee, engine, output);
                for arg in args {
                    expr(arg, engine, output);
                }
            }
            HirExprKind::Try {
                expr: operand,
                branch_method,
                from_residual_target,
                return_ty,
                ..
            } => {
                if branch_method.is_none() {
                    collect_type_vars(&engine.resolve(&operand.ty), output);
                }
                if from_residual_target.is_none() {
                    collect_type_vars(&engine.resolve(return_ty), output);
                }
                expr(operand, engine, output);
            }
            HirExprKind::MethodCall(receiver, _, args, _, _) => {
                expr(receiver, engine, output);
                for arg in args {
                    expr(arg, engine, output);
                }
            }
            HirExprKind::FieldAccess(inner, _, _)
            | HirExprKind::Deref(inner)
            | HirExprKind::Ref(_, inner)
            | HirExprKind::Cast(inner, _)
            | HirExprKind::TupleIndex(inner, _)
            | HirExprKind::UnaryOp(_, inner)
            | HirExprKind::ArrayRepeat(inner, _) => expr(inner, engine, output),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                expr(condition, engine, output);
                collect_unresolved_authority_vars(then_branch, engine, output);
                if let Some(else_branch) = else_branch {
                    collect_unresolved_authority_vars(else_branch, engine, output);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                expr(scrutinee, engine, output);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        expr(guard, engine, output);
                    }
                    collect_unresolved_authority_vars(&arm.body, engine, output);
                }
            }
            HirExprKind::While { condition, body } => {
                expr(condition, engine, output);
                collect_unresolved_authority_vars(body, engine, output);
            }
            HirExprKind::For { iter, body, .. } => {
                expr(iter, engine, output);
                collect_unresolved_authority_vars(body, engine, output);
            }
            HirExprKind::Block(body)
            | HirExprKind::Loop(body)
            | HirExprKind::UnsafeBlock(body)
            | HirExprKind::Lambda { body, .. } => {
                collect_unresolved_authority_vars(body, engine, output)
            }
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                expr(left, engine, output);
                expr(right, engine, output);
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    expr(arg, engine, output);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    expr(&field.value, engine, output);
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

    for statement in &block.stmts {
        match statement {
            HirStmt::Let { value, .. }
            | HirStmt::Expr(value)
            | HirStmt::Return(Some(value))
            | HirStmt::Break(Some(value)) => expr(value, engine, output),
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
        }
    }
}

pub(super) fn materialize_pending(
    hir: &mut PartialHir,
    strict: bool,
    try_strict: bool,
    owners: Option<&HashSet<ConstraintOwner>>,
    selected_site: Option<AuthoritySiteId>,
) -> Result<bool, Vec<ResolveError>> {
    let struct_ids = hir
        .structs
        .values()
        .map(|structure| (structure.id, structure.clone()))
        .collect();
    let traits = hir
        .traits
        .values()
        .map(|trait_def| (trait_def.id, trait_def.clone()))
        .collect();
    let impls = hir
        .impls
        .iter()
        .map(|(&id, impl_def)| (id, impl_def.clone()))
        .collect();
    let functions = hir.functions.clone();
    let mut context = MethodAuthorityContext {
        language_items: hir.language_items.clone(),
        functions,
        traits: &traits,
        impls: &impls,
        sized_trait_id: hir
            .language_items
            .sized
            .as_ref()
            .map(|items| items.trait_id),
        bounds: HirGenericBounds::new(),
        constraint_store: &mut hir.constraint_store,
        effective_trait_methods: &hir.imported_effective_trait_methods,
        resolver: &hir.resolver,
        structs: &struct_ids,
        engine: RefCell::new(&mut hir.engine),
        try_protocol: hir.language_items.try_protocol.clone(),
        unsafe_context: Cell::new(false),
        mutable_locals: HashSet::new(),
        local_bindings: HashMap::new(),
        next_local_id: 0,
        ambiguous: false,
        selected_site,
        strict,
        try_strict,
    };

    let mut errors = Vec::new();
    let mut function_ids = hir.functions.keys().copied().collect::<Vec<_>>();
    function_ids.sort();
    for id in function_ids {
        if !owner_selected(owners, ConstraintOwner::Body(id)) {
            continue;
        }
        if let Some(function) = hir.functions.get_mut(&id) {
            context.materialize_function(function, &mut errors);
            context.functions.insert(id, function.clone());
        }
    }
    let mut trait_ids = hir.traits.keys().copied().collect::<Vec<_>>();
    trait_ids.sort();
    for id in trait_ids {
        if let Some(trait_def) = hir.traits.get_mut(&id) {
            let mut methods = trait_def
                .methods
                .iter()
                .map(|(name, function)| (function.id, name.clone()))
                .collect::<Vec<_>>();
            methods.sort_by_key(|(method_id, _)| *method_id);
            for (_, name) in methods {
                if !owner_selected(owners, ConstraintOwner::Body(trait_def.methods[&name].id)) {
                    continue;
                }
                if let Some(function) = trait_def.methods.get_mut(&name) {
                    context.materialize_function(function, &mut errors);
                }
            }
        }
    }
    let mut impl_ids = hir.impls.keys().copied().collect::<Vec<_>>();
    impl_ids.sort();
    for id in impl_ids {
        if let Some(imp) = hir.impls.get_mut(&id) {
            let mut methods = imp
                .methods
                .iter()
                .map(|(name, function)| (function.id, name.clone()))
                .collect::<Vec<_>>();
            methods.sort_by_key(|(method_id, _)| *method_id);
            for (_, name) in methods {
                if !owner_selected(owners, ConstraintOwner::Body(imp.methods[&name].id)) {
                    continue;
                }
                if let Some(function) = imp.methods.get_mut(&name) {
                    context.materialize_function(function, &mut errors);
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(context.ambiguous)
    } else {
        Err(errors)
    }
}

pub(in crate::infer) fn finalize_method_instantiations(
    hir: &mut PartialHir,
) -> Result<(), Vec<ResolveError>> {
    materialize_pending(hir, true, true, None, None).map(|_| ())
}
