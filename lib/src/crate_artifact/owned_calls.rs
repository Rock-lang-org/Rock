//! Recheck unique-owner proofs against remapped local and dependency interfaces.
use std::collections::HashMap;

use crate::crate_system::CrateContext;
use crate::hir::*;

pub(super) fn validate(
    interface: &super::ArtifactCrateInterface,
    bodies: &super::ArtifactCrossCrateHir,
    items: &HirLanguageItems,
    name: &str,
    context: &CrateContext,
) -> Result<(), String> {
    let mut calls = Vec::new();
    for function in bodies
        .generic_functions
        .values()
        .chain(
            bodies
                .traits_with_defaults
                .values()
                .flat_map(|definition| definition.methods.values()),
        )
        .chain(
            bodies
                .generic_impls
                .iter()
                .flat_map(|implementation| implementation.methods.values()),
        )
    {
        collect_block(&function.body, &mut calls);
    }
    let mut functions = HashMap::new();
    let mut structs = HashMap::new();
    let mut enums = HashMap::new();
    let mut traits = HashMap::new();
    let mut impls = HashMap::new();
    let mut externs = HashMap::new();
    let mut effective_trait_methods = HashMap::new();
    for interface in std::iter::once(interface).chain(
        context
            .extern_crates()
            .map(|dependency| dependency.metadata().interface()),
    ) {
        for (&key, &method) in &interface.effective_trait_methods {
            if effective_trait_methods
                .insert(key, method)
                .is_some_and(|old| old != method)
            {
                return Err(format!("artifact owned-object evidence: conflicting canonical member mapping for {key:?}"));
            }
        }
        functions.extend(
            interface
                .function_items()
                .into_iter()
                .map(|(_, function)| (function.id, function)),
        );
        structs.extend(
            interface
                .struct_items()
                .into_iter()
                .map(|(_, definition)| (definition.id, definition)),
        );
        enums.extend(
            interface
                .enum_items()
                .into_iter()
                .map(|(_, definition)| (definition.id, definition)),
        );
        traits.extend(
            interface
                .trait_items()
                .into_iter()
                .map(|(_, definition)| (definition.id, definition)),
        );
        impls.extend(
            interface
                .impl_items()
                .into_iter()
                .map(|implementation| (implementation.id, implementation)),
        );
        externs.extend(
            interface
                .extern_items()
                .into_iter()
                .map(|external| (external.id, external)),
        );
    }
    let dependencies = context.extern_crates().collect::<Vec<_>>();
    let language_items = crate::language_items::merge_language_item_providers(
        std::iter::once((name, items)).chain(
            dependencies
                .iter()
                .map(|dependency| (dependency.name(), dependency.metadata().language_items())),
        ),
    )
    .map_err(|error| error.to_string())?;
    let mut program = HirProgram::from_id_parts_with_names_and_canonical_names(
        functions,
        structs,
        enums,
        traits,
        impls,
        externs,
        HirNameTables::default(),
        language_items,
        &HashMap::new(),
    );
    program.indexes.effective_trait_methods = effective_trait_methods;
    let mut scope_errors = Vec::new();
    crate::hir::existential::validate_program(&program, &mut scope_errors);
    for function in bodies
        .generic_functions
        .values()
        .chain(
            bodies
                .traits_with_defaults
                .values()
                .flat_map(|declaration| declaration.methods.values()),
        )
        .chain(
            bodies
                .generic_impls
                .iter()
                .flat_map(|implementation| implementation.methods.values()),
        )
    {
        crate::hir::existential::validate_function(&program, function, &mut scope_errors);
    }
    if !scope_errors.is_empty() {
        return Err(format!(
            "artifact existential evidence: {}",
            scope_errors.join("; ")
        ));
    }
    for (owner, args, result, call) in calls {
        crate::hir::owned_objects::validate(&program, owner, args, result, call)
            .map_err(|error| format!("artifact owned-object evidence: {error}"))?;
    }
    Ok(())
}

type Call<'a> = (
    &'a AcceptedHirExpr,
    &'a [AcceptedHirExpr],
    &'a crate::types::Type,
    &'a HirOwnedObjectCall,
);

fn collect_block<'a>(block: &'a AcceptedHirBlock, calls: &mut Vec<Call<'a>>) {
    for statement in &block.stmts {
        match statement {
            HirStmtFor::Let { value, .. }
            | HirStmtFor::Expr(value)
            | HirStmtFor::Return(Some(value))
            | HirStmtFor::Break(Some(value)) => collect_expr(value, calls),
            _ => {}
        }
    }
}

fn collect_expr<'a>(expr: &'a AcceptedHirExpr, calls: &mut Vec<Call<'a>>) {
    match &expr.kind {
        HirExprKindFor::Open { source, body, .. } => {
            collect_expr(source, calls);
            collect_block(body, calls);
        }
        HirExprKindFor::OwnedObjectCall { owner, args, call } => {
            calls.push((owner, args, &expr.ty, call));
            collect_expr(owner, calls);
            for arg in args {
                collect_expr(arg, calls);
            }
        }
        HirExprKindFor::Call(callee, args, _)
        | HirExprKindFor::MethodCall(callee, _, args, _, _) => {
            collect_expr(callee, calls);
            for arg in args {
                collect_expr(arg, calls);
            }
        }
        HirExprKindFor::ArrayLiteral(args)
        | HirExprKindFor::TupleLiteral(args)
        | HirExprKindFor::EnumVariant(_, _, args, _)
        | HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                collect_expr(arg, calls);
            }
        }
        HirExprKindFor::ArrayRepeat(inner, _)
        | HirExprKindFor::FieldAccess(inner, _, _)
        | HirExprKindFor::TupleIndex(inner, _)
        | HirExprKindFor::UnaryOp(_, inner)
        | HirExprKindFor::Ref(_, inner)
        | HirExprKindFor::Deref(inner)
        | HirExprKindFor::Cast(inner, _)
        | HirExprKindFor::ObjectCoercion(inner, _)
        | HirExprKindFor::Try { expr: inner, .. } => collect_expr(inner, calls),
        HirExprKindFor::BinOp(_, left, right) | HirExprKindFor::Assign(left, right) => {
            collect_expr(left, calls);
            collect_expr(right, calls);
        }
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_expr(condition, calls);
            collect_block(then_branch, calls);
            if let Some(body) = else_branch {
                collect_block(body, calls);
            }
        }
        HirExprKindFor::Match { scrutinee, arms } => {
            collect_expr(scrutinee, calls);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_expr(guard, calls);
                }
                collect_block(&arm.body, calls);
            }
        }
        HirExprKindFor::While { condition, body } => {
            collect_expr(condition, calls);
            collect_block(body, calls);
        }
        HirExprKindFor::For { iter, body, .. } => {
            collect_expr(iter, calls);
            collect_block(body, calls);
        }
        HirExprKindFor::Loop(body)
        | HirExprKindFor::Block(body)
        | HirExprKindFor::UnsafeBlock(body)
        | HirExprKindFor::Lambda { body, .. } => collect_block(body, calls),
        HirExprKindFor::StructLiteral(_, _, fields) => {
            for field in fields {
                collect_expr(&field.value, calls);
            }
        }
        HirExprKindFor::IntLiteral(_)
        | HirExprKindFor::FloatLiteral(_)
        | HirExprKindFor::BoolLiteral(_)
        | HirExprKindFor::StringLiteral(_)
        | HirExprKindFor::CharLiteral(_)
        | HirExprKindFor::Unit
        | HirExprKindFor::Var(_)
        | HirExprKindFor::ResolvedVar(_) => {}
    }
}
