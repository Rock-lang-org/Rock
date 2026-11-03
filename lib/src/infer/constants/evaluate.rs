use crate::hir::*;
use crate::types::Type;

use super::mechanisms::{mechanism, mechanism_arity, truncate, width};
use super::validate::Check;
use super::{Constant, Environment, Evaluation};

pub(super) fn evaluate(
    program: &Check<'_>,
    expr: &HirExpr,
    env: &Environment,
    fuel: &mut usize,
    depth: usize,
) -> Evaluation {
    if *fuel == 0 || depth >= 32 {
        return Ok(None);
    }
    *fuel -= 1;
    let eval = |expr: &HirExpr, fuel: &mut usize| evaluate(program, expr, env, fuel, depth + 1);
    Ok(match &expr.kind {
        HirExprKind::IntLiteral(value) => {
            width(&expr.ty).map(|_| Constant::Integer(truncate(*value as i128, &expr.ty)))
        }
        HirExprKind::BoolLiteral(value) => Some(Constant::Bool(*value)),
        HirExprKind::ResolvedVar(reference) => match reference.target {
            HirVarTarget::Local(id) => env.get(&id).cloned(),
            _ => None,
        },
        HirExprKind::Cast(inner, ty) => match eval(inner, fuel)? {
            Some(Constant::Integer(value)) if width(ty).is_some() => {
                Some(Constant::Integer(truncate(value, ty)))
            }
            _ => None,
        },
        HirExprKind::Ref(false, inner) => {
            eval(inner, fuel)?.map(|value| Constant::Ref(Box::new(value)))
        }
        HirExprKind::Deref(inner) => match eval(inner, fuel)? {
            Some(Constant::Ref(value)) => Some(*value),
            _ => None,
        },
        HirExprKind::TupleIndex(inner, index) => match eval(inner, fuel)? {
            Some(Constant::Tuple(values)) => values.get(*index as usize).cloned(),
            _ => None,
        },
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => match eval(condition, fuel)? {
            Some(Constant::Bool(true)) => {
                evaluate_block(program, then_branch, env.clone(), fuel, depth + 1)?
            }
            Some(Constant::Bool(false)) => match else_branch {
                Some(block) => evaluate_block(program, block, env.clone(), fuel, depth + 1)?,
                None => None,
            },
            _ => None,
        },
        HirExprKind::Block(block) => evaluate_block(program, block, env.clone(), fuel, depth + 1)?,
        HirExprKind::MethodCall(receiver, _, args, _, Some(target)) => {
            let HirSelectedMethodTarget::ImplMethod {
                impl_id, method_id, ..
            } = target.target
            else {
                return Ok(None);
            };
            let Some(method) = program
                .impls
                .get(&impl_id)
                .and_then(|imp| imp.methods.values().find(|method| method.id == method_id))
            else {
                return Ok(None);
            };
            if method.is_unsafe || method.params.len() != args.len() + 1 {
                return Ok(None);
            }
            let Some(mut receiver) = eval(receiver, fuel)? else {
                return Ok(None);
            };
            if matches!(method.params[0].ty, Type::Reference { mutable: false, .. })
                && !matches!(receiver, Constant::Ref(_))
            {
                receiver = Constant::Ref(Box::new(receiver));
            }
            let mut values = vec![receiver];
            for arg in args {
                let Some(value) = eval(arg, fuel)? else {
                    return Ok(None);
                };
                values.push(value);
            }
            match program.summaries.get(&method_id) {
                Some(summary) if summary.charge(fuel).is_some() => summary.evaluate(&values)?,
                _ => None,
            }
        }
        HirExprKind::Intrinsic { name, args } => {
            let Some(first) = args.first() else {
                return Ok(None);
            };
            if mechanism_arity(name, &first.ty) != Some(args.len()) {
                return Ok(None);
            }
            let mut values = Vec::new();
            for arg in args {
                let Some(value) = eval(arg, fuel)? else {
                    return Ok(None);
                };
                values.push(value);
            }
            mechanism(name, &first.ty, &values)?
        }
        // Do not execute arbitrary functions, mutable storage, loops or unsafe code.
        _ => None,
    })
}

fn evaluate_block(
    program: &Check<'_>,
    block: &HirBlock,
    mut env: Environment,
    fuel: &mut usize,
    depth: usize,
) -> Evaluation {
    let mut result = None;
    for stmt in &block.stmts {
        match stmt {
            HirStmt::Let {
                local_id,
                value,
                mutable: false,
                ..
            } => {
                let Some(value) = evaluate(program, value, &env, fuel, depth)? else {
                    return Ok(None);
                };
                env.insert(*local_id, value);
                result = None;
            }
            HirStmt::Expr(expr) => {
                result = evaluate(program, expr, &env, fuel, depth)?;
                if result.is_none() {
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        }
    }
    Ok(result)
}
