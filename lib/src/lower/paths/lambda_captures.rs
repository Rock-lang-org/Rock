//! Lambda capture collection.

use crate::hir::*;
use crate::lower::Lowerer;
use crate::types::Type;

impl Lowerer {
    pub(crate) fn collect_lambda_captures(
        &self,
        body: &HirBlock,
        params: &[HirParam],
    ) -> Vec<HirClosureCapture> {
        let mut defined = std::collections::HashSet::new();
        for param in params {
            defined.insert(param.name.clone());
        }

        let mut used = Vec::new();
        let mut capture_kinds = std::collections::HashMap::new();
        self.collect_lambda_captures_block(
            body,
            &mut defined,
            &mut used,
            &mut capture_kinds,
            HirClosureCaptureKind::SharedBorrow,
        );

        let mut captures = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for name in used {
            if !seen.insert(name.clone()) {
                continue;
            }

            if let Some(binding) = self.scope.lookup(&name) {
                if binding.is_top_level && matches!(&binding.ty, Type::Function { .. }) {
                    continue;
                }

                let Some(local_id) = binding.local_id else {
                    continue;
                };
                let kind = capture_kinds.remove(&name).unwrap_or_else(|| {
                    if binding.mutable {
                        HirClosureCaptureKind::MutableBorrow
                    } else {
                        HirClosureCaptureKind::SharedBorrow
                    }
                });
                captures.push(HirClosureCapture {
                    name,
                    local_id,
                    kind,
                    mutable: binding.mutable,
                    ty: binding.ty.clone(),
                });
            }
        }

        captures
    }

    fn define_capture_pattern(
        pattern: &HirPattern,
        defined: &mut std::collections::HashSet<String>,
    ) {
        match pattern {
            HirPattern::Binding { name, .. } => {
                defined.insert(name.clone());
            }
            HirPattern::Tuple(patterns)
            | HirPattern::Or(patterns)
            | HirPattern::Enum(_, _, _, patterns) => {
                for pattern in patterns {
                    Self::define_capture_pattern(pattern, defined);
                }
            }
            HirPattern::Struct(_, _, _, fields) => {
                for field in fields {
                    Self::define_capture_pattern(&field.pattern, defined);
                }
            }
            HirPattern::Wildcard | HirPattern::Literal(_) => {}
        }
    }

    fn collect_lambda_captures_block(
        &self,
        block: &HirBlock,
        defined: &mut std::collections::HashSet<String>,
        used: &mut Vec<String>,
        capture_kinds: &mut std::collections::HashMap<String, HirClosureCaptureKind>,
        current_kind: HirClosureCaptureKind,
    ) {
        for stmt in &block.stmts {
            match stmt {
                HirStmt::Let { name, value, .. } => {
                    self.collect_lambda_captures_expr(
                        value,
                        defined,
                        used,
                        capture_kinds,
                        Self::lambda_capture_kind_for_value(
                            &self.engine.resolve(&value.ty),
                            current_kind,
                        ),
                    );
                    defined.insert(name.clone());
                }
                HirStmt::Expr(expr) => self.collect_lambda_captures_expr(
                    expr,
                    defined,
                    used,
                    capture_kinds,
                    Self::lambda_capture_kind_for_value(
                        &self.engine.resolve(&expr.ty),
                        current_kind,
                    ),
                ),
                HirStmt::Return(Some(expr)) => self.collect_lambda_captures_expr(
                    expr,
                    defined,
                    used,
                    capture_kinds,
                    Self::lambda_capture_kind_for_value(
                        &self.engine.resolve(&expr.ty),
                        current_kind,
                    ),
                ),
                HirStmt::Break(Some(expr)) => self.collect_lambda_captures_expr(
                    expr,
                    defined,
                    used,
                    capture_kinds,
                    Self::lambda_capture_kind_for_value(
                        &self.engine.resolve(&expr.ty),
                        current_kind,
                    ),
                ),
                _ => {}
            }
        }
    }

    fn collect_lambda_captures_expr(
        &self,
        expr: &HirExpr,
        defined: &mut std::collections::HashSet<String>,
        used: &mut Vec<String>,
        capture_kinds: &mut std::collections::HashMap<String, HirClosureCaptureKind>,
        current_kind: HirClosureCaptureKind,
    ) {
        match &expr.kind {
            HirExprKind::Var(name) => {
                self.record_lambda_capture(name, current_kind, defined, used, capture_kinds);
            }
            HirExprKind::ResolvedVar(reference)
                if matches!(reference.target, HirVarTarget::Local(_)) =>
            {
                self.record_lambda_capture(
                    &reference.name,
                    current_kind,
                    defined,
                    used,
                    capture_kinds,
                );
            }
            HirExprKind::ResolvedVar(_) => {}
            HirExprKind::FieldAccess(base, _, _) => {
                self.collect_lambda_captures_expr(base, defined, used, capture_kinds, current_kind)
            }
            HirExprKind::TupleIndex(base, _) => {
                self.collect_lambda_captures_expr(base, defined, used, capture_kinds, current_kind)
            }
            HirExprKind::BinOp(_, lhs, rhs) => {
                self.collect_lambda_captures_expr(lhs, defined, used, capture_kinds, current_kind);
                self.collect_lambda_captures_expr(rhs, defined, used, capture_kinds, current_kind);
            }
            HirExprKind::UnaryOp(_, inner)
            | HirExprKind::Deref(inner)
            | HirExprKind::Cast(inner, _) => {
                self.collect_lambda_captures_expr(
                    inner,
                    defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
            }
            HirExprKind::Ref(mutable, inner) => {
                let ref_kind = if *mutable {
                    HirClosureCaptureKind::MutableBorrow
                } else {
                    HirClosureCaptureKind::SharedBorrow
                };
                self.collect_lambda_captures_expr(inner, defined, used, capture_kinds, ref_kind);
            }
            HirExprKind::Call(func, args, _) => {
                let mut function_ty = self.engine.resolve(&func.ty);
                while let Type::Reference { inner, .. } = function_ty {
                    function_ty = *inner;
                }
                let callee_kind = match function_ty {
                    Type::Function {
                        callable_kind: crate::types::CallableKind::FnMut,
                        ..
                    } => HirClosureCaptureKind::MutableBorrow,
                    Type::Function {
                        callable_kind: crate::types::CallableKind::FnOnce,
                        ..
                    } => HirClosureCaptureKind::Move,
                    _ => HirClosureCaptureKind::SharedBorrow,
                };
                self.collect_lambda_captures_expr(func, defined, used, capture_kinds, callee_kind);
                let expected_params = match &func.ty {
                    Type::Function { params, .. } => Some(params.as_slice()),
                    _ => None,
                };
                for (index, arg) in args.iter().enumerate() {
                    let arg_kind = expected_params
                        .and_then(|params| params.get(index))
                        .map(|expected| Self::lambda_capture_kind_for_value(expected, current_kind))
                        .unwrap_or_else(|| {
                            Self::lambda_capture_kind_for_value(&arg.ty, current_kind)
                        });
                    self.collect_lambda_captures_expr(arg, defined, used, capture_kinds, arg_kind);
                }
            }
            HirExprKind::MethodCall(recv, _, args, self_receiver, _) => {
                let recv_kind = match self_receiver {
                    Some(crate::types::ReceiverMode::Move) => HirClosureCaptureKind::Move,
                    Some(crate::types::ReceiverMode::Mut) => HirClosureCaptureKind::MutableBorrow,
                    Some(crate::types::ReceiverMode::Shared) | None => {
                        HirClosureCaptureKind::SharedBorrow
                    }
                };
                self.collect_lambda_captures_expr(recv, defined, used, capture_kinds, recv_kind);
                for arg in args {
                    self.collect_lambda_captures_expr(
                        arg,
                        defined,
                        used,
                        capture_kinds,
                        current_kind,
                    );
                }
            }
            HirExprKind::Try { expr, .. } => {
                self.collect_lambda_captures_expr(expr, defined, used, capture_kinds, current_kind);
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    let field_kind =
                        Self::lambda_capture_kind_for_value(&field.value.ty, current_kind);
                    self.collect_lambda_captures_expr(
                        &field.value,
                        defined,
                        used,
                        capture_kinds,
                        field_kind,
                    );
                }
            }
            HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    let arg_kind = Self::lambda_capture_kind_for_value(&arg.ty, current_kind);
                    self.collect_lambda_captures_expr(arg, defined, used, capture_kinds, arg_kind);
                }
            }
            HirExprKind::TupleLiteral(elems) | HirExprKind::ArrayLiteral(elems) => {
                for elem in elems {
                    self.collect_lambda_captures_expr(
                        elem,
                        defined,
                        used,
                        capture_kinds,
                        current_kind,
                    );
                }
            }
            HirExprKind::ArrayRepeat(value, _) => {
                self.collect_lambda_captures_expr(value, defined, used, capture_kinds, current_kind)
            }
            HirExprKind::Block(block)
            | HirExprKind::Loop(block)
            | HirExprKind::UnsafeBlock(block) => {
                let mut nested_defined = defined.clone();
                self.collect_lambda_captures_block(
                    block,
                    &mut nested_defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
            }
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.collect_lambda_captures_expr(
                    condition,
                    defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
                let mut then_defined = defined.clone();
                self.collect_lambda_captures_block(
                    then_branch,
                    &mut then_defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
                if let Some(else_branch) = else_branch {
                    let mut else_defined = defined.clone();
                    self.collect_lambda_captures_block(
                        else_branch,
                        &mut else_defined,
                        used,
                        capture_kinds,
                        current_kind,
                    );
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.collect_lambda_captures_expr(
                    scrutinee,
                    defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
                for arm in arms {
                    let mut arm_defined = defined.clone();
                    Self::define_capture_pattern(&arm.pattern, &mut arm_defined);
                    if let Some(guard) = &arm.guard {
                        self.collect_lambda_captures_expr(
                            guard,
                            &mut arm_defined,
                            used,
                            capture_kinds,
                            current_kind,
                        );
                    }
                    self.collect_lambda_captures_block(
                        &arm.body,
                        &mut arm_defined,
                        used,
                        capture_kinds,
                        current_kind,
                    );
                }
            }
            HirExprKind::While { condition, body } => {
                self.collect_lambda_captures_expr(
                    condition,
                    defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
                let mut nested_defined = defined.clone();
                self.collect_lambda_captures_block(
                    body,
                    &mut nested_defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
            }
            HirExprKind::For {
                var, iter, body, ..
            } => {
                self.collect_lambda_captures_expr(iter, defined, used, capture_kinds, current_kind);
                let mut nested_defined = defined.clone();
                nested_defined.insert(var.clone());
                self.collect_lambda_captures_block(
                    body,
                    &mut nested_defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
            }
            HirExprKind::Lambda { captures, .. } => {
                for capture in captures {
                    self.record_lambda_capture(
                        &capture.name, capture.kind, defined, used, capture_kinds,
                    );
                }
            }
            HirExprKind::Intrinsic { args, .. } => {
                for arg in args {
                    self.collect_lambda_captures_expr(
                        arg,
                        defined,
                        used,
                        capture_kinds,
                        current_kind,
                    );
                }
            }
            HirExprKind::Assign(lhs, rhs) => {
                self.collect_lambda_captures_expr(
                    lhs,
                    defined,
                    used,
                    capture_kinds,
                    HirClosureCaptureKind::MutableBorrow,
                );
                self.collect_lambda_captures_expr(rhs, defined, used, capture_kinds, current_kind);
            }
            HirExprKind::Unit
            | HirExprKind::IntLiteral(_) | HirExprKind::ConstParam(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_) => {}
        }
    }

    fn lambda_capture_kind_for_value(
        ty: &Type,
        default: HirClosureCaptureKind,
    ) -> HirClosureCaptureKind {
        match ty {
            Type::Reference { mutable: true, .. } => HirClosureCaptureKind::MutableBorrow,
            Type::Reference { mutable: false, .. } => HirClosureCaptureKind::SharedBorrow,
            _ if !crate::type_services::facts::TypeFacts::is_copy(ty) => {
                HirClosureCaptureKind::Move
            }
            _ => default,
        }
    }

    fn record_lambda_capture(
        &self,
        name: &str,
        kind: HirClosureCaptureKind,
        defined: &std::collections::HashSet<String>,
        used: &mut Vec<String>,
        capture_kinds: &mut std::collections::HashMap<String, HirClosureCaptureKind>,
    ) {
        if defined.contains(name) {
            return;
        }

        match capture_kinds.get_mut(name) {
            Some(existing) => {
                if Self::capture_kind_rank(kind) > Self::capture_kind_rank(*existing) {
                    *existing = kind;
                }
            }
            None => {
                used.push(name.to_string());
                capture_kinds.insert(name.to_string(), kind);
            }
        }
    }

    fn capture_kind_rank(kind: HirClosureCaptureKind) -> u8 {
        match kind {
            HirClosureCaptureKind::SharedBorrow => 0,
            HirClosureCaptureKind::MutableBorrow => 1,
            HirClosureCaptureKind::Move => 2,
        }
    }
}
