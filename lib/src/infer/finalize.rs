//! Type finalization: apply inference engine substitutions to all HIR nodes.

use crate::hir::*;
use crate::ids::DefId;
use crate::lower::ResolveError;
use crate::type_services::facts::TypeFacts;
use crate::types::{GenericParamId, Type};

use super::{InferenceEngine, InferenceError, PartialHir, UnifyError};

/// Finalization context carrying the engine and error list.
pub(super) struct FinalizeCtx<'a> {
    engine: &'a InferenceEngine,
    method_generic_params:
        std::collections::HashMap<DefId, std::collections::HashSet<GenericParamId>>,
    errors: Vec<InferenceError>,
}

impl<'a> FinalizeCtx<'a> {
    fn finalize(&mut self, ty: &Type) -> Type {
        self.engine.finalize_strict(ty, &mut self.errors)
    }
}

fn finalize_pattern(ctx: &mut FinalizeCtx<'_>, pattern: &mut HirPattern) {
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                finalize_pattern(ctx, pattern);
            }
        }
        HirPattern::Struct(_, _, type_args, field_patterns) => {
            for ty in type_args {
                *ty = ctx.finalize(ty);
            }
            for field in field_patterns {
                finalize_pattern(ctx, &mut field.pattern);
            }
        }
        HirPattern::Enum(_, _, _, patterns) => {
            for pattern in patterns {
                finalize_pattern(ctx, pattern);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

fn finalize_generic_bounds(ctx: &mut FinalizeCtx<'_>, bounds: &mut HirGenericBounds) {
    let mut params = bounds.keys().copied().collect::<Vec<_>>();
    params.sort_by_key(|param| (param.owner, param.index));
    for param in params {
        if let Some(trait_bounds) = bounds.get_mut(&param) {
            for bound in trait_bounds {
                for ty in &mut bound.type_args {
                    *ty = ctx.finalize(ty);
                }
            }
        }
    }
    finalize_predicates(ctx, &mut bounds.predicates);
}

fn finalize_predicates(ctx: &mut FinalizeCtx<'_>, predicates: &mut [crate::types::Predicate]) {
    for predicate in predicates {
        match predicate {
            crate::types::Predicate::Trait { subject, args, .. } => {
                *subject = ctx.finalize(subject);
                for arg in args {
                    *arg = ctx.finalize(arg);
                }
            }
        }
    }
}

fn finalize_method_target(ctx: &mut FinalizeCtx<'_>, target: &mut HirMethodCallTarget) {
    if let Some(method_id) = target.method_id() {
        let declared = ctx
            .method_generic_params
            .get(&method_id)
            .cloned()
            .unwrap_or_default();
        target
            .method_substitution
            .retain(|binding| declared.contains(&binding.param));
    }
    target.for_each_type_mut(|ty| *ty = ctx.finalize(ty));
}

fn finalize_call_target(ctx: &mut FinalizeCtx<'_>, target: &mut HirCallTarget) {
    if let HirCallTarget::StaticMethod(target) = target {
        target.owner_ty = ctx.finalize(&target.owner_ty);
        finalize_method_target(ctx, &mut target.method);
    }
}

fn finalize_impl_receiver_pattern(ctx: &mut FinalizeCtx<'_>, pattern: &mut HirImplReceiverPattern) {
    match pattern {
        HirImplReceiverPattern::Exact(ty)
        | HirImplReceiverPattern::SliceFamily { element: ty }
        | HirImplReceiverPattern::Constructor(ty) => {
            *ty = ctx.finalize(ty);
        }
    }
}

/// Apply strict finalization to every type-bearing HIR payload.
pub(super) fn apply_finalization(hir: &mut PartialHir) -> Vec<ResolveError> {
    let method_generic_params = hir
        .traits
        .values()
        .flat_map(|trait_def| {
            trait_def
                .methods
                .values()
                .map(|method| (method.id, method.generic_params.as_slice()))
                .chain(
                    trait_def
                        .signatures
                        .values()
                        .map(|signature| (signature.id, signature.generic_params.as_slice())),
                )
        })
        .chain(
            hir.impls
                .values()
                .flat_map(|imp| imp.methods.values())
                .map(|method| (method.id, method.generic_params.as_slice())),
        )
        .map(|(method_id, generic_params)| {
            (
                method_id,
                generic_params.iter().map(|param| param.id).collect(),
            )
        })
        .collect();
    let mut ctx = FinalizeCtx {
        engine: &hir.engine,
        method_generic_params,
        errors: Vec::new(),
    };

    let mut function_ids = hir.functions.keys().copied().collect::<Vec<_>>();
    function_ids.sort();
    for id in function_ids {
        if let Some(mut function) = hir.functions.remove(&id) {
            finalize_function(&mut ctx, &mut function);
            hir.functions.insert(id, function);
        }
    }

    let mut struct_ids = hir.structs.keys().copied().collect::<Vec<_>>();
    struct_ids.sort();
    for id in struct_ids {
        if let Some(mut structure) = hir.structs.remove(&id) {
            for field in &mut structure.fields {
                field.ty = ctx.finalize(&field.ty);
            }
            hir.structs.insert(id, structure);
        }
    }

    let mut enum_ids = hir.enums.keys().copied().collect::<Vec<_>>();
    enum_ids.sort();
    for id in enum_ids {
        if let Some(mut enumeration) = hir.enums.remove(&id) {
            for variant in &mut enumeration.variants {
                match &mut variant.fields {
                    HirVariantFields::Named(fields) => {
                        for field in fields {
                            field.ty = ctx.finalize(&field.ty);
                        }
                    }
                    HirVariantFields::Positional(types) => {
                        for ty in types {
                            *ty = ctx.finalize(ty);
                        }
                    }
                    HirVariantFields::Unit => {}
                }
            }
            hir.enums.insert(id, enumeration);
        }
    }

    let mut trait_ids = hir.traits.keys().copied().collect::<Vec<_>>();
    trait_ids.sort();
    for id in trait_ids {
        if let Some(mut trait_def) = hir.traits.remove(&id) {
            finalize_predicates(&mut ctx, &mut trait_def.predicates);
            let mut methods = trait_def
                .methods
                .iter()
                .map(|(name, function)| (function.id, name.clone()))
                .collect::<Vec<_>>();
            methods.sort_by_key(|(method_id, _)| *method_id);
            for (_, name) in methods {
                if let Some(mut function) = trait_def.methods.remove(&name) {
                    finalize_function(&mut ctx, &mut function);
                    trait_def.methods.insert(name, function);
                }
            }
            let mut signature_names = trait_def.signatures.keys().cloned().collect::<Vec<_>>();
            signature_names.sort();
            for name in signature_names {
                if let Some(signature) = trait_def.signatures.get_mut(&name) {
                    finalize_function_signature(&mut ctx, signature);
                }
            }
            hir.traits.insert(id, trait_def);
        }
    }

    let mut impl_ids = hir.impls.keys().copied().collect::<Vec<_>>();
    impl_ids.sort();
    for id in impl_ids {
        if let Some(mut imp) = hir.impls.remove(&id) {
            finalize_impl_receiver_pattern(&mut ctx, &mut imp.receiver_pattern);
            for ty in &mut imp.trait_arg_types {
                *ty = ctx.finalize(ty);
            }
            for associated_type in &mut imp.associated_types {
                associated_type.ty = ctx.finalize(&associated_type.ty);
            }
            finalize_generic_bounds(&mut ctx, &mut imp.bounds);

            let mut methods = imp
                .methods
                .iter()
                .map(|(name, function)| (function.id, name.clone()))
                .collect::<Vec<_>>();
            methods.sort_by_key(|(method_id, _)| *method_id);
            for (_, name) in methods {
                if let Some(mut function) = imp.methods.remove(&name) {
                    finalize_function(&mut ctx, &mut function);
                    imp.methods.insert(name, function);
                }
            }
            hir.impls.insert(id, imp);
        }
    }

    let mut extern_ids = hir.externs.keys().copied().collect::<Vec<_>>();
    extern_ids.sort();
    for id in extern_ids {
        if let Some(ext) = hir.externs.get_mut(&id) {
            for param in &mut ext.params {
                *param = ctx.finalize(param);
            }
            ext.ret = ctx.finalize(&ext.ret);
        }
    }

    ctx.errors
        .into_iter()
        .map(|error| match error.span {
            Some(span) => ResolveError::with_span_code(
                error.error.render(&hir.engine),
                span,
                crate::diagnostic::DiagnosticCode::Type,
            ),
            None => ResolveError::non_source_code(
                error.error.render(&hir.engine),
                crate::diagnostic::DiagnosticCode::Internal,
            ),
        })
        .collect()
}

fn finalize_function(ctx: &mut FinalizeCtx<'_>, function: &mut HirFunction) {
    finalize_generic_bounds(ctx, &mut function.generic_bounds);
    for param in &mut function.params {
        param.ty = ctx.finalize(&param.ty);
    }
    function.ret_type = ctx.finalize(&function.ret_type);
    finalize_block(ctx, &mut function.body);
}

fn finalize_function_signature(ctx: &mut FinalizeCtx<'_>, signature: &mut HirFunctionSig) {
    finalize_generic_bounds(ctx, &mut signature.generic_bounds);
    for param in &mut signature.params {
        *param = ctx.finalize(param);
    }
    signature.ret = ctx.finalize(&signature.ret);
}

fn finalize_block(ctx: &mut FinalizeCtx<'_>, block: &mut HirBlock) {
    block.ty = ctx.finalize(&block.ty);
    for stmt in &mut block.stmts {
        finalize_stmt(ctx, stmt);
    }
}

fn finalize_stmt(ctx: &mut FinalizeCtx<'_>, stmt: &mut HirStmt) {
    match stmt {
        HirStmt::Let { ty, value, .. } => {
            *ty = ctx.finalize(ty);
            finalize_expr(ctx, value);
        }
        HirStmt::Expr(expr) => finalize_expr(ctx, expr),
        HirStmt::Return(Some(expr)) => finalize_expr(ctx, expr),
        HirStmt::Break(Some(expr)) => finalize_expr(ctx, expr),
        HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
    }
}

fn finalize_expr(ctx: &mut FinalizeCtx<'_>, expr: &mut HirExpr) {
    expr.ty = ctx.finalize(&expr.ty);
    match &mut expr.kind {
        HirExprKind::Open { source, binding, body } => {
            finalize_expr(ctx, source);
            binding.visit_types_mut(&mut |ty| *ty = ctx.finalize(ty));
            finalize_block(ctx, body);
        }
        HirExprKind::OwnedObjectCall { owner, args, call } => {
            finalize_expr(ctx, owner);
            for arg in args {
                finalize_expr(ctx, arg);
            }
            call.visit_types_mut(&mut |ty| *ty = ctx.finalize(ty));
        }
        HirExprKind::BinOp(_, lhs, rhs) => {
            finalize_expr(ctx, lhs);
            finalize_expr(ctx, rhs);
        }
        HirExprKind::UnaryOp(_, inner) => finalize_expr(ctx, inner),
        HirExprKind::Call(function, args, target) => {
            finalize_expr(ctx, function);
            for arg in args {
                finalize_expr(ctx, arg);
            }
            if let Some(target) = target {
                finalize_call_target(ctx, target);
            }
        }
        HirExprKind::MethodCall(receiver, _, args, _, target) => {
            finalize_expr(ctx, receiver);
            for arg in args {
                finalize_expr(ctx, arg);
            }
            if let Some(target) = target {
                finalize_method_target(ctx, target);
            }
        }
        HirExprKind::Try {
            expr,
            branch_method,
            branch_target,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            ..
        } => {
            finalize_expr(ctx, expr);
            if let Some(target) = branch_method {
                finalize_method_target(ctx, target);
            }
            if let Some(target) = branch_target {
                finalize_call_target(ctx, target);
            }
            if let Some(target) = from_residual_target {
                finalize_call_target(ctx, target);
            }
            *output_ty = ctx.finalize(output_ty);
            *residual_ty = ctx.finalize(residual_ty);
            *return_ty = ctx.finalize(return_ty);
        }
        HirExprKind::FieldAccess(inner, _, _) | HirExprKind::TupleIndex(inner, _) => {
            finalize_expr(ctx, inner)
        }
        HirExprKind::ArrayLiteral(elements) | HirExprKind::TupleLiteral(elements) => {
            for element in elements {
                finalize_expr(ctx, element);
            }
        }
        HirExprKind::ArrayRepeat(value, len) => {
            finalize_expr(ctx, value);
            if *len > 1 && !TypeFacts::is_copy(&value.ty) {
                ctx.errors.push(InferenceError {
                    error: UnifyError::NonCopyArrayRepeat {
                        found: value.ty.clone(),
                    },
                    span: Some(value.span.clone()),
                });
            }
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                finalize_expr(ctx, &mut field.value);
            }
        }
        HirExprKind::EnumVariant(_, _, args, _) | HirExprKind::Intrinsic { args, .. } => {
            for arg in args {
                finalize_expr(ctx, arg);
            }
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            finalize_expr(ctx, condition);
            finalize_block(ctx, then_branch);
            if let Some(else_branch) = else_branch {
                finalize_block(ctx, else_branch);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            finalize_expr(ctx, scrutinee);
            for arm in arms {
                finalize_pattern(ctx, &mut arm.pattern);
                if let Some(guard) = &mut arm.guard {
                    finalize_expr(ctx, guard);
                }
                finalize_block(ctx, &mut arm.body);
            }
        }
        HirExprKind::While { condition, body } => {
            finalize_expr(ctx, condition);
            finalize_block(ctx, body);
        }
        HirExprKind::For { iter, body, .. } => {
            finalize_expr(ctx, iter);
            finalize_block(ctx, body);
        }
        HirExprKind::Loop(body) | HirExprKind::Block(body) | HirExprKind::UnsafeBlock(body) => {
            finalize_block(ctx, body)
        }
        HirExprKind::Lambda {
            params,
            body,
            captures,
        } => {
            for param in params {
                param.ty = ctx.finalize(&param.ty);
            }
            for capture in captures {
                capture.ty = ctx.finalize(&capture.ty);
            }
            finalize_block(ctx, body);
        }
        HirExprKind::Ref(_, inner) | HirExprKind::Deref(inner) => finalize_expr(ctx, inner),
        HirExprKind::Cast(inner, target) => {
            finalize_expr(ctx, inner);
            *target = ctx.finalize(target);
        }
        HirExprKind::ObjectCoercion(inner, coercion) => {
            finalize_expr(ctx, inner);
            coercion.visit_types_mut(&mut |ty| *ty = ctx.finalize(ty));
        }
        HirExprKind::Assign(left, right) => {
            finalize_expr(ctx, left);
            finalize_expr(ctx, right);
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
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use crate::collect::resolver::ResolverTables;
    use crate::hir::{HirBlock, HirFunction, HirFunctionSig, HirImpl, HirImplOwner, HirTrait};
    use crate::ids::{CrateId, DefId, IdGen, LocalDefId};
    use crate::infer::ConstraintStore;
    use crate::lexer::Span;
    use crate::types::{GenericParamDecl, Type};

    use super::*;

    fn def_id(raw: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(raw))
    }

    fn function(id: DefId, name: &str, ret_type: Type) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: ret_type.clone(),
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: true,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn partial_hir(engine: InferenceEngine) -> PartialHir {
        PartialHir {
            functions: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            traits: HashMap::new(),
            impls: HashMap::new(),
            externs: HashMap::new(),
            type_aliases: HashMap::new(),
            engine,
            function_type_vars: HashMap::new(),
            import_aliases: HashMap::new(),
            loaded_module_paths: Vec::new(),
            constraint_store: ConstraintStore::new(),
            resolver: ResolverTables::default(),
            current_def_ids: BTreeSet::new(),
            root_crate_id: CrateId(0),
            local_def_ids: IdGen::new(),
            language_items: HirLanguageItems::default(),
            imported_effective_trait_methods: HashMap::new(),
            inference_sccs: HashMap::new(),
            inference_scc_order: Vec::new(),
            source_map: Default::default(),
        }
    }

    #[test]
    fn all_impl_methods_finalize_strictly_without_ownership_metadata() {
        let mut engine = InferenceEngine::new();
        let own_ret = engine.fresh_type_var_at(Span::test());
        let external_ret = engine.fresh_type_var_at(Span::test());
        let own_method_id = def_id(1);
        let external_method_id = def_id(2);
        let mut hir = partial_hir(engine);
        hir.impls.insert(
            def_id(10),
            HirImpl {
                id: def_id(10),
                owner: HirImplOwner::Named("Point".to_string()),
                type_name: "Point".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: Vec::new().into(),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([
                    ("own".to_string(), function(own_method_id, "own", own_ret)),
                    (
                        "external".to_string(),
                        function(external_method_id, "external", external_ret),
                    ),
                ]),
            },
        );

        let errors = apply_finalization(&mut hir);

        assert_eq!(errors.len(), 2);
        assert_eq!(hir.impls[&def_id(10)].methods["own"].ret_type, Type::Error);
        assert_eq!(
            hir.impls[&def_id(10)].methods["external"].ret_type,
            Type::Error
        );
    }

    #[test]
    fn trait_signatures_and_declaration_bounds_finalize_strictly() {
        let mut engine = InferenceEngine::new();
        let signature_param = engine.fresh_type_var();
        let signature_return = engine.fresh_type_var();
        let function_id = def_id(11);
        let signature_id = def_id(12);
        let generic = crate::types::GenericParamId {
            owner: signature_id,
            index: 0,
        };
        let mut hir = partial_hir(engine);
        hir.traits.insert(
            def_id(13),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: def_id(13),
                name: "Trait".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "method".to_string(),
                    HirFunctionSig {
                        id: signature_id,
                        name: "method".to_string(),
                        generic_params: vec![GenericParamDecl::type_param(generic, "T")],
                        params: vec![signature_param],
                        ret: signature_return,
                        generic_bounds: HashMap::from([(
                            generic,
                            vec![crate::types::TraitBound {
                                trait_id: def_id(14),
                                type_args: vec![Type::TypeVar(crate::ids::TypeVarId(2))],
                            }],
                        )])
                        .into(),
                        self_receiver: None,
                        is_unsafe: false,
                    },
                )]),
            },
        );
        hir.functions.insert(
            function_id,
            function(function_id, "function", Type::Generic(generic)),
        );

        let errors = apply_finalization(&mut hir);

        assert_eq!(errors.len(), 3);
        assert_eq!(
            hir.traits[&def_id(13)].signatures["method"].params[0],
            Type::Error
        );
        assert_eq!(
            hir.traits[&def_id(13)].signatures["method"].ret,
            Type::Error
        );
        assert_eq!(hir.functions[&function_id].ret_type, Type::Generic(generic));
    }
}
