use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use crate::hir::{HirBlock, HirExpr, HirExprKind, HirGenericBounds, HirStmt};
use crate::ids::DefId;
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
            | HirExprKind::IntLiteral(_)
            | HirExprKind::ConstParam(_)
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
    // Bodies are materialized against the program as it stood when the pass
    // started; only the selected bodies are copied, then written back.
    enum Slot {
        Function(DefId),
        TraitMethod(DefId, String),
        ImplMethod(DefId, String),
    }
    // Imported declarations arrive header-only; their signatures are already
    // final, so an empty foreign body has nothing to materialize.
    let root_crate_id = hir.root_crate_id;
    let selected = |function: &crate::hir::HirFunction| {
        owner_selected(owners, ConstraintOwner::Body(function.id))
            && (function.id.crate_id == root_crate_id || !function.body.stmts.is_empty())
    };
    let mut pending = Vec::new();
    let mut function_ids = hir.functions.keys().copied().collect::<Vec<_>>();
    function_ids.sort();
    for id in function_ids {
        if selected(&hir.functions[&id]) {
            pending.push((Slot::Function(id), hir.functions[&id].clone()));
        }
    }
    let mut trait_ids = hir.traits.keys().copied().collect::<Vec<_>>();
    trait_ids.sort();
    for id in trait_ids {
        let mut methods = hir.traits[&id].methods.iter().collect::<Vec<_>>();
        methods.sort_by_key(|(_, function)| function.id);
        for (name, function) in methods {
            if selected(function) {
                pending.push((Slot::TraitMethod(id, name.clone()), function.clone()));
            }
        }
    }
    let mut impl_ids = hir.impls.keys().copied().collect::<Vec<_>>();
    impl_ids.sort();
    for id in impl_ids {
        let mut methods = hir.impls[&id].methods.iter().collect::<Vec<_>>();
        methods.sort_by_key(|(_, function)| function.id);
        for (name, function) in methods {
            if selected(function) {
                pending.push((Slot::ImplMethod(id, name.clone()), function.clone()));
            }
        }
    }

    let mut context = MethodAuthorityContext {
        language_items: hir.language_items.clone(),
        functions: &hir.functions,
        materialized_functions: HashMap::new(),
        traits: &hir.traits,
        impls: &hir.impls,
        sized_trait_id: hir
            .language_items
            .sized
            .as_ref()
            .map(|items| items.trait_id),
        bounds: HirGenericBounds::new(),
        constraint_store: &mut hir.constraint_store,
        effective_trait_methods: &hir.imported_effective_trait_methods,
        resolver: &hir.resolver,
        structs: &hir.structs,
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
    for (slot, function) in &mut pending {
        context.materialize_function(function, &mut errors);
        if let Slot::Function(id) = slot {
            context.materialized_functions.insert(*id, function.clone());
        }
    }
    let ambiguous = context.ambiguous;
    for (slot, function) in pending {
        let target = match slot {
            Slot::Function(id) => hir.functions.get_mut(&id),
            Slot::TraitMethod(id, name) => hir
                .traits
                .get_mut(&id)
                .and_then(|trait_def| trait_def.methods.get_mut(&name)),
            Slot::ImplMethod(id, name) => hir
                .impls
                .get_mut(&id)
                .and_then(|imp| imp.methods.get_mut(&name)),
        };
        if let Some(target) = target {
            *target = function;
        }
    }

    if errors.is_empty() {
        Ok(ambiguous)
    } else {
        Err(errors)
    }
}

pub(in crate::infer) fn finalize_method_instantiations(
    hir: &mut PartialHir,
) -> Result<(), Vec<ResolveError>> {
    materialize_pending(hir, true, true, None, None).map(|_| ())
}
