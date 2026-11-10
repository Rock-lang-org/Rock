use super::body_type_remap::{remap_function_signature_types, remap_impl_types};
use crate::hir::{
    HirCallTarget, HirEnum, HirExtern, HirFunctionFor, HirImplFor, HirMethodCallTarget, HirPattern,
    HirPhase, HirStruct, HirTraitFor, HirTypeAlias, HirVarTarget, HirVariantFields,
};
use crate::ids::DefId;
use crate::types::GenericParamDecl;

fn remap_def_id_if_matches(id: &mut DefId, old_id: DefId, new_id: DefId) {
    if *id == old_id {
        *id = new_id;
    }
}

fn remap_type_owned_generic_owner(ty: &mut crate::types::Type, old_id: DefId, new_id: DefId) {
    use crate::types::Type;

    struct OwnedGenericOwnerFolder {
        old_id: DefId,
        new_id: DefId,
    }

    impl crate::type_services::visit::TypeFolder for OwnedGenericOwnerFolder {
        fn fold_type(&mut self, mut ty: Type) -> Type {
            if let Type::Generic(param) = &mut ty {
                if param.owner == self.old_id {
                    param.owner = self.new_id;
                }
            }
            crate::type_services::visit::fold_type_children(ty, self)
        }

        fn fold_array_len(&mut self, len: crate::types::ArrayLen) -> crate::types::ArrayLen {
            match len {
                crate::types::ArrayLen::Param(mut param) if param.owner == self.old_id => {
                    param.owner = self.new_id;
                    crate::types::ArrayLen::Param(param)
                }
                other => other,
            }
        }
    }

    crate::type_services::visit::fold_type_in_place(
        ty,
        &mut OwnedGenericOwnerFolder { old_id, new_id },
    );
}

fn remap_generic_param_decls_owned_owner(
    generic_param_decls: &mut [GenericParamDecl],
    old_id: DefId,
    new_id: DefId,
) {
    for generic_decl in generic_param_decls {
        if generic_decl.id.owner == old_id {
            generic_decl.id.owner = new_id;
        }
        if let Some(default) = &mut generic_decl.default {
            remap_type_owned_generic_owner(default, old_id, new_id);
        }
    }
}

fn remap_generic_bounds_owned_owner(
    bounds: &mut crate::hir::HirGenericBounds,
    old_id: DefId,
    new_id: DefId,
) {
    let mut original = std::mem::take(bounds);
    let mut predicates = std::mem::take(&mut original.predicates);
    for predicate in &mut predicates {
        match predicate {
            crate::types::Predicate::Trait { subject, args, .. } => {
                remap_type_owned_generic_owner(subject, old_id, new_id);
                for arg in args {
                    remap_type_owned_generic_owner(arg, old_id, new_id);
                }
            }
        }
    }
    for (mut generic_param, mut trait_bounds) in original {
        if generic_param.owner == old_id {
            generic_param.owner = new_id;
        }
        for bound in &mut trait_bounds {
            for ty in &mut bound.type_args {
                remap_type_owned_generic_owner(ty, old_id, new_id);
            }
        }
        bounds.insert(generic_param, trait_bounds);
    }
    bounds.predicates = predicates;
}

pub(super) fn remap_function_owned_type_ids<P: HirPhase>(
    function: &mut HirFunctionFor<P>,
    old_id: DefId,
) {
    let new_id = function.id;
    remap_generic_param_decls_owned_owner(&mut function.generic_params, old_id, new_id);
    remap_generic_bounds_owned_owner(&mut function.generic_bounds, old_id, new_id);
    let mut remap_type = |ty: &mut crate::types::Type| {
        remap_type_owned_generic_owner(ty, old_id, new_id);
    };
    remap_function_signature_types(function, &mut remap_type);
    remap_block_owned_type_ids(&mut function.body, old_id, new_id);
}

pub(super) fn remap_struct_owned_type_ids(strukt: &mut HirStruct, old_id: DefId) {
    let new_id = strukt.id;
    remap_generic_param_decls_owned_owner(&mut strukt.generic_params, old_id, new_id);
    for field in &mut strukt.fields {
        remap_type_owned_generic_owner(&mut field.ty, old_id, new_id);
    }
}

pub(super) fn remap_enum_owned_type_ids(enm: &mut HirEnum, old_id: DefId) {
    let new_id = enm.id;
    remap_generic_param_decls_owned_owner(&mut enm.generic_params, old_id, new_id);
    for variant in &mut enm.variants {
        match &mut variant.fields {
            HirVariantFields::Named(fields) => {
                for field in fields {
                    remap_type_owned_generic_owner(&mut field.ty, old_id, new_id);
                }
            }
            HirVariantFields::Positional(types) => {
                for ty in types {
                    remap_type_owned_generic_owner(ty, old_id, new_id);
                }
            }
            HirVariantFields::Unit => {}
        }
    }
}

pub(super) fn remap_type_alias_owned_type_ids(alias: &mut HirTypeAlias, old_id: DefId) {
    let new_id = alias.id;
    remap_generic_param_decls_owned_owner(&mut alias.generic_params, old_id, new_id);
    remap_type_owned_generic_owner(&mut alias.ty, old_id, new_id);
}

pub(super) fn remap_trait_owned_type_ids<P: HirPhase>(trait_: &mut HirTraitFor<P>, old_id: DefId) {
    let new_id = trait_.id;
    remap_generic_param_decls_owned_owner(&mut trait_.generic_params, old_id, new_id);
    if let Some(target) = &mut trait_.target {
        if target.id.owner == old_id {
            target.id.owner = new_id;
        }
    }
    let mut remap_type = |ty: &mut crate::types::Type| {
        remap_type_owned_generic_owner(ty, old_id, new_id);
    };
    for predicate in &mut trait_.predicates {
        match predicate {
            crate::types::Predicate::Trait {
                subject,
                trait_id,
                args,
            } => {
                if *trait_id == old_id {
                    *trait_id = new_id;
                }
                remap_type(subject);
                for arg in args {
                    remap_type(arg);
                }
            }
        }
    }
    for method in trait_.methods.values_mut() {
        remap_generic_param_decls_owned_owner(&mut method.generic_params, old_id, new_id);
        remap_generic_bounds_owned_owner(&mut method.generic_bounds, old_id, new_id);
        remap_function_signature_types(method, &mut remap_type);
    }
    for sig in trait_.signatures.values_mut() {
        remap_generic_param_decls_owned_owner(&mut sig.generic_params, old_id, new_id);
        remap_generic_bounds_owned_owner(&mut sig.generic_bounds, old_id, new_id);
        for param in &mut sig.params {
            remap_type(param);
        }
        remap_type(&mut sig.ret);
    }
}

pub(super) fn remap_impl_owned_type_ids<P: HirPhase>(imp: &mut HirImplFor<P>, old_id: DefId) {
    let new_id = imp.id;
    remap_generic_param_decls_owned_owner(&mut imp.type_generics, old_id, new_id);
    remap_generic_param_decls_owned_owner(&mut imp.trait_generics, old_id, new_id);
    for method in imp.methods.values_mut() {
        remap_generic_param_decls_owned_owner(&mut method.generic_params, old_id, new_id);
        remap_generic_bounds_owned_owner(&mut method.generic_bounds, old_id, new_id);
    }
    let mut remap_type = |ty: &mut crate::types::Type| {
        remap_type_owned_generic_owner(ty, old_id, new_id);
    };
    remap_impl_types(imp, &mut remap_type);
}

pub(super) fn remap_extern_owned_type_ids(ext: &mut HirExtern, old_id: DefId) {
    let new_id = ext.id;
    for param in &mut ext.params {
        remap_type_owned_generic_owner(param, old_id, new_id);
    }
    remap_type_owned_generic_owner(&mut ext.ret, old_id, new_id);
}

fn remap_block_owned_type_ids<P: HirPhase>(
    block: &mut crate::hir::HirBlockFor<P>,
    old_id: DefId,
    new_id: DefId,
) {
    for stmt in &mut block.stmts {
        remap_stmt_owned_type_ids(stmt, old_id, new_id);
    }
    remap_type_owned_generic_owner(&mut block.ty, old_id, new_id);
}

fn remap_stmt_owned_type_ids<P: HirPhase>(
    stmt: &mut crate::hir::HirStmtFor<P>,
    old_id: DefId,
    new_id: DefId,
) {
    match stmt {
        crate::hir::HirStmtFor::Let { ty, value, .. } => {
            remap_type_owned_generic_owner(ty, old_id, new_id);
            remap_expr_owned_type_ids(value, old_id, new_id);
        }
        crate::hir::HirStmtFor::Expr(expr) => remap_expr_owned_type_ids(expr, old_id, new_id),
        crate::hir::HirStmtFor::Return(expr) | crate::hir::HirStmtFor::Break(expr) => {
            if let Some(expr) = expr {
                remap_expr_owned_type_ids(expr, old_id, new_id);
            }
        }
        crate::hir::HirStmtFor::Continue => {}
    }
}

fn remap_expr_owned_type_ids<P: HirPhase>(
    expr: &mut crate::hir::HirExprFor<P>,
    old_id: DefId,
    new_id: DefId,
) {
    remap_type_owned_generic_owner(&mut expr.ty, old_id, new_id);
    match &mut expr.kind {
        crate::hir::HirExprKindFor::ArrayLiteral(elems)
        | crate::hir::HirExprKindFor::TupleLiteral(elems) => {
            for elem in elems {
                remap_expr_owned_type_ids(elem, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::ArrayRepeat(value, _) => {
            remap_expr_owned_type_ids(value, old_id, new_id);
        }
        crate::hir::HirExprKindFor::FieldAccess(inner, _, _)
        | crate::hir::HirExprKindFor::TupleIndex(inner, _)
        | crate::hir::HirExprKindFor::UnaryOp(_, inner)
        | crate::hir::HirExprKindFor::Ref(_, inner)
        | crate::hir::HirExprKindFor::Deref(inner) => {
            remap_expr_owned_type_ids(inner, old_id, new_id)
        }
        crate::hir::HirExprKindFor::BinOp(_, lhs, rhs)
        | crate::hir::HirExprKindFor::Assign(lhs, rhs) => {
            remap_expr_owned_type_ids(lhs, old_id, new_id);
            remap_expr_owned_type_ids(rhs, old_id, new_id);
        }
        crate::hir::HirExprKindFor::Call(func, args, target) => {
            if let Some(target) = target {
                remap_call_target_owned_type_ids(target, old_id, new_id);
            }
            remap_expr_owned_type_ids(func, old_id, new_id);
            for arg in args {
                remap_expr_owned_type_ids(arg, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::MethodCall(func, _, args, _, target) => {
            if let Some(target) = P::method_authority_mut(target) {
                remap_method_target_owned_type_ids(target, old_id, new_id);
            }
            remap_expr_owned_type_ids(func, old_id, new_id);
            for arg in args {
                remap_expr_owned_type_ids(arg, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::Try {
            expr,
            branch_method,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            control_flow_enum,
            break_variant,
            continue_variant,
            ..
        } => {
            if let Some(target) = P::method_authority_mut(branch_method) {
                remap_method_target_owned_type_ids(target, old_id, new_id);
            }
            if let Some(target) = P::residual_authority_mut(from_residual_target) {
                remap_call_target_owned_type_ids(target, old_id, new_id);
            }
            remap_type_owned_generic_owner(output_ty, old_id, new_id);
            remap_type_owned_generic_owner(residual_ty, old_id, new_id);
            remap_type_owned_generic_owner(return_ty, old_id, new_id);
            remap_def_id_if_matches(control_flow_enum, old_id, new_id);
            remap_def_id_if_matches(&mut break_variant.owner, old_id, new_id);
            remap_def_id_if_matches(&mut continue_variant.owner, old_id, new_id);
            remap_expr_owned_type_ids(expr, old_id, new_id);
        }
        crate::hir::HirExprKindFor::StructLiteral(_, struct_id, fields) => {
            if let Some(struct_id) = struct_id {
                remap_def_id_if_matches(struct_id, old_id, new_id);
            }
            for field in fields {
                remap_expr_owned_type_ids(&mut field.value, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::EnumVariant(_, _, args, _)
        | crate::hir::HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                remap_expr_owned_type_ids(arg, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            remap_expr_owned_type_ids(condition, old_id, new_id);
            remap_block_owned_type_ids(then_branch, old_id, new_id);
            if let Some(else_branch) = else_branch {
                remap_block_owned_type_ids(else_branch, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::Match { scrutinee, arms } => {
            remap_expr_owned_type_ids(scrutinee, old_id, new_id);
            for arm in arms {
                remap_pattern_owned_type_ids(&mut arm.pattern, old_id, new_id);
                if let Some(guard) = &mut arm.guard {
                    remap_expr_owned_type_ids(guard, old_id, new_id);
                }
                remap_block_owned_type_ids(&mut arm.body, old_id, new_id);
            }
        }
        crate::hir::HirExprKindFor::While { condition, body } => {
            remap_expr_owned_type_ids(condition, old_id, new_id);
            remap_block_owned_type_ids(body, old_id, new_id);
        }
        crate::hir::HirExprKindFor::For { iter, body, .. } => {
            remap_expr_owned_type_ids(iter, old_id, new_id);
            remap_block_owned_type_ids(body, old_id, new_id);
        }
        crate::hir::HirExprKindFor::Loop(body)
        | crate::hir::HirExprKindFor::Block(body)
        | crate::hir::HirExprKindFor::UnsafeBlock(body) => {
            remap_block_owned_type_ids(body, old_id, new_id);
        }
        crate::hir::HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => {
            for param in params {
                remap_type_owned_generic_owner(&mut param.ty, old_id, new_id);
            }
            for capture in captures {
                remap_type_owned_generic_owner(&mut capture.ty, old_id, new_id);
            }
            remap_block_owned_type_ids(body, old_id, new_id);
        }
        crate::hir::HirExprKindFor::Cast(inner, ty) => {
            remap_expr_owned_type_ids(inner, old_id, new_id);
            remap_type_owned_generic_owner(ty, old_id, new_id);
        }
        crate::hir::HirExprKindFor::IntLiteral(_) | crate::hir::HirExprKindFor::ConstParam(_)
        | crate::hir::HirExprKindFor::FloatLiteral(_)
        | crate::hir::HirExprKindFor::BoolLiteral(_)
        | crate::hir::HirExprKindFor::StringLiteral(_)
        | crate::hir::HirExprKindFor::CharLiteral(_)
        | crate::hir::HirExprKindFor::Unit
        | crate::hir::HirExprKindFor::Var(_) => {}
        crate::hir::HirExprKindFor::ResolvedVar(reference) => {
            remap_var_target_owned_type_ids(&mut reference.target, old_id, new_id);
        }
    }
}

fn remap_var_target_owned_type_ids(target: &mut HirVarTarget, old_id: DefId, new_id: DefId) {
    match target {
        HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
            remap_def_id_if_matches(id, old_id, new_id);
        }
        HirVarTarget::Instance(_) | HirVarTarget::Local(_) => {}
    }
}

fn remap_call_target_owned_type_ids(target: &mut HirCallTarget, old_id: DefId, new_id: DefId) {
    if let HirCallTarget::StaticMethod(target) = target {
        remap_type_owned_generic_owner(&mut target.owner_ty, old_id, new_id);
        remap_method_target_owned_type_ids(&mut target.method, old_id, new_id);
    }
}

fn remap_method_target_owned_type_ids(
    target: &mut HirMethodCallTarget,
    old_id: DefId,
    new_id: DefId,
) {
    target.for_each_type_mut(|ty| remap_type_owned_generic_owner(ty, old_id, new_id));
    for binding in target
        .owner_substitution
        .iter_mut()
        .chain(target.method_substitution.iter_mut())
    {
        remap_def_id_if_matches(&mut binding.param.owner, old_id, new_id);
    }
}

pub(super) fn remap_pattern_owned_type_ids(pattern: &mut HirPattern, old_id: DefId, new_id: DefId) {
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                remap_pattern_owned_type_ids(pattern, old_id, new_id);
            }
        }
        HirPattern::Struct(_, struct_id, args, fields) => {
            if let Some(struct_id) = struct_id {
                remap_def_id_if_matches(struct_id, old_id, new_id);
            }
            for arg in args {
                remap_type_owned_generic_owner(arg, old_id, new_id);
            }
            for field in fields {
                if let Some(location) = &mut field.field {
                    remap_def_id_if_matches(&mut location.owner, old_id, new_id);
                }
                remap_pattern_owned_type_ids(&mut field.pattern, old_id, new_id);
            }
        }
        HirPattern::Enum(_, _, location, patterns) => {
            if let Some(location) = location {
                remap_def_id_if_matches(&mut location.owner, old_id, new_id);
            }
            for pattern in patterns {
                remap_pattern_owned_type_ids(pattern, old_id, new_id);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}
