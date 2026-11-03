use std::collections::HashMap;

use crate::hir::{
    HirBlockFor, HirEnum, HirExprFor, HirExprKindFor as HirExprKind, HirExtern, HirFunctionFor,
    HirImplFor, HirPattern, HirPhase, HirProgramFor, HirStmtFor, HirStruct, HirTraitFor,
    HirVariantFields,
};
use crate::ids::{AssocTypeId, DefId, FieldId, TypeId, VariantId};
use crate::type_context::TypeContext;
use crate::types::{GenericParamId, TraitBound, Type};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HirTypeLocation {
    FunctionReturn {
        function: DefId,
    },
    FunctionParam {
        function: DefId,
        index: usize,
    },
    FunctionBoundArg {
        function: DefId,
        param: GenericParamId,
        bound_index: usize,
        arg_index: usize,
    },
    ExternReturn {
        extern_id: DefId,
    },
    ExternParam {
        extern_id: DefId,
        index: usize,
    },
    StructField {
        owner: DefId,
        field: FieldId,
    },
    EnumVariantNamedField {
        owner: DefId,
        variant: VariantId,
        field: FieldId,
    },
    EnumVariantPositionalField {
        owner: DefId,
        variant: VariantId,
        index: usize,
    },
    TraitSignatureReturn {
        trait_id: DefId,
        signature: DefId,
    },
    TraitSignatureParam {
        trait_id: DefId,
        signature: DefId,
        index: usize,
    },
    TraitSignatureBoundArg {
        trait_id: DefId,
        signature: DefId,
        param: GenericParamId,
        bound_index: usize,
        arg_index: usize,
    },
    ImplReceiverArg {
        impl_id: DefId,
        index: usize,
    },
    ImplTraitArg {
        impl_id: DefId,
        index: usize,
    },
    ImplBoundArg {
        impl_id: DefId,
        param: GenericParamId,
        bound_index: usize,
        arg_index: usize,
    },
    PredicateSubject {
        owner: DefId,
        index: usize,
    },
    PredicateArg {
        owner: DefId,
        predicate_index: usize,
        arg_index: usize,
    },
    AssociatedTypeDef {
        impl_id: DefId,
        assoc_type: AssocTypeId,
    },
    TypeAliasBody {
        alias: DefId,
    },
    Block {
        owner: DefId,
        path: Vec<usize>,
    },
    LetStmt {
        owner: DefId,
        path: Vec<usize>,
        name: String,
    },
    Expr {
        owner: DefId,
        path: Vec<usize>,
    },
    ClosureCapture {
        owner: DefId,
        path: Vec<usize>,
        name: String,
    },
    ClosureParam {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    CastTarget {
        owner: DefId,
        path: Vec<usize>,
    },
    MethodCallTraitArg {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    MethodCallOwnerSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    MethodCallMethodSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    StaticCallOwner {
        owner: DefId,
        path: Vec<usize>,
    },
    StaticCallTraitArg {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    StaticCallOwnerSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    StaticCallMethodSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    TryOutput {
        owner: DefId,
        path: Vec<usize>,
    },
    TryResidual {
        owner: DefId,
        path: Vec<usize>,
    },
    TryReturn {
        owner: DefId,
        path: Vec<usize>,
    },
    TryBranchTraitArg {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    TryBranchOwnerSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    TryBranchMethodSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    TryFromResidualStaticCallOwner {
        owner: DefId,
        path: Vec<usize>,
    },
    TryFromResidualStaticCallTraitArg {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    TryFromResidualStaticCallOwnerSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    TryFromResidualStaticCallMethodSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    StructPatternArg {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
}

#[derive(Debug, Clone, Default)]
pub struct HirTypeIds {
    ids: HashMap<HirTypeLocation, TypeId>,
}

impl HirTypeIds {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, location: HirTypeLocation, id: TypeId) {
        if let Some(existing) = self.ids.insert(location.clone(), id) {
            debug_assert_eq!(
                existing, id,
                "duplicate HIR type location with different TypeId: {:?}",
                location
            );
        }
    }

    pub fn get(&self, location: &HirTypeLocation) -> Option<TypeId> {
        self.ids.get(location).copied()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&HirTypeLocation, TypeId)> {
        self.ids.iter().map(|(location, id)| (location, *id))
    }
}

pub fn collect_hir_type_ids<P: HirPhase>(
    program: &HirProgramFor<P>,
    context: &mut TypeContext,
) -> HirTypeIds {
    let mut ids = HirTypeIds::new();

    let mut functions: Vec<_> = program.functions.values().collect();
    functions.sort_by_key(|function| function.id);
    for function in functions {
        collect_function_signature(function, context, &mut ids);
        collect_block(function.id, &function.body, Vec::new(), context, &mut ids);
    }

    let mut externs: Vec<_> = program.externs.values().collect();
    externs.sort_by_key(|ext| ext.id);
    for ext in externs {
        collect_extern_signature(ext, context, &mut ids);
    }

    let mut aliases: Vec<_> = program.type_aliases.values().collect();
    aliases.sort_by_key(|alias| alias.id);
    for alias in aliases {
        record_type(
            context,
            &mut ids,
            HirTypeLocation::TypeAliasBody { alias: alias.id },
            &alias.ty,
        );
    }

    let mut structs: Vec<_> = program.structs.values().collect();
    structs.sort_by_key(|strukt| strukt.id);
    for strukt in structs {
        collect_struct_fields(strukt, context, &mut ids);
    }

    let mut enums: Vec<_> = program.enums.values().collect();
    enums.sort_by_key(|enm| enm.id);
    for enm in enums {
        collect_enum_fields(enm, context, &mut ids);
    }

    let mut traits: Vec<_> = program.traits.values().collect();
    traits.sort_by_key(|trait_def| trait_def.id);
    for trait_def in traits {
        collect_trait_signatures(trait_def, context, &mut ids);
        let mut methods: Vec<_> = trait_def.methods.values().collect();
        methods.sort_by_key(|method| method.id);
        for method in methods {
            collect_function_signature(method, context, &mut ids);
            collect_block(method.id, &method.body, Vec::new(), context, &mut ids);
        }
    }

    let mut impls: Vec<_> = program.impls.values().collect();
    impls.sort_by_key(|imp| imp.id);
    for imp in impls {
        collect_impl_types(imp, context, &mut ids);
        let mut methods: Vec<_> = imp.methods.values().collect();
        methods.sort_by_key(|method| method.id);
        for method in methods {
            collect_function_signature(method, context, &mut ids);
            collect_block(method.id, &method.body, Vec::new(), context, &mut ids);
        }
    }

    ids
}

fn record_type(
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
    location: HirTypeLocation,
    ty: &Type,
) {
    let id = context.intern_canonical_type(ty);
    ids.insert(location, id);
}

fn collect_function_signature<P: HirPhase>(
    function: &HirFunctionFor<P>,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    record_type(
        context,
        ids,
        HirTypeLocation::FunctionReturn {
            function: function.id,
        },
        &function.ret_type,
    );
    for (index, param) in function.params.iter().enumerate() {
        record_type(
            context,
            ids,
            HirTypeLocation::FunctionParam {
                function: function.id,
                index,
            },
            &param.ty,
        );
    }
    collect_generic_bound_args(
        &function.generic_bounds,
        context,
        ids,
        |param, bound_index, arg_index| HirTypeLocation::FunctionBoundArg {
            function: function.id,
            param,
            bound_index,
            arg_index,
        },
    );
    collect_predicate_types(
        &function.generic_bounds.predicates,
        function.id,
        context,
        ids,
    );
}

fn collect_extern_signature(ext: &HirExtern, context: &mut TypeContext, ids: &mut HirTypeIds) {
    record_type(
        context,
        ids,
        HirTypeLocation::ExternReturn { extern_id: ext.id },
        &ext.ret,
    );
    for (index, param) in ext.params.iter().enumerate() {
        record_type(
            context,
            ids,
            HirTypeLocation::ExternParam {
                extern_id: ext.id,
                index,
            },
            param,
        );
    }
}

fn collect_struct_fields(strukt: &HirStruct, context: &mut TypeContext, ids: &mut HirTypeIds) {
    for field in &strukt.fields {
        record_type(
            context,
            ids,
            HirTypeLocation::StructField {
                owner: strukt.id,
                field: field.id,
            },
            &field.ty,
        );
    }
}

fn collect_enum_fields(enm: &HirEnum, context: &mut TypeContext, ids: &mut HirTypeIds) {
    for variant in &enm.variants {
        match &variant.fields {
            HirVariantFields::Named(fields) => {
                for field in fields {
                    record_type(
                        context,
                        ids,
                        HirTypeLocation::EnumVariantNamedField {
                            owner: enm.id,
                            variant: variant.id,
                            field: field.id,
                        },
                        &field.ty,
                    );
                }
            }
            HirVariantFields::Positional(types) => {
                for (index, ty) in types.iter().enumerate() {
                    record_type(
                        context,
                        ids,
                        HirTypeLocation::EnumVariantPositionalField {
                            owner: enm.id,
                            variant: variant.id,
                            index,
                        },
                        ty,
                    );
                }
            }
            HirVariantFields::Unit => {}
        }
    }
}

fn collect_trait_signatures<P: HirPhase>(
    trait_def: &HirTraitFor<P>,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    collect_predicate_types(&trait_def.predicates, trait_def.id, context, ids);
    let mut signatures: Vec<_> = trait_def.signatures.values().collect();
    signatures.sort_by_key(|signature| signature.id);
    for signature in signatures {
        record_type(
            context,
            ids,
            HirTypeLocation::TraitSignatureReturn {
                trait_id: trait_def.id,
                signature: signature.id,
            },
            &signature.ret,
        );
        for (index, param) in signature.params.iter().enumerate() {
            record_type(
                context,
                ids,
                HirTypeLocation::TraitSignatureParam {
                    trait_id: trait_def.id,
                    signature: signature.id,
                    index,
                },
                param,
            );
        }
        collect_generic_bound_args(
            &signature.generic_bounds,
            context,
            ids,
            |param, bound_index, arg_index| HirTypeLocation::TraitSignatureBoundArg {
                trait_id: trait_def.id,
                signature: signature.id,
                param,
                bound_index,
                arg_index,
            },
        );
        collect_predicate_types(
            &signature.generic_bounds.predicates,
            signature.id,
            context,
            ids,
        );
    }
}

fn collect_impl_types<P: HirPhase>(
    imp: &HirImplFor<P>,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    let mut receiver_index = 0;
    P::visit_impl_receiver_types(&imp.receiver_pattern, &mut |ty| {
        record_type(
            context,
            ids,
            HirTypeLocation::ImplReceiverArg {
                impl_id: imp.id,
                index: receiver_index,
            },
            ty,
        );
        receiver_index += 1;
    });

    for (index, ty) in imp.trait_arg_types.iter().enumerate() {
        record_type(
            context,
            ids,
            HirTypeLocation::ImplTraitArg {
                impl_id: imp.id,
                index,
            },
            ty,
        );
    }

    for assoc in &imp.associated_types {
        record_type(
            context,
            ids,
            HirTypeLocation::AssociatedTypeDef {
                impl_id: imp.id,
                assoc_type: assoc.id,
            },
            &assoc.ty,
        );
    }

    let mut bounds: Vec<_> = imp.bounds.iter().collect();
    bounds.sort_by_key(|(param, _)| (param.owner, param.index));
    for (param, trait_bounds) in bounds {
        for (bound_index, bound) in trait_bounds.iter().enumerate() {
            collect_bound_type_args(bound, context, ids, |arg_index| {
                HirTypeLocation::ImplBoundArg {
                    impl_id: imp.id,
                    param: *param,
                    bound_index,
                    arg_index,
                }
            });
        }
    }
    collect_predicate_types(&imp.bounds.predicates, imp.id, context, ids);
}

fn collect_predicate_types(
    predicates: &[crate::types::Predicate],
    owner: DefId,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    for (predicate_index, predicate) in predicates.iter().enumerate() {
        match predicate {
            crate::types::Predicate::Trait { subject, args, .. } => {
                record_type(
                    context,
                    ids,
                    HirTypeLocation::PredicateSubject {
                        owner,
                        index: predicate_index,
                    },
                    subject,
                );
                for (arg_index, arg) in args.iter().enumerate() {
                    record_type(
                        context,
                        ids,
                        HirTypeLocation::PredicateArg {
                            owner,
                            predicate_index,
                            arg_index,
                        },
                        arg,
                    );
                }
            }
        }
    }
}

fn child_path(path: &[usize], segment: usize) -> Vec<usize> {
    let mut child = path.to_vec();
    child.push(segment);
    child
}

fn collect_block<P: HirPhase>(
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

fn collect_generic_bound_args(
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

fn collect_bound_type_args(
    bound: &TraitBound,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
    location: impl Fn(usize) -> HirTypeLocation,
) {
    for (arg_index, ty) in bound.type_args.iter().enumerate() {
        record_type(context, ids, location(arg_index), ty);
    }
}

#[cfg(test)]
mod tests;
