use std::collections::HashMap;

use crate::hir::*;
use crate::ids::{DefId, HirLocalId};
use crate::types::Type;

use super::mechanisms::{truncate, width};
use super::summary::{ArithmeticSummary, ArithmeticType};

pub(super) fn scalar(ty: &Type) -> bool {
    width(ty).is_some()
        || *ty == Type::Bool
        || matches!(ty, Type::Reference { mutable: false, inner } if width(inner).is_some() || **inner == Type::Bool)
}

pub(super) struct SummaryContext<'a, P: HirPhase> {
    methods: &'a HashMap<DefId, &'a HirFunctionFor<P>>,
    summaries: &'a HashMap<DefId, ArithmeticSummary>,
}

/// Build the same bounded expression model for local diagnostics and exports.
/// Unresolved/recursive calls never acquire summaries; iteration is bounded by
/// the maximum admitted summary depth and independent of declaration order.
pub(crate) fn method_summaries<P: HirPhase>(
    program: &HirProgramFor<P>,
    mut summaries: HashMap<DefId, ArithmeticSummary>,
) -> HashMap<DefId, ArithmeticSummary> {
    let methods = program
        .impls
        .values()
        .flat_map(|imp| imp.methods.values())
        .map(|method| (method.id, method))
        .collect::<HashMap<_, _>>();
    let mut ids = methods.keys().copied().collect::<Vec<_>>();
    ids.sort_unstable();
    for _ in 0..32 {
        let mut added = false;
        for id in &ids {
            if summaries.contains_key(id) {
                continue;
            }
            let context = SummaryContext {
                methods: &methods,
                summaries: &summaries,
            };
            if let Some(summary) = ArithmeticSummary::from_function(methods[id], &context) {
                summaries.insert(*id, summary);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    summaries
}

fn substitute_arguments(
    summary: &ArithmeticSummary,
    args: &[ArithmeticSummary],
    fuel: &mut usize,
    depth: usize,
) -> Option<ArithmeticSummary> {
    if *fuel == 0 || depth >= 32 {
        return None;
    }
    *fuel -= 1;
    let inner =
        |value, fuel: &mut usize| substitute_arguments(value, args, fuel, depth + 1).map(Box::new);
    Some(match summary {
        ArithmeticSummary::Argument(index) => {
            let value = args.get(*index)?;
            value.charge(fuel)?;
            value.clone()
        }
        ArithmeticSummary::Integer(value) => ArithmeticSummary::Integer(*value),
        ArithmeticSummary::Bool(value) => ArithmeticSummary::Bool(*value),
        ArithmeticSummary::Ref(value) => ArithmeticSummary::Ref(inner(value, fuel)?),
        ArithmeticSummary::Deref(value) => ArithmeticSummary::Deref(inner(value, fuel)?),
        ArithmeticSummary::Cast(value, ty) => ArithmeticSummary::Cast(inner(value, fuel)?, *ty),
        ArithmeticSummary::TupleIndex(value, index) => {
            ArithmeticSummary::TupleIndex(inner(value, fuel)?, *index)
        }
        ArithmeticSummary::If(condition, yes, no) => {
            ArithmeticSummary::If(inner(condition, fuel)?, inner(yes, fuel)?, inner(no, fuel)?)
        }
        ArithmeticSummary::Intrinsic(name, ty, values) => ArithmeticSummary::Intrinsic(
            name.clone(),
            *ty,
            values
                .iter()
                .map(|value| substitute_arguments(value, args, fuel, depth + 1))
                .collect::<Option<_>>()?,
        ),
        ArithmeticSummary::Sequence(values) => ArithmeticSummary::Sequence(
            values
                .iter()
                .map(|value| substitute_arguments(value, args, fuel, depth + 1))
                .collect::<Option<_>>()?,
        ),
    })
}

pub(super) fn summarize_block<P: HirPhase>(
    block: &HirBlockFor<P>,
    mut env: HashMap<HirLocalId, ArithmeticSummary>,
    context: &SummaryContext<'_, P>,
    fuel: &mut usize,
    depth: usize,
) -> Option<ArithmeticSummary> {
    let mut result = None;
    let mut sequence = Vec::new();
    for stmt in &block.stmts {
        match stmt {
            HirStmtFor::Let {
                local_id,
                value,
                mutable: false,
                ..
            } => {
                let value = summarize(value, &env, context, fuel, depth)?;
                sequence.push(value.clone());
                env.insert(*local_id, value);
                result = None;
            }
            HirStmtFor::Expr(expr) => {
                let value = summarize(expr, &env, context, fuel, depth)?;
                sequence.push(value.clone());
                result = Some(value);
            }
            _ => return None,
        }
    }
    result.map(|value| {
        if sequence.len() == 1 {
            value
        } else {
            ArithmeticSummary::Sequence(sequence)
        }
    })
}

fn summarize<P: HirPhase>(
    expr: &HirExprFor<P>,
    env: &HashMap<HirLocalId, ArithmeticSummary>,
    context: &SummaryContext<'_, P>,
    fuel: &mut usize,
    depth: usize,
) -> Option<ArithmeticSummary> {
    if *fuel == 0 || depth >= 32 {
        return None;
    }
    *fuel -= 1;
    let inner =
        |expr, fuel: &mut usize| summarize(expr, env, context, fuel, depth + 1).map(Box::new);
    Some(match &expr.kind {
        HirExprKindFor::IntLiteral(value) if width(&expr.ty).is_some() => {
            ArithmeticSummary::Integer(truncate(*value as i128, &expr.ty))
        }
        HirExprKindFor::BoolLiteral(value) => ArithmeticSummary::Bool(*value),
        HirExprKindFor::ResolvedVar(reference) => match reference.target {
            HirVarTarget::Local(id) => {
                let summary = env.get(&id)?;
                summary.charge(fuel)?;
                summary.clone()
            }
            _ => return None,
        },
        HirExprKindFor::Ref(false, value) => ArithmeticSummary::Ref(inner(value, fuel)?),
        HirExprKindFor::Deref(value)
            if matches!(value.ty, Type::Reference { mutable: false, .. }) =>
        {
            ArithmeticSummary::Deref(inner(value, fuel)?)
        }
        HirExprKindFor::Cast(value, ty) if width(ty).is_some() => {
            ArithmeticSummary::Cast(inner(value, fuel)?, ArithmeticType::from_type(ty)?)
        }
        HirExprKindFor::TupleIndex(value, index) => {
            ArithmeticSummary::TupleIndex(inner(value, fuel)?, *index)
        }
        HirExprKindFor::Block(block) => {
            summarize_block(block, env.clone(), context, fuel, depth + 1)?
        }
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch: Some(no),
        } => ArithmeticSummary::If(
            inner(condition, fuel)?,
            Box::new(summarize_block(
                then_branch,
                env.clone(),
                context,
                fuel,
                depth + 1,
            )?),
            Box::new(summarize_block(no, env.clone(), context, fuel, depth + 1)?),
        ),
        HirExprKindFor::Intrinsic { name, args } => ArithmeticSummary::Intrinsic(
            name.clone(),
            ArithmeticType::from_type(&args.first()?.ty)?,
            args.iter()
                .map(|arg| summarize(arg, env, context, fuel, depth + 1))
                .collect::<Option<_>>()?,
        ),
        HirExprKindFor::MethodCall(receiver, _, args, _, authority) => {
            let target = P::method_authority(authority)?;
            let HirSelectedMethodTarget::ImplMethod { method_id, .. } = target.target else {
                return None;
            };
            let method = context.methods.get(&method_id)?;
            if method.is_unsafe || method.params.len() != args.len() + 1 {
                return None;
            }
            let summary = context.summaries.get(&method_id)?;
            let mut value = summarize(receiver, env, context, fuel, depth + 1)?;
            if matches!(method.params[0].ty, Type::Reference { mutable: false, .. })
                && !matches!(receiver.ty, Type::Reference { .. })
            {
                value = ArithmeticSummary::Ref(Box::new(value));
            }
            let mut values = vec![value];
            for arg in args {
                values.push(summarize(arg, env, context, fuel, depth + 1)?);
            }
            let body = substitute_arguments(summary, &values, fuel, depth + 1)?;
            // Arguments execute even when the callee ignores them.
            values.push(body);
            ArithmeticSummary::Sequence(values)
        }
        _ => return None,
    })
}
