use std::collections::HashMap;

use crate::ids::{CrateId, DefId, LocalDefId, TypeId};
use crate::mono::hir_types::{HirBlock, HirExpr, HirExprKind, HirFunction, HirParam, HirStmt};
use crate::types::{GenericParamDecl, Type};

use crate::mono::Monomorphizer;

impl Monomorphizer {
    /// Specialize an impl method by substituting type parameters
    pub(super) fn specialize_impl_method(
        &mut self,
        method: &HirFunction,
        type_args: &[TypeId],
        type_generics: &[GenericParamDecl],
        specialized_name: &str,
        impl_id: DefId,
        _trait_id: Option<DefId>,
    ) -> HirFunction {
        let mut substitution = HashMap::new();
        let impl_type_arg_count = type_generics.len().min(type_args.len());
        for (i, decl) in type_generics.iter().enumerate() {
            if i < type_args.len() {
                substitution.insert(decl.id, type_args[i]);
            }
        }

        let method_type_arg_count = type_args.len().saturating_sub(impl_type_arg_count);
        let method_generic_candidates: Vec<_> = Self::method_generic_ids_in_order(method)
            .into_iter()
            .filter(|id| !type_generics.iter().any(|decl| decl.id == *id))
            .collect();
        let method_generic_ids = method_generic_candidates[method_generic_candidates
            .len()
            .saturating_sub(method_type_arg_count)..]
            .to_vec();

        for (i, generic_id) in method_generic_ids.iter().enumerate() {
            if impl_type_arg_count + i < type_args.len() {
                substitution.insert(*generic_id, type_args[impl_type_arg_count + i]);
            }
        }

        let specialized_impl_type = if let Some(self_param) = method.params.first() {
            self.substitute_type_with_map(&self_param.ty, &substitution)
        } else {
            match self.impl_receiver_pattern_by_id(impl_id).cloned() {
                Some(crate::hir::HirImplReceiverPattern::Exact(ty)) => {
                    self.substitute_type_with_map(&ty, &substitution)
                }
                Some(crate::hir::HirImplReceiverPattern::Constructor(ty)) => {
                    self.substitute_type_with_map(&ty, &substitution)
                }
                Some(crate::hir::HirImplReceiverPattern::SliceFamily { element }) => Type::Slice(
                    Box::new(self.substitute_type_with_map(&element, &substitution)),
                ),
                None => panic!(
                    "accepted impl {:?} is missing canonical receiver authority",
                    impl_id
                ),
            }
        };

        let params: Vec<HirParam> = method
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let ty = if i == 0 && p.name == "self" && method.is_method {
                    specialized_impl_type.clone()
                } else {
                    self.substitute_type_with_map(&p.ty, &substitution)
                };
                HirParam {
                    name: p.name.clone(),
                    local_id: p.local_id,
                    ty,
                    mutable: p.mutable,
                    is_ref: p.is_ref,
                }
            })
            .collect();

        let ret_type = self.substitute_type_with_map(&method.ret_type, &substitution);
        let mut body = self.substitute_block(&method.body, &substitution);

        self.update_self_types(&mut body, &specialized_impl_type);

        let old_type_args = self.current_type_args.clone();
        let old_var_types = self.var_types.clone();
        self.current_type_args = type_args.to_vec();
        self.var_types.clear();
        let mut params_for_body = params.clone();
        self.process_params(Some(method.id), &mut params_for_body);
        self.process_block(&mut body);
        self.current_type_args = old_type_args;
        self.var_types = old_var_types;

        HirFunction {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            name: specialized_name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: crate::hir::HirGenericBounds::new(),
            params,
            ret_type,
            body,
            is_curried: method.is_curried,
            is_method: method.is_method,
            self_receiver: method.self_receiver,
            is_unsafe: method.is_unsafe,
        }
    }

    /// Update all `self` variable expressions in a block to have the correct type
    fn update_self_types(&self, block: &mut HirBlock, self_type: &Type) {
        for stmt in &mut block.stmts {
            self.update_self_types_stmt(stmt, self_type);
        }
    }

    fn update_self_types_stmt(&self, stmt: &mut HirStmt, self_type: &Type) {
        match stmt {
            HirStmt::Let { value, .. } => {
                self.update_self_types_expr(value, self_type);
            }
            HirStmt::Expr(expr) => {
                self.update_self_types_expr(expr, self_type);
            }
            HirStmt::Return(expr) => {
                if let Some(e) = expr {
                    self.update_self_types_expr(e, self_type);
                }
            }
            HirStmt::Break(expr) => {
                if let Some(e) = expr {
                    self.update_self_types_expr(e, self_type);
                }
            }
            HirStmt::Continue => {}
        }
    }

    fn update_self_types_expr(&self, expr: &mut HirExpr, self_type: &Type) {
        if let HirExprKind::Var(name) = &expr.kind {
            if name == "self" {
                expr.ty = self_type.clone();
            }
        }

        match &mut expr.kind {
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit => {}
            HirExprKind::BinOp(_, lhs, rhs) => {
                self.update_self_types_expr(lhs, self_type);
                self.update_self_types_expr(rhs, self_type);
            }
            HirExprKind::UnaryOp(_, inner) => {
                self.update_self_types_expr(inner, self_type);
            }
            HirExprKind::Call(func, args, _) => {
                self.update_self_types_expr(func, self_type);
                for arg in args {
                    self.update_self_types_expr(arg, self_type);
                }
            }
            HirExprKind::MethodCall(recv, _, args, _, _) => {
                self.update_self_types_expr(recv, self_type);
                for arg in args {
                    self.update_self_types_expr(arg, self_type);
                }
            }
            HirExprKind::Try { expr, .. } => {
                self.update_self_types_expr(expr, self_type);
            }
            HirExprKind::FieldAccess(inner, _, _) | HirExprKind::TupleIndex(inner, _) => {
                self.update_self_types_expr(inner, self_type);
            }
            HirExprKind::ArrayLiteral(elems) | HirExprKind::TupleLiteral(elems) => {
                for e in elems {
                    self.update_self_types_expr(e, self_type);
                }
            }
            HirExprKind::ArrayRepeat(value, _) => {
                self.update_self_types_expr(value, self_type);
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    self.update_self_types_expr(&mut field.value, self_type);
                }
            }
            HirExprKind::EnumVariant(_, _, args, _) => {
                for e in args {
                    self.update_self_types_expr(e, self_type);
                }
            }
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.update_self_types_expr(condition, self_type);
                self.update_self_types(then_branch, self_type);
                if let Some(e) = else_branch {
                    self.update_self_types(e, self_type);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.update_self_types_expr(scrutinee, self_type);
                for arm in arms {
                    if let Some(g) = &mut arm.guard {
                        self.update_self_types_expr(g, self_type);
                    }
                    self.update_self_types(&mut arm.body, self_type);
                }
            }
            HirExprKind::While { condition, body } => {
                self.update_self_types_expr(condition, self_type);
                self.update_self_types(body, self_type);
            }
            HirExprKind::For { iter, body, .. } => {
                self.update_self_types_expr(iter, self_type);
                self.update_self_types(body, self_type);
            }
            HirExprKind::Loop(body) | HirExprKind::Block(body) | HirExprKind::UnsafeBlock(body) => {
                self.update_self_types(body, self_type);
            }
            HirExprKind::Lambda { body, .. } => {
                self.update_self_types(body, self_type);
            }
            HirExprKind::Ref(_, inner)
            | HirExprKind::Deref(inner)
            | HirExprKind::Cast(inner, _) => {
                self.update_self_types_expr(inner, self_type);
            }
            HirExprKind::Assign(lhs, rhs) => {
                self.update_self_types_expr(lhs.as_mut(), self_type);
                self.update_self_types_expr(rhs.as_mut(), self_type);
            }
            HirExprKind::Intrinsic { args, .. } => {
                for arg in args {
                    self.update_self_types_expr(arg, self_type);
                }
            }
        }
    }
}
