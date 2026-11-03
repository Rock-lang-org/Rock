use super::language_items_build::is_current_crate_def;
use crate::hir::{
    HirCallTarget, HirEnum, HirExtern, HirFunctionFor, HirImplFor, HirMethodCallTarget, HirPattern,
    HirPhase, HirSelectedMethodTarget, HirStruct, HirTraitFor, HirVarTarget, HirVariantFields,
};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::infer::ResolvedHirProgram;
use crate::types::GenericParamDecl;

pub(super) fn assert_valid_current_crate_hir_ids(hir: &ResolvedHirProgram) {
    for id in &hir.current_def_ids {
        assert_valid_product_input_def_id(*id);
    }

    for function in hir.program.functions.values() {
        if is_current_crate_def(hir, function.id) {
            assert_valid_product_function_ids(function);
        }
    }
    for strukt in hir.program.structs.values() {
        if is_current_crate_def(hir, strukt.id) {
            assert_valid_product_struct_ids(strukt);
        }
    }
    for enm in hir.program.enums.values() {
        if is_current_crate_def(hir, enm.id) {
            assert_valid_product_enum_ids(enm);
        }
    }
    for alias in hir.program.type_aliases.values() {
        if is_current_crate_def(hir, alias.id) {
            assert_valid_product_input_def_id(alias.id);
            assert_valid_product_generic_param_decls(&alias.generic_params);
            assert_valid_product_type_ids(&alias.ty);
        }
    }
    for trait_def in hir.program.traits.values() {
        if is_current_crate_def(hir, trait_def.id) {
            assert_valid_product_trait_ids(trait_def);
        }
    }
    for imp in hir.program.impls.values() {
        if is_current_crate_def(hir, imp.id) {
            assert_valid_product_impl_ids(imp);
        }
    }
    for ext in hir.program.externs.values() {
        if is_current_crate_def(hir, ext.id) {
            assert_valid_product_extern_ids(ext);
        }
    }
}

fn assert_valid_product_input_def_id(id: DefId) {
    if id.crate_id == CrateId(u32::MAX) || id.local == LocalDefId(u32::MAX) {
        panic!(
            "compiler product emission received invalid current-crate DefId {:?}",
            id
        );
    }
}

fn assert_valid_product_type_ids(ty: &crate::types::Type) {
    use crate::types::Type;

    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| match nested {
        Type::Struct { id, .. } | Type::Enum { id, .. } | Type::Constructor { id, .. } => {
            assert_valid_product_input_def_id(*id)
        }
        Type::Generic(param) => assert_valid_product_input_def_id(param.owner),
        Type::Projection {
            trait_id,
            assoc_type,
            ..
        } => {
            assert_valid_product_input_def_id(*trait_id);
            assert_valid_product_input_def_id(assoc_type.owner);
        }
        _ => {}
    });
}

fn assert_valid_product_generic_param_decls(decls: &[GenericParamDecl]) {
    for decl in decls {
        assert_valid_product_input_def_id(decl.id.owner);
        if let Some(default) = &decl.default {
            assert_valid_product_type_ids(default);
        }
    }
}

fn assert_valid_product_generic_bounds(bounds: &crate::hir::HirGenericBounds) {
    for (generic_id, trait_bounds) in bounds {
        assert_valid_product_input_def_id(generic_id.owner);
        for bound in trait_bounds {
            assert_valid_product_input_def_id(bound.trait_id);
            for ty in &bound.type_args {
                assert_valid_product_type_ids(ty);
            }
        }
    }
    assert_valid_product_predicates(&bounds.predicates);
}

fn assert_valid_product_predicates(predicates: &[crate::types::Predicate]) {
    for predicate in predicates {
        match predicate {
            crate::types::Predicate::Trait {
                subject,
                trait_id,
                args,
            } => {
                assert_valid_product_input_def_id(*trait_id);
                assert_valid_product_type_ids(subject);
                for arg in args {
                    assert_valid_product_type_ids(arg);
                }
            }
        }
    }
}

fn assert_valid_product_function_ids<P: HirPhase>(function: &HirFunctionFor<P>) {
    assert_valid_product_input_def_id(function.id);
    assert_valid_product_generic_param_decls(&function.generic_params);
    assert_valid_product_generic_bounds(&function.generic_bounds);
    for param in &function.params {
        assert_valid_product_type_ids(&param.ty);
    }
    assert_valid_product_type_ids(&function.ret_type);
    assert_valid_product_block_ids(&function.body);
}

fn assert_valid_product_function_sig_ids(sig: &crate::hir::HirFunctionSig) {
    assert_valid_product_input_def_id(sig.id);
    assert_valid_product_generic_param_decls(&sig.generic_params);
    assert_valid_product_generic_bounds(&sig.generic_bounds);
    for param in &sig.params {
        assert_valid_product_type_ids(param);
    }
    assert_valid_product_type_ids(&sig.ret);
}

fn assert_valid_product_block_ids<P: HirPhase>(block: &crate::hir::HirBlockFor<P>) {
    assert_valid_product_type_ids(&block.ty);
    for stmt in &block.stmts {
        assert_valid_product_stmt_ids(stmt);
    }
}

fn assert_valid_product_stmt_ids<P: HirPhase>(stmt: &crate::hir::HirStmtFor<P>) {
    match stmt {
        crate::hir::HirStmtFor::Let { ty, value, .. } => {
            assert_valid_product_type_ids(ty);
            assert_valid_product_expr_ids(value);
        }
        crate::hir::HirStmtFor::Expr(expr) => assert_valid_product_expr_ids(expr),
        crate::hir::HirStmtFor::Return(expr) | crate::hir::HirStmtFor::Break(expr) => {
            if let Some(expr) = expr {
                assert_valid_product_expr_ids(expr);
            }
        }
        crate::hir::HirStmtFor::Continue => {}
    }
}

fn assert_valid_product_expr_ids<P: HirPhase>(expr: &crate::hir::HirExprFor<P>) {
    assert_valid_product_type_ids(&expr.ty);
    match &expr.kind {
        crate::hir::HirExprKindFor::ArrayLiteral(elems)
        | crate::hir::HirExprKindFor::TupleLiteral(elems) => {
            for elem in elems {
                assert_valid_product_expr_ids(elem);
            }
        }
        crate::hir::HirExprKindFor::ArrayRepeat(value, _) => {
            assert_valid_product_expr_ids(value);
        }
        crate::hir::HirExprKindFor::FieldAccess(inner, _, location) => {
            assert_valid_product_expr_ids(inner);
            if let Some(location) = location {
                assert_valid_product_input_def_id(location.owner);
            }
        }
        crate::hir::HirExprKindFor::TupleIndex(inner, _)
        | crate::hir::HirExprKindFor::UnaryOp(_, inner)
        | crate::hir::HirExprKindFor::Ref(_, inner)
        | crate::hir::HirExprKindFor::Deref(inner) => assert_valid_product_expr_ids(inner),
        crate::hir::HirExprKindFor::BinOp(_, lhs, rhs)
        | crate::hir::HirExprKindFor::Assign(lhs, rhs) => {
            assert_valid_product_expr_ids(lhs);
            assert_valid_product_expr_ids(rhs);
        }
        crate::hir::HirExprKindFor::Call(func, args, target) => {
            if let Some(target) = target {
                assert_valid_product_call_target(target);
            }
            assert_valid_product_expr_ids(func);
            for arg in args {
                assert_valid_product_expr_ids(arg);
            }
        }
        crate::hir::HirExprKindFor::MethodCall(func, _, args, _, target) => {
            if let Some(target) = P::method_authority(target) {
                assert_valid_product_method_target(target);
            }
            assert_valid_product_expr_ids(func);
            for arg in args {
                assert_valid_product_expr_ids(arg);
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
            assert_valid_product_expr_ids(expr);
            if let Some(target) = P::method_authority(branch_method) {
                assert_valid_product_method_target(target);
            }
            if let Some(target) = P::residual_authority(from_residual_target) {
                assert_valid_product_call_target(target);
            }
            assert_valid_product_type_ids(output_ty);
            assert_valid_product_type_ids(residual_ty);
            assert_valid_product_type_ids(return_ty);
            assert_valid_product_input_def_id(*control_flow_enum);
            assert_valid_product_input_def_id(break_variant.owner);
            assert_valid_product_input_def_id(continue_variant.owner);
        }
        crate::hir::HirExprKindFor::StructLiteral(_, struct_id, fields) => {
            if let Some(struct_id) = struct_id {
                assert_valid_product_input_def_id(*struct_id);
            }
            for field in fields {
                if let Some(location) = &field.field {
                    assert_valid_product_input_def_id(location.owner);
                }
                assert_valid_product_expr_ids(&field.value);
            }
        }
        crate::hir::HirExprKindFor::EnumVariant(_, _, args, location) => {
            if let Some(location) = location {
                assert_valid_product_input_def_id(location.owner);
            }
            for arg in args {
                assert_valid_product_expr_ids(arg);
            }
        }
        crate::hir::HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            assert_valid_product_expr_ids(condition);
            assert_valid_product_block_ids(then_branch);
            if let Some(else_branch) = else_branch {
                assert_valid_product_block_ids(else_branch);
            }
        }
        crate::hir::HirExprKindFor::Match { scrutinee, arms } => {
            assert_valid_product_expr_ids(scrutinee);
            for arm in arms {
                assert_valid_product_pattern_ids(&arm.pattern);
                if let Some(guard) = &arm.guard {
                    assert_valid_product_expr_ids(guard);
                }
                assert_valid_product_block_ids(&arm.body);
            }
        }
        crate::hir::HirExprKindFor::While { condition, body } => {
            assert_valid_product_expr_ids(condition);
            assert_valid_product_block_ids(body);
        }
        crate::hir::HirExprKindFor::For { iter, body, .. } => {
            assert_valid_product_expr_ids(iter);
            assert_valid_product_block_ids(body);
        }
        crate::hir::HirExprKindFor::Loop(body)
        | crate::hir::HirExprKindFor::Block(body)
        | crate::hir::HirExprKindFor::UnsafeBlock(body) => {
            assert_valid_product_block_ids(body);
        }
        crate::hir::HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => {
            for param in params {
                assert_valid_product_type_ids(&param.ty);
            }
            for capture in captures {
                assert_valid_product_type_ids(&capture.ty);
            }
            assert_valid_product_block_ids(body);
        }
        crate::hir::HirExprKindFor::Cast(inner, ty) => {
            assert_valid_product_expr_ids(inner);
            assert_valid_product_type_ids(ty);
        }
        crate::hir::HirExprKindFor::ResolvedVar(reference) => match reference.target {
            HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
                assert_valid_product_input_def_id(id);
            }
            HirVarTarget::Instance(_) | HirVarTarget::Local(_) => {}
        },
        crate::hir::HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                assert_valid_product_expr_ids(arg);
            }
        }
        crate::hir::HirExprKindFor::IntLiteral(_)
        | crate::hir::HirExprKindFor::FloatLiteral(_)
        | crate::hir::HirExprKindFor::BoolLiteral(_)
        | crate::hir::HirExprKindFor::StringLiteral(_)
        | crate::hir::HirExprKindFor::CharLiteral(_)
        | crate::hir::HirExprKindFor::Unit
        | crate::hir::HirExprKindFor::Var(_) => {}
    }
}

fn assert_valid_product_call_target(target: &HirCallTarget) {
    match target {
        HirCallTarget::Function(id) | HirCallTarget::Extern(id) => {
            assert_valid_product_input_def_id(*id);
        }
        HirCallTarget::StaticMethod(target) => {
            assert_valid_product_type_ids(&target.owner_ty);
            assert_valid_product_method_target(&target.method);
        }
        HirCallTarget::Instance(_) | HirCallTarget::Local(_) | HirCallTarget::Intrinsic(_) => {}
    }
}

fn assert_valid_product_method_target(target: &HirMethodCallTarget) {
    match &target.target {
        HirSelectedMethodTarget::ImplMethod {
            impl_id,
            method_id,
            selected_trait,
        } => {
            assert_valid_product_input_def_id(*impl_id);
            assert_valid_product_input_def_id(*method_id);
            if let Some(selected_trait) = selected_trait {
                assert_valid_product_input_def_id(selected_trait.trait_id);
                assert_valid_product_input_def_id(selected_trait.member_id);
            }
        }
        HirSelectedMethodTarget::TraitMethod {
            trait_id,
            member_id,
            ..
        } => {
            assert_valid_product_input_def_id(*trait_id);
            assert_valid_product_input_def_id(*member_id);
        }
    }
    for arg in target.trait_args() {
        assert_valid_product_type_ids(arg);
    }
    for binding in target
        .owner_substitution
        .iter()
        .chain(target.method_substitution.iter())
    {
        assert_valid_product_input_def_id(binding.param.owner);
        assert_valid_product_type_ids(&binding.ty);
    }
}

fn assert_valid_product_pattern_ids(pattern: &HirPattern) {
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                assert_valid_product_pattern_ids(pattern);
            }
        }
        HirPattern::Struct(_, struct_id, args, fields) => {
            if let Some(struct_id) = struct_id {
                assert_valid_product_input_def_id(*struct_id);
            }
            for arg in args {
                assert_valid_product_type_ids(arg);
            }
            for field in fields {
                if let Some(location) = &field.field {
                    assert_valid_product_input_def_id(location.owner);
                }
                assert_valid_product_pattern_ids(&field.pattern);
            }
        }
        HirPattern::Enum(_, _, location, patterns) => {
            if let Some(location) = location {
                assert_valid_product_input_def_id(location.owner);
            }
            for pattern in patterns {
                assert_valid_product_pattern_ids(pattern);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

fn assert_valid_product_struct_ids(strukt: &HirStruct) {
    assert_valid_product_input_def_id(strukt.id);
    assert_valid_product_generic_param_decls(&strukt.generic_params);
    for field in &strukt.fields {
        assert_valid_product_type_ids(&field.ty);
    }
}

fn assert_valid_product_enum_ids(enm: &HirEnum) {
    assert_valid_product_input_def_id(enm.id);
    assert_valid_product_generic_param_decls(&enm.generic_params);
    for variant in &enm.variants {
        match &variant.fields {
            HirVariantFields::Named(fields) => {
                for field in fields {
                    assert_valid_product_type_ids(&field.ty);
                }
            }
            HirVariantFields::Positional(types) => {
                for ty in types {
                    assert_valid_product_type_ids(ty);
                }
            }
            HirVariantFields::Unit => {}
        }
    }
}

fn assert_valid_product_trait_ids<P: HirPhase>(trait_def: &HirTraitFor<P>) {
    assert_valid_product_input_def_id(trait_def.id);
    assert_valid_product_generic_param_decls(&trait_def.generic_params);
    if let Some(target) = &trait_def.target {
        assert_valid_product_generic_param_decls(std::slice::from_ref(target));
    }
    assert_valid_product_predicates(&trait_def.predicates);
    for method in trait_def.methods.values() {
        assert_valid_product_function_ids(method);
    }
    for sig in trait_def.signatures.values() {
        assert_valid_product_function_sig_ids(sig);
    }
}

fn assert_valid_product_impl_ids<P: HirPhase>(imp: &HirImplFor<P>) {
    assert_valid_product_input_def_id(imp.id);
    assert_valid_product_generic_param_decls(&imp.type_generics);
    assert_valid_product_generic_param_decls(&imp.trait_generics);
    if let Some(trait_id) = imp.trait_id {
        assert_valid_product_input_def_id(trait_id);
    }
    P::visit_impl_receiver_types(&imp.receiver_pattern, &mut assert_valid_product_type_ids);
    for ty in &imp.trait_arg_types {
        assert_valid_product_type_ids(ty);
    }
    for associated in &imp.associated_types {
        assert_valid_product_type_ids(&associated.ty);
    }
    for (param, bounds) in &imp.bounds {
        assert_valid_product_input_def_id(param.owner);
        for bound in bounds {
            assert_valid_product_input_def_id(bound.trait_id);
            for ty in &bound.type_args {
                assert_valid_product_type_ids(ty);
            }
        }
    }
    for method in imp.methods.values() {
        assert_valid_product_function_ids(method);
    }
}

fn assert_valid_product_extern_ids(ext: &HirExtern) {
    assert_valid_product_input_def_id(ext.id);
    for param in &ext.params {
        assert_valid_product_type_ids(param);
    }
    assert_valid_product_type_ids(&ext.ret);
}
