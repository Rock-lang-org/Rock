use std::collections::HashSet;

use crate::diagnostic::DiagnosticCode;
use crate::ids::DefId;

use super::hir_types::{HirBlock, HirCallTarget, HirExpr, HirExprKind, HirStmt, HirVarTarget};
use super::Monomorphizer;

impl Monomorphizer {
    pub(super) fn materialize_registered_body_edges(&mut self) {
        let mut processed = std::collections::HashSet::new();
        loop {
            let bodies = self
                .instances
                .records()
                .filter(|record| !processed.contains(&record.id))
                .filter_map(|record| {
                    self.instances
                        .pre_mir_body(record.id)
                        .cloned()
                        .map(|body| (record.id, body))
                })
                .collect::<Vec<_>>();
            if bodies.is_empty() {
                break;
            }

            for (instance_id, body) in bodies {
                let processed_body = self.process_function(body);
                self.instances
                    .replace_pre_mir_body(instance_id, processed_body);
                processed.insert(instance_id);
            }
        }
    }

    pub(super) fn validate_materialized_body_edges(&mut self) {
        let bodies = self
            .instances
            .records()
            .filter_map(|record| {
                self.instances
                    .pre_mir_body(record.id)
                    .cloned()
                    .map(|body| (record.symbols.source_name.clone(), body))
            })
            .collect::<Vec<_>>();
        let method_ids = self
            .generic_impls
            .values()
            .chain(self.trait_impls.values().flat_map(|impls| impls.iter()))
            .flat_map(|imp| imp.methods.values().map(|method| method.id))
            .collect::<HashSet<_>>();

        for (owner, body) in bodies {
            self.validate_materialized_block(&owner, &body.body, &method_ids);
        }
    }

    fn validate_materialized_block(
        &mut self,
        owner: &str,
        block: &HirBlock,
        method_ids: &HashSet<DefId>,
    ) {
        for stmt in &block.stmts {
            match stmt {
                HirStmt::Let { value, .. }
                | HirStmt::Expr(value)
                | HirStmt::Return(Some(value))
                | HirStmt::Break(Some(value)) => {
                    self.validate_materialized_expr(owner, value, method_ids)
                }
                HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
            }
        }
    }

    fn validate_materialized_expr(
        &mut self,
        owner: &str,
        expr: &HirExpr,
        method_ids: &HashSet<DefId>,
    ) {
        match &expr.kind {
            HirExprKind::MethodCall(receiver, name, args, _, _target) => {
                self.validate_materialized_expr(owner, receiver, method_ids);
                for arg in args {
                    self.validate_materialized_expr(owner, arg, method_ids);
                }
                self.diagnostics.push(
                    crate::diagnostic::Diagnostic::for_internal(
                    format!(
                        "post-monomorphization body '{owner}' retains unmaterialized method call '{name}'"
                    ),
                    )
                    .with_code(DiagnosticCode::Mono),
                );
            }
            HirExprKind::Call(callee, args, target) => {
                self.validate_materialized_expr(owner, callee, method_ids);
                for arg in args {
                    self.validate_materialized_expr(owner, arg, method_ids);
                }
                let unresolved_static = matches!(target, Some(HirCallTarget::StaticMethod(_)));
                let method_def_target = match target {
                    Some(HirCallTarget::Function(id)) => method_ids.contains(id),
                    _ => false,
                };
                let unresolved_field_call =
                    matches!(&callee.kind, HirExprKind::FieldAccess(_, _, None));
                if unresolved_static || method_def_target || unresolved_field_call {
                    self.diagnostics.push(
                        crate::diagnostic::Diagnostic::for_internal(format!(
                            "post-monomorphization body '{owner}' retains an unmaterialized method call edge"
                        ))
                        .with_code(DiagnosticCode::Mono),
                    );
                }
            }
            HirExprKind::Try {
                expr: carrier,
                branch_target,
                from_residual_target,
                ..
            } => {
                self.validate_materialized_expr(owner, carrier, method_ids);
                if !matches!(branch_target, Some(HirCallTarget::Instance(_)))
                    || !matches!(from_residual_target, HirCallTarget::Instance(_))
                {
                    self.diagnostics.push(
                        crate::diagnostic::Diagnostic::for_internal(format!(
                            "post-monomorphization body '{owner}' retains an unmaterialized Try call edge"
                        ))
                        .with_code(DiagnosticCode::Mono),
                    );
                }
            }
            HirExprKind::FieldAccess(base, _, _)
            | HirExprKind::TupleIndex(base, _)
            | HirExprKind::UnaryOp(_, base)
            | HirExprKind::Ref(_, base)
            | HirExprKind::Deref(base)
            | HirExprKind::Cast(base, _) => {
                self.validate_materialized_expr(owner, base, method_ids)
            }
            HirExprKind::ArrayLiteral(values) | HirExprKind::TupleLiteral(values) => {
                for value in values {
                    self.validate_materialized_expr(owner, value, method_ids);
                }
            }
            HirExprKind::ArrayRepeat(value, _) => {
                self.validate_materialized_expr(owner, value, method_ids);
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    self.validate_materialized_expr(owner, &field.value, method_ids);
                }
            }
            HirExprKind::EnumVariant(_, _, args, _) | HirExprKind::Intrinsic { args, .. } => {
                for arg in args {
                    self.validate_materialized_expr(owner, arg, method_ids);
                }
            }
            HirExprKind::BinOp(_, left, right) | HirExprKind::Assign(left, right) => {
                self.validate_materialized_expr(owner, left, method_ids);
                self.validate_materialized_expr(owner, right, method_ids);
            }
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.validate_materialized_expr(owner, condition, method_ids);
                self.validate_materialized_block(owner, then_branch, method_ids);
                if let Some(else_branch) = else_branch {
                    self.validate_materialized_block(owner, else_branch, method_ids);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.validate_materialized_expr(owner, scrutinee, method_ids);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.validate_materialized_expr(owner, guard, method_ids);
                    }
                    self.validate_materialized_block(owner, &arm.body, method_ids);
                }
            }
            HirExprKind::While { condition, body } => {
                self.validate_materialized_expr(owner, condition, method_ids);
                self.validate_materialized_block(owner, body, method_ids);
            }
            HirExprKind::For { iter, body, .. } => {
                self.validate_materialized_expr(owner, iter, method_ids);
                self.validate_materialized_block(owner, body, method_ids);
            }
            HirExprKind::Lambda { body, .. }
            | HirExprKind::Block(body)
            | HirExprKind::Loop(body)
            | HirExprKind::UnsafeBlock(body) => {
                self.validate_materialized_block(owner, body, method_ids)
            }
            HirExprKind::ResolvedVar(reference) => {
                if matches!(reference.target, HirVarTarget::Function(id) if method_ids.contains(&id))
                {
                    self.diagnostics.push(
                        crate::diagnostic::Diagnostic::for_internal(format!(
                            "post-monomorphization body '{owner}' retains an unmaterialized static method value"
                        ))
                        .with_code(DiagnosticCode::Mono),
                    );
                }
            }
            HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit
            | HirExprKind::Var(_) => {}
        }
    }
}
