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
    /// Static proof terms (including owner constructors), in type visitor order.
    OpenEvidence {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    /// Static proof terms (including owner constructors), in type visitor order.
    OwnedObjectCallEvidence {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    /// Static coercion proof terms, not necessarily runtime storage types.
    ObjectCoercionEvidence {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
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
        HirExprKind::Open { source, binding, body } => {
            let mut index = 0;
            binding.visit_types(&mut |ty| {
                record_type(context, ids, HirTypeLocation::OpenEvidence { owner, path: path.clone(), index }, ty);
                index += 1;
            });
            let mut source_path = path.clone();
            source_path.push(0);
            collect_expr(owner, source, source_path, context, ids);
            let mut body_path = path;
            body_path.push(1);
            collect_block(owner, body, body_path, context, ids);
        }
        HirExprKind::OwnedObjectCall {
            owner: value,
            args,
            call,
        } => {
            let mut index = 0;
            call.visit_types(&mut |ty| {
                record_type(
                    context,
                    ids,
                    HirTypeLocation::OwnedObjectCallEvidence {
                        owner,
                        path: path.clone(),
                        index,
                    },
                    ty,
                );
                index += 1;
            });
            collect_expr(owner, value, child_path(&path, 0), context, ids);
            for (index, arg) in args.iter().enumerate() {
                collect_expr(owner, arg, child_path(&path, index + 1), context, ids);
            }
        }
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
        HirExprKind::ObjectCoercion(inner, coercion) => {
            let mut index = 0;
            coercion.visit_types(&mut |ty| {
                record_type(
                    context,
                    ids,
                    HirTypeLocation::ObjectCoercionEvidence {
                        owner,
                        path: path.clone(),
                        index,
                    },
                    ty,
                );
                index += 1;
            });
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
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::hir::{
        HirAssociatedTypeDef, HirBlock, HirEnum, HirExpr, HirExprKind, HirExtern, HirField,
        HirFunction, HirFunctionSig, HirImpl, HirImplOwner, HirMethodCallTarget, HirNameTables,
        HirParam, HirPattern, HirProgram, HirStmt, HirStruct, HirTrait, HirVariant,
        HirVariantFields,
    };
    use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
    use crate::type_context::TypeContext;
    use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, TraitBound, Type};

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn generic(owner: DefId, index: u32) -> GenericParamId {
        GenericParamId { owner, index }
    }

    fn empty_body(ty: Type) -> HirBlock {
        HirBlock {
            stmts: Vec::new(),
            ty,
        }
    }

    fn function(id: DefId, name: &str, param_ty: Type, ret_type: Type) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: param_ty,
                mutable: false,
                is_ref: false,
            }],
            ret_type: ret_type.clone(),
            body: empty_body(ret_type),
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn program(
        functions: HashMap<DefId, HirFunction>,
        structs: HashMap<DefId, HirStruct>,
        enums: HashMap<DefId, HirEnum>,
        traits: HashMap<DefId, HirTrait>,
        impls: HashMap<DefId, HirImpl>,
        externs: HashMap<DefId, HirExtern>,
        names: HirNameTables,
        canonical_names: HashMap<DefId, String>,
    ) -> HirProgram {
        HirProgram::from_id_parts_with_names_and_canonical_names(
            functions,
            structs,
            enums,
            traits,
            impls,
            externs,
            names,
            crate::hir::HirLanguageItems::default(),
            &canonical_names,
        )
    }

    fn program_with_function_body(owner: DefId, body: HirBlock) -> HirProgram {
        let function = HirFunction {
            id: owner,
            name: "main".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: body.ty.clone(),
            body,
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        program(
            HashMap::from([(owner, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("main".to_string(), owner)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        )
    }

    fn single_type_arg_bound(trait_id: DefId, ty: Type) -> TraitBound {
        TraitBound {
            trait_id,
            type_args: vec![ty],
        }
    }

    #[test]
    fn hir_type_ids_record_and_read_locations() {
        let function = def_id(1);
        let location = HirTypeLocation::FunctionReturn { function };
        let id = crate::ids::TypeId(7);
        let mut ids = HirTypeIds::new();

        assert!(ids.is_empty());
        ids.insert(location.clone(), id);

        assert_eq!(ids.get(&location), Some(id));
        assert_eq!(ids.len(), 1);
        assert!(!ids.is_empty());
        assert_eq!(ids.iter().collect::<Vec<_>>(), vec![(&location, id)]);
    }

    #[test]
    fn hir_type_ids_collect_top_level_type_locations() {
        let function_id = def_id(1);
        let extern_id = def_id(2);
        let struct_id = def_id(3);
        let other_struct_id = def_id(4);
        let enum_id = def_id(5);
        let trait_id = def_id(6);
        let signature_id = def_id(7);
        let impl_id = def_id(8);
        let assoc_id = AssocTypeId(0);
        let generic_param = generic(impl_id, 0);
        let projection = Type::Projection {
            ty: Box::new(Type::Generic(generic_param)),
            trait_id,
            assoc_type: AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: assoc_id,
            },
            trait_args: vec![Type::I64],
        };

        let program = program(
            HashMap::from([(
                function_id,
                function(function_id, "id", Type::I64, Type::I64),
            )]),
            HashMap::from([
                (
                    struct_id,
                    HirStruct {
                        id: struct_id,
                        name: "Box".to_string(),
                        generic_params: Vec::new(),
                        fields: vec![HirField {
                            id: FieldId(0),
                            name: "value".to_string(),
                            ty: Type::Struct {
                                id: struct_id,
                                args: Vec::new(),
                            },
                            public: false,
                        }],
                    },
                ),
                (
                    other_struct_id,
                    HirStruct {
                        id: other_struct_id,
                        name: "OtherBox".to_string(),
                        generic_params: Vec::new(),
                        fields: vec![HirField {
                            id: FieldId(0),
                            name: "value".to_string(),
                            ty: Type::Struct {
                                id: other_struct_id,
                                args: Vec::new(),
                            },
                            public: false,
                        }],
                    },
                ),
            ]),
            HashMap::from([(
                enum_id,
                HirEnum {
                    id: enum_id,
                    name: "Maybe".to_string(),
                    generic_params: Vec::new(),
                    variants: vec![
                        HirVariant {
                            id: VariantId(0),
                            name: "Named".to_string(),
                            fields: HirVariantFields::Named(vec![HirField {
                                id: FieldId(0),
                                name: "payload".to_string(),
                                ty: Type::Bool,
                                public: false,
                            }]),
                        },
                        HirVariant {
                            id: VariantId(1),
                            name: "Tuple".to_string(),
                            fields: HirVariantFields::Positional(vec![Type::I64]),
                        },
                    ],
                },
            )]),
            HashMap::from([(
                trait_id,
                HirTrait {
                    target: None,
                    predicates: Vec::new(),
                    id: trait_id,
                    name: "Iterable".to_string(),
                    generic_params: Vec::new(),
                    associated_types: Vec::new(),
                    methods: HashMap::new(),
                    signatures: HashMap::from([(
                        "next".to_string(),
                        HirFunctionSig {
                            id: signature_id,
                            name: "next".to_string(),
                            generic_params: Vec::new(),
                            params: vec![Type::I64],
                            ret: Type::Bool,
                            generic_bounds: HashMap::new().into(),
                            self_receiver: None,
                            is_unsafe: false,
                        },
                    )]),
                },
            )]),
            HashMap::from([(
                impl_id,
                HirImpl {
                    id: impl_id,
                    owner: HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: GenericParamDecl::type_params(impl_id, ["T"]),
                    receiver_pattern: vec![Type::Generic(generic_param)].into(),
                    trait_name: Some("Iterable".to_string()),
                    trait_id: Some(trait_id),
                    trait_generics: Vec::new(),
                    trait_arg_types: vec![Type::I64],
                    associated_types: vec![HirAssociatedTypeDef {
                        id: assoc_id,
                        name: "Item".to_string(),
                        kind: crate::type_services::kind::Kind::Type,
                        ty: projection.clone(),
                    }],
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::new(),
                },
            )]),
            HashMap::from([(
                extern_id,
                HirExtern {
                    id: extern_id,
                    name: "puts".to_string(),
                    params: vec![Type::Pointer(Box::new(Type::U8))],
                    ret: Type::I32,
                    variadic: false,
                    is_unsafe: false,
                },
            )]),
            HirNameTables {
                functions_by_name: HashMap::from([("id".to_string(), function_id)]),
                structs_by_name: HashMap::from([
                    ("Box".to_string(), struct_id),
                    ("OtherBox".to_string(), other_struct_id),
                ]),
                enums_by_name: HashMap::from([("Maybe".to_string(), enum_id)]),
                traits_by_name: HashMap::from([("Iterable".to_string(), trait_id)]),
                externs_by_name: HashMap::from([("puts".to_string(), extern_id)]),
                type_aliases_by_name: HashMap::new(),
            },
            HashMap::new(),
        );

        let mut context = TypeContext::new();
        let ids = collect_hir_type_ids(&program, &mut context);

        let function_ret = ids
            .get(&HirTypeLocation::FunctionReturn {
                function: function_id,
            })
            .unwrap();
        let function_param = ids
            .get(&HirTypeLocation::FunctionParam {
                function: function_id,
                index: 0,
            })
            .unwrap();
        assert_eq!(function_ret, function_param);
        assert_eq!(context.type_for(function_ret), Type::I64);
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::ExternParam {
                    extern_id,
                    index: 0,
                })
                .unwrap()
            ),
            Type::Pointer(Box::new(Type::U8))
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::ExternReturn { extern_id })
                    .unwrap()
            ),
            Type::I32
        );
        let first_nominal = ids
            .get(&HirTypeLocation::StructField {
                owner: struct_id,
                field: FieldId(0),
            })
            .unwrap();
        let second_nominal = ids
            .get(&HirTypeLocation::StructField {
                owner: other_struct_id,
                field: FieldId(0),
            })
            .unwrap();
        assert_ne!(first_nominal, second_nominal);
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::EnumVariantNamedField {
                    owner: enum_id,
                    variant: VariantId(0),
                    field: FieldId(0),
                })
                .unwrap()
            ),
            Type::Bool
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::EnumVariantPositionalField {
                    owner: enum_id,
                    variant: VariantId(1),
                    index: 0,
                })
                .unwrap()
            ),
            Type::I64
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::TraitSignatureReturn {
                    trait_id,
                    signature: signature_id,
                })
                .unwrap()
            ),
            Type::Bool
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::TraitSignatureParam {
                    trait_id,
                    signature: signature_id,
                    index: 0,
                })
                .unwrap()
            ),
            Type::I64
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::ImplReceiverArg { impl_id, index: 0 })
                    .unwrap()
            ),
            Type::Generic(generic_param)
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::ImplTraitArg { impl_id, index: 0 })
                    .unwrap()
            ),
            Type::I64
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::AssociatedTypeDef {
                    impl_id,
                    assoc_type: assoc_id,
                })
                .unwrap()
            ),
            projection
        );
        assert!(!ids.is_empty());
    }

    #[test]
    fn hir_type_ids_collect_downstream_required_payload_types() {
        let owner = def_id(200);
        let trait_id = def_id(201);
        let struct_id = def_id(202);
        let method_id = def_id(203);
        let mut context = TypeContext::new();
        let expr = HirExpr {
            kind: HirExprKind::Cast(
                Box::new(HirExpr {
                    kind: HirExprKind::MethodCall(
                        Box::new(HirExpr {
                            kind: HirExprKind::Var("receiver".to_string()),
                            ty: Type::Struct {
                                id: struct_id,
                                args: vec![Type::I64],
                            },
                            span: crate::lexer::Span::test(),
                        }),
                        "value".to_string(),
                        Vec::new(),
                        None,
                        Some(HirMethodCallTarget::impl_method(
                            owner,
                            method_id,
                            Some(crate::hir::HirSelectedTraitMember {
                                trait_id,
                                member_id: method_id,
                                trait_args: vec![Type::Bool],
                            }),
                        )),
                    ),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                }),
                Type::U64,
            ),
            ty: Type::U64,
            span: crate::lexer::Span::test(),
        };
        let pattern = HirPattern::Struct(
            "Box".to_string(),
            Some(struct_id),
            vec![Type::Bool],
            Vec::new(),
        );
        let program = program_with_function_body(
            owner,
            HirBlock {
                stmts: vec![HirStmt::Expr(expr)],
                ty: Type::Unit,
            },
        );
        let ids = collect_hir_type_ids(&program, &mut context);

        assert!(ids
            .get(&HirTypeLocation::CastTarget {
                owner,
                path: vec![0, 0],
            })
            .is_some());
        assert!(ids
            .get(&HirTypeLocation::MethodCallTraitArg {
                owner,
                path: vec![0, 0, 0],
                index: 0,
            })
            .is_some());

        let mut pattern_context = TypeContext::new();
        let mut pattern_ids = HirTypeIds::new();
        collect_pattern_type_ids(
            owner,
            &pattern,
            vec![9],
            &mut pattern_context,
            &mut pattern_ids,
        );
        assert!(pattern_ids
            .get(&HirTypeLocation::StructPatternArg {
                owner,
                path: vec![9],
                index: 0,
            })
            .is_some());
    }

    #[test]
    fn hir_type_ids_collect_body_type_locations() {
        let function_id = def_id(20);
        let capture_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };
        let lambda_ty = Type::function(vec![Type::I64], Type::I64);
        let function = HirFunction {
            id: function_id,
            name: "main".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: vec![
                    crate::hir::HirStmt::Let {
                        name: "x".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        ty: Type::I64,
                        value: crate::hir::HirExpr {
                            kind: crate::hir::HirExprKind::IntLiteral(1),
                            ty: Type::I64,
                            span: crate::lexer::Span::test(),
                        },
                        mutable: false,
                    },
                    crate::hir::HirStmt::Expr(crate::hir::HirExpr {
                        kind: crate::hir::HirExprKind::Lambda {
                            params: vec![HirParam {
                                name: "value".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                ty: Type::I64,
                                mutable: false,
                                is_ref: false,
                            }],
                            body: HirBlock {
                                stmts: vec![crate::hir::HirStmt::Expr(crate::hir::HirExpr {
                                    kind: crate::hir::HirExprKind::Var("value".to_string()),
                                    ty: Type::I64,
                                    span: crate::lexer::Span::test(),
                                })],
                                ty: Type::I64,
                            },
                            captures: vec![crate::hir::HirClosureCapture {
                                name: "x".to_string(),
                                local_id: crate::ids::HirLocalId(0),
                                kind: crate::hir::HirClosureCaptureKind::SharedBorrow,
                                mutable: false,
                                ty: capture_ty.clone(),
                            }],
                        },
                        ty: lambda_ty.clone(),
                        span: crate::lexer::Span::test(),
                    }),
                ],
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let program = program(
            HashMap::from([(function_id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("main".to_string(), function_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        let mut context = TypeContext::new();
        let ids = collect_hir_type_ids(&program, &mut context);

        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::Block {
                    owner: function_id,
                    path: vec![],
                })
                .unwrap()
            ),
            Type::Unit
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::LetStmt {
                    owner: function_id,
                    path: vec![0],
                    name: "x".to_string(),
                })
                .unwrap()
            ),
            Type::I64
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::Expr {
                    owner: function_id,
                    path: vec![0, 0],
                })
                .unwrap()
            ),
            Type::I64
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::Expr {
                    owner: function_id,
                    path: vec![1, 0],
                })
                .unwrap()
            ),
            lambda_ty
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::ClosureCapture {
                    owner: function_id,
                    path: vec![1, 0],
                    name: "x".to_string(),
                })
                .unwrap()
            ),
            capture_ty
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::ClosureParam {
                    owner: function_id,
                    path: vec![1, 0],
                    index: 0,
                })
                .unwrap()
            ),
            Type::I64
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::Block {
                    owner: function_id,
                    path: vec![1, 0, 1],
                })
                .unwrap()
            ),
            Type::I64
        );
    }

    #[test]
    fn hir_type_ids_collect_in_stable_semantic_order() {
        let mut functions = HashMap::new();
        let mut function_names = HashMap::new();
        for index in (1..=16).rev() {
            let id = def_id(index);
            let ty = Type::Struct {
                id,
                args: Vec::new(),
            };
            functions.insert(id, function(id, &format!("f{index}"), ty.clone(), ty));
            function_names.insert(format!("f{index}"), id);
        }

        let program = program(
            functions,
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: function_names,
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        let mut context = TypeContext::new();
        let ids = collect_hir_type_ids(&program, &mut context);

        for index in 1..=16 {
            let function = def_id(index);
            let id = ids
                .get(&HirTypeLocation::FunctionReturn { function })
                .unwrap();
            assert_eq!(id, crate::ids::TypeId(index - 1));
            assert_eq!(
                context.type_for(id),
                Type::Struct {
                    id: function,
                    args: Vec::new(),
                }
            );
        }
    }

    #[test]
    fn hir_type_ids_collect_bound_type_args() {
        let function_id = def_id(1);
        let trait_id = def_id(2);
        let signature_id = def_id(3);
        let impl_id = def_id(4);
        let function_param = generic(function_id, 0);
        let signature_param = generic(signature_id, 0);
        let impl_param = generic(impl_id, 0);

        let mut bounded_function = function(function_id, "bounded", Type::I64, Type::I64);
        *bounded_function.generic_bounds = HashMap::from([(
            function_param,
            vec![single_type_arg_bound(
                trait_id,
                Type::Pointer(Box::new(Type::U8)),
            )],
        )])
        .into();

        let program = program(
            HashMap::from([(function_id, bounded_function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                trait_id,
                HirTrait {
                    target: None,
                    predicates: Vec::new(),
                    id: trait_id,
                    name: "Iterable".to_string(),
                    generic_params: Vec::new(),
                    associated_types: Vec::new(),
                    methods: HashMap::new(),
                    signatures: HashMap::from([(
                        "next".to_string(),
                        HirFunctionSig {
                            id: signature_id,
                            name: "next".to_string(),
                            generic_params: vec![GenericParamDecl::type_param(
                                signature_param,
                                "T",
                            )],
                            params: vec![Type::I64],
                            ret: Type::Bool,
                            generic_bounds: HashMap::from([(
                                signature_param,
                                vec![single_type_arg_bound(trait_id, Type::Bool)],
                            )])
                            .into(),
                            self_receiver: None,
                            is_unsafe: false,
                        },
                    )]),
                },
            )]),
            HashMap::from([(
                impl_id,
                HirImpl {
                    id: impl_id,
                    owner: HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: GenericParamDecl::type_params(impl_id, ["T"]),
                    receiver_pattern: Vec::new().into(),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: HashMap::from([(
                        impl_param,
                        vec![single_type_arg_bound(trait_id, Type::Generic(impl_param))],
                    )])
                    .into(),
                    methods: HashMap::new(),
                },
            )]),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("bounded".to_string(), function_id)]),
                traits_by_name: HashMap::from([("Iterable".to_string(), trait_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        let mut context = TypeContext::new();
        let ids = collect_hir_type_ids(&program, &mut context);

        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::FunctionBoundArg {
                    function: function_id,
                    param: function_param,
                    bound_index: 0,
                    arg_index: 0,
                })
                .unwrap()
            ),
            Type::Pointer(Box::new(Type::U8))
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::TraitSignatureBoundArg {
                    trait_id,
                    signature: signature_id,
                    param: signature_param,
                    bound_index: 0,
                    arg_index: 0,
                })
                .unwrap()
            ),
            Type::Bool
        );
        assert_eq!(
            context.type_for(
                ids.get(&HirTypeLocation::ImplBoundArg {
                    impl_id,
                    param: impl_param,
                    bound_index: 0,
                    arg_index: 0,
                })
                .unwrap()
            ),
            Type::Generic(impl_param)
        );
    }
}
