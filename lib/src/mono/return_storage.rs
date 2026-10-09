//! Callable-owned return storage; normal block values are a separate join.

use super::hir_types::{HirBlock, HirExpr, HirExprKind, HirStmt};
use super::Monomorphizer;
use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::lexer::Span;
use crate::types::Type;

#[derive(Default)]
struct Flow {
    normal: bool,
    breaks: bool,
    continues: bool,
}

impl Flow {
    fn normal() -> Self {
        Self { normal: true, ..Self::default() }
    }

    fn join(&mut self, other: Self) {
        self.normal |= other.normal;
        self.breaks |= other.breaks;
        self.continues |= other.continues;
    }

    fn then(&mut self, other: Self) {
        self.normal = other.normal;
        self.breaks |= other.breaks;
        self.continues |= other.continues;
    }
}

struct Returned<'a> {
    ty: &'a Type,
    span: Option<&'a Span>,
}

fn block_flow<'a>(block: &'a HirBlock, returns: &mut Vec<Returned<'a>>) -> Flow {
    let mut flow = Flow::normal();
    for statement in &block.stmts {
        if !flow.normal {
            break;
        }
        let next = match statement {
            HirStmt::Return(value) => {
                let mut next = value.as_ref()
                    .map(|value| expression_flow(value, returns))
                    .unwrap_or_else(Flow::normal);
                if next.normal {
                    returns.push(Returned {
                        ty: value.as_ref().map(|value| &value.ty).unwrap_or(&Type::Unit),
                        span: value.as_ref().map(|value| &value.span),
                    });
                }
                next.normal = false;
                next
            }
            HirStmt::Break(value) => {
                let mut next = value.as_ref()
                    .map(|value| expression_flow(value, returns))
                    .unwrap_or_else(Flow::normal);
                next.breaks |= next.normal;
                next.normal = false;
                next
            }
            HirStmt::Continue => Flow { continues: true, ..Flow::default() },
            HirStmt::Let { value, .. } | HirStmt::Expr(value) => {
                expression_flow(value, returns)
            }
        };
        flow.then(next);
    }
    flow
}

fn expression_flow<'a>(expr: &'a HirExpr, returns: &mut Vec<Returned<'a>>) -> Flow {
    let mut flow = match &expr.kind {
        // The lambda value continues; its body has a different return owner.
        HirExprKind::Lambda { .. } => Flow::normal(),
        HirExprKind::If { condition, then_branch, else_branch } => {
            let mut flow = expression_flow(condition, returns);
            if flow.normal {
                let mut branches = block_flow(then_branch, returns);
                branches.join(else_branch.as_ref()
                    .map(|branch| block_flow(branch, returns))
                    .unwrap_or_else(Flow::normal));
                flow.then(branches);
            }
            flow
        }
        HirExprKind::Match { scrutinee, arms } => {
            let mut flow = expression_flow(scrutinee, returns);
            if flow.normal {
                let mut branches = Flow::default();
                for arm in arms {
                    let mut arm_flow = arm.guard.as_ref()
                        .map(|guard| expression_flow(guard, returns))
                        .unwrap_or_else(Flow::normal);
                    if arm_flow.normal {
                        arm_flow.then(block_flow(&arm.body, returns));
                    }
                    branches.join(arm_flow);
                }
                flow.then(branches);
            }
            flow
        }
        HirExprKind::Loop(body) => {
            let body = block_flow(body, returns);
            Flow { normal: body.breaks, ..Flow::default() }
        }
        HirExprKind::While { condition, body } => {
            let condition = expression_flow(condition, returns);
            if condition.normal {
                block_flow(body, returns);
            }
            condition
        }
        HirExprKind::For { iter, body, .. } => {
            let iter = expression_flow(iter, returns);
            if iter.normal {
                block_flow(body, returns);
            }
            iter
        }
        _ => {
            let mut flow = Flow::normal();
            crate::hir::visit::visit_expr_children(expr, &mut |child| {
                if flow.normal {
                    flow.then(match child {
                        crate::hir::visit::HirChild::Expr(expr) => expression_flow(expr, returns),
                        crate::hir::visit::HirChild::Block(block) => block_flow(block, returns),
                    });
                }
            });
            flow
        }
    };
    flow.normal &= expr.ty != Type::Never;
    flow
}

pub(super) fn completes_normally(block: &HirBlock) -> bool {
    block_flow(block, &mut Vec::new()).normal
}

impl Monomorphizer {
    // A failed agreement never commits a partially refined return type.
    pub(super) fn agree_return_storage(
        &mut self,
        contract: &Type,
        declared: Option<&Type>,
        body: &HirBlock,
    ) -> Option<Type> {
        let mut returns = Vec::new();
        if block_flow(body, &mut returns).normal {
            returns.push(Returned {
                ty: &body.ty,
                span: body.stmts.last().and_then(|statement| match statement {
                    HirStmt::Expr(value) => Some(&value.span),
                    _ => None,
                }),
            });
        }
        let measured = super::process::has_measured_storage(contract)
            || returns.iter().any(|value| super::process::has_measured_storage(value.ty));
        let mut candidate: Option<Type> = None;
        let mut first_span = None;
        for value in returns {
            let refined = match declared {
                Some(declared) => crate::type_services::callable::declared_value_representation(
                    declared, contract, value.ty,
                ),
                None => crate::type_services::callable::value_representation(contract, value.ty),
            };
            if refined.is_none() || candidate.as_ref().is_some_and(|first| Some(first) != refined.as_ref()) {
                // Unrepresented semantic/borrow metadata remains inference's
                // responsibility; it must not choose one alternative's header.
                if !measured {
                    return None;
                }
                let message = "return paths require an agreed field representation".to_string();
                let mut diagnostic = match value.span.or(first_span) {
                    Some(span) => Diagnostic::new(message, span.clone()),
                    None => Diagnostic::for_internal(message),
                }.with_code(DiagnosticCode::Mono);
                if let (Some(first), Some(current)) = (first_span, value.span) {
                    if first != current {
                        diagnostic = diagnostic.with_label(
                            "another return requires different callable or field storage".into(),
                            first.clone(),
                        );
                    }
                }
                self.diagnostics.push(diagnostic);
                return None;
            }
            if candidate.is_none() {
                candidate = refined;
                first_span = value.span;
            }
        }
        candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CallableKind, CaptureKind, FunctionCapture, FunctionSafety};

    fn value(ty: Type) -> HirExpr {
        HirExpr { kind: HirExprKind::Unit, ty, span: Span::test() }
    }

    fn captured(ty: Type) -> Type {
        Type::function_with_metadata(
            vec![], Type::I64, FunctionSafety::Safe, CallableKind::FnOnce,
            vec![FunctionCapture::new(CaptureKind::Move, ty)],
        )
    }

    fn branch(statement: HirStmt) -> HirExpr {
        HirExpr {
            kind: HirExprKind::If {
                condition: Box::new(value(Type::Bool)),
                then_branch: HirBlock { stmts: vec![statement], ty: Type::Never },
                else_branch: None,
            },
            ty: Type::Unit,
            span: Span::test(),
        }
    }

    #[test]
    fn all_reachable_returns_are_checked_atomically_in_both_orders() {
        let plain = Type::function(vec![], Type::I64);
        let storage = captured(Type::Array(Box::new(Type::I64), 1));
        for reverse in [false, true] {
            let (early, tail) = if reverse { (&plain, &storage) } else { (&storage, &plain) };
            let body = HirBlock {
                stmts: vec![
                    HirStmt::Expr(branch(HirStmt::Return(Some(value(early.clone()))))),
                    HirStmt::Expr(value(tail.clone())),
                ],
                ty: tail.clone(),
            };
            let mut mono = Monomorphizer::new();
            assert_eq!(mono.agree_return_storage(&plain, None, &body), None);
            assert_eq!(mono.diagnostics.0.len(), 1);
            assert_eq!(body.ty, *tail, "agreement must not partially mutate the body");
        }
    }

    #[test]
    fn failed_transport_and_same_width_environments_reject() {
        let plain = Type::function(vec![], Type::I64);
        for contract in [plain.clone(), Type::function(vec![], Type::Bool)] {
            let body = HirBlock {
                stmts: vec![
                    HirStmt::Expr(branch(HirStmt::Return(Some(value(captured(Type::I64)))))),
                    HirStmt::Return(Some(value(captured(Type::Tuple(vec![Type::I64]))))),
                ],
                ty: Type::Never,
            };
            let mut mono = Monomorphizer::new();
            assert_eq!(mono.agree_return_storage(&contract, None, &body), None);
            assert_eq!(mono.diagnostics.0.len(), 1);
        }
    }

    #[test]
    fn selected_generic_result_cannot_be_refined_from_an_actual_environment() {
        let declared = Type::Generic(crate::types::GenericParamId {
            owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(1)),
            index: 0,
        });
        let selected = Type::function(vec![], Type::I64);
        let actual = captured(Type::I64);
        let body = HirBlock {
            stmts: vec![HirStmt::Return(Some(value(actual)))], ty: Type::Never,
        };
        let mut mono = Monomorphizer::new();
        assert_eq!(mono.agree_return_storage(&selected, Some(&declared), &body), None);
        assert_eq!(mono.diagnostics.0.len(), 1);
    }

    #[test]
    fn explicit_returns_ignore_dead_tails_and_lambda_return_targets() {
        let plain = Type::function(vec![], Type::I64);
        let storage = captured(Type::I64);
        let body = HirBlock {
            stmts: vec![
                HirStmt::Return(Some(value(storage.clone()))),
                HirStmt::Return(Some(value(plain.clone()))),
            ],
            ty: Type::Never,
        };
        let mut mono = Monomorphizer::new();
        assert_eq!(mono.agree_return_storage(&plain, None, &body), Some(storage.clone()));
        let lambda = HirExpr {
            kind: HirExprKind::Lambda {
                params: vec![], captures: vec![],
                body: HirBlock { stmts: vec![HirStmt::Return(Some(value(Type::Bool)))], ty: Type::Never },
            },
            ty: Type::function(vec![], Type::Bool),
            span: Span::test(),
        };
        let body = HirBlock {
            stmts: vec![HirStmt::Expr(lambda), HirStmt::Expr(value(storage.clone()))],
            ty: storage.clone(),
        };
        assert_eq!(mono.agree_return_storage(&plain, None, &body), Some(storage));
        assert!(mono.diagnostics.0.is_empty());
    }

    #[test]
    fn evaluated_arguments_stop_after_return_and_never_paths_supply_no_value() {
        let storage = captured(Type::I64);
        let first = HirExpr {
            kind: HirExprKind::UnsafeBlock(HirBlock {
                stmts: vec![HirStmt::Return(Some(value(storage.clone())))], ty: Type::Never,
            }),
            ty: Type::Never,
            span: Span::test(),
        };
        let expr = HirExpr {
            kind: HirExprKind::TupleLiteral(vec![first, branch(HirStmt::Return(Some(value(Type::Bool))))]),
            ty: Type::Never,
            span: Span::test(),
        };
        let body = HirBlock { stmts: vec![HirStmt::Expr(expr)], ty: Type::Never };
        let mut mono = Monomorphizer::new();
        assert_eq!(mono.agree_return_storage(&Type::function(vec![], Type::I64), None, &body), Some(storage));
        let body = HirBlock { stmts: vec![HirStmt::Expr(value(Type::Never))], ty: Type::Never };
        assert_eq!(mono.agree_return_storage(&Type::Unit, None, &body), None);
        assert!(mono.diagnostics.0.is_empty());
    }

    #[test]
    fn nested_loops_own_breaks_but_not_callable_returns() {
        let inner = HirExpr {
            kind: HirExprKind::Loop(HirBlock { stmts: vec![HirStmt::Break(None)], ty: Type::Never }),
            ty: Type::Unit,
            span: Span::test(),
        };
        let outer = HirBlock { stmts: vec![HirStmt::Expr(inner), HirStmt::Continue], ty: Type::Never };
        let mut returns = Vec::new();
        let flow = block_flow(&outer, &mut returns);
        assert!(!flow.normal && !flow.breaks && flow.continues);
        assert!(returns.is_empty());
        let body = HirBlock {
            stmts: vec![HirStmt::Expr(branch(HirStmt::Return(Some(value(Type::I64))))), HirStmt::Break(None)],
            ty: Type::Never,
        };
        let flow = block_flow(&body, &mut returns);
        assert!(!flow.normal && flow.breaks);
        assert_eq!(returns.len(), 1);
        assert_eq!(*returns[0].ty, Type::I64);
    }

    #[test]
    fn initializer_guard_and_call_argument_returns_keep_their_own_spans() {
        let storage = captured(Type::I64);
        let mut returned = value(storage.clone());
        returned.span.start = 17;
        returned.span.end = 23;
        let exit = HirExpr {
            kind: HirExprKind::UnsafeBlock(HirBlock {
                stmts: vec![HirStmt::Return(Some(returned.clone()))], ty: Type::Never,
            }),
            ty: Type::Never,
            span: Span::test(),
        };
        let guard = HirExpr {
            kind: HirExprKind::Match {
                scrutinee: Box::new(value(Type::Bool)),
                arms: vec![super::super::hir_types::HirMatchArm {
                    pattern: crate::hir::HirPattern::Wildcard,
                    guard: Some(exit.clone()),
                    body: HirBlock {
                        stmts: vec![HirStmt::Return(Some(value(Type::Bool)))], ty: Type::Never,
                    },
                }],
            },
            ty: Type::Never,
            span: Span::test(),
        };
        let call = HirExpr {
            kind: HirExprKind::Call(
                Box::new(value(Type::function(vec![Type::I64], Type::Unit))),
                vec![exit.clone(), branch(HirStmt::Return(Some(value(Type::Bool))))],
                None,
            ),
            ty: Type::Never,
            span: Span::test(),
        };
        for statement in [
            HirStmt::Let {
                name: "uninitialized".into(), local_id: crate::ids::HirLocalId(0),
                ty: Type::Never, value: exit, mutable: false,
            },
            HirStmt::Expr(guard),
            HirStmt::Expr(call),
        ] {
            let body = HirBlock {
                stmts: vec![statement, HirStmt::Return(Some(value(Type::Bool)))], ty: Type::Never,
            };
            let mut exits = Vec::new();
            assert!(!block_flow(&body, &mut exits).normal);
            assert_eq!(exits.len(), 1);
            assert_eq!(exits[0].ty, &storage);
            assert_eq!(exits[0].span, Some(&returned.span));
        }
    }
}
