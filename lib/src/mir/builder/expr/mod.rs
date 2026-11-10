mod aggregates;
mod calls;
mod control_flow;
mod for_loop;
mod match_expr;
mod operators;
mod try_expr;

use crate::hir::BinOp;
use crate::types::Type;

use super::{HirExpr, HirExprKind, MirBuilder};
use crate::mir::{
    AggregateKind, Constant, MirBinOp, MirClosure, MirUnaryOp, Mutability, Operand, Place,
    Projection, ReferenceOrigin, Rvalue,
};

impl<'a> MirBuilder<'a> {
    pub(super) fn lower_expr_with_context(
        &mut self,
        expr: &HirExpr,
        dest: Place,
        borrow_context: bool,
    ) {
        let _current_block = match self.current_block {
            Some(b) => b,
            None => return,
        };

        let span = Some(expr.span.clone());

        match &expr.kind {
            HirExprKind::IntLiteral(val) => {
                self.emit_assign(
                    dest,
                    Rvalue::Use(Operand::Constant(Constant::Int(*val))),
                    span,
                );
            }
            HirExprKind::ConstParam(param) => {
                // Monomorphization diagnoses unresolved const params before MIR is built.
                panic!(
                    "const generic parameter {param:?} reached MIR lowering in {:?} ({:?})",
                    self.current_function_id, self.current_function_name
                );
            }
            HirExprKind::FloatLiteral(val) => {
                self.emit_assign(
                    dest,
                    Rvalue::Use(Operand::Constant(Constant::Float(*val))),
                    span,
                );
            }
            HirExprKind::BoolLiteral(val) => {
                self.emit_assign(
                    dest,
                    Rvalue::Use(Operand::Constant(Constant::Bool(*val))),
                    span,
                );
            }
            HirExprKind::StringLiteral(val) => {
                self.emit_assign(
                    dest,
                    Rvalue::Use(Operand::Constant(Constant::String(val.clone()))),
                    span,
                );
            }
            HirExprKind::CharLiteral(val) => {
                self.emit_assign(
                    dest,
                    Rvalue::Use(Operand::Constant(Constant::Char(*val))),
                    span,
                );
            }
            HirExprKind::Unit => {
                self.emit_assign(dest, Rvalue::Use(Operand::Constant(Constant::Unit)), span);
            }
            HirExprKind::Var(name) => {
                if let Some(local) = self.var_map.get(name) {
                    let local = *local;
                    let operand = if let Some(src_place) =
                        self.place_for_borrowed_binding_value(local, &expr.ty)
                    {
                        self.operand_for_place(&expr.ty, src_place, false)
                    } else {
                        let src_place = Place {
                            local,
                            projection: vec![],
                        };
                        if let Some(ty) = self.get_local_type(local) {
                            if Self::needs_move(&ty) {
                                Operand::Move(src_place)
                            } else {
                                Operand::Copy(src_place)
                            }
                        } else {
                            Operand::Copy(src_place)
                        }
                    };
                    self.emit_assign(dest, Rvalue::Use(operand), span);
                }
            }
            HirExprKind::ResolvedVar(reference) => {
                if let Some(callable) = self.callable_for_var_target(&reference.target) {
                    self.emit_assign(
                        dest,
                        Rvalue::Use(Operand::Constant(Constant::Callable(callable))),
                        span,
                    );
                } else if let Some(local) = self.var_map.get(&reference.name) {
                    let local = *local;
                    let operand = if let Some(src_place) =
                        self.place_for_borrowed_binding_value(local, &expr.ty)
                    {
                        self.operand_for_place(&expr.ty, src_place, false)
                    } else {
                        let src_place = Place {
                            local,
                            projection: vec![],
                        };
                        if let Some(ty) = self.get_local_type(local) {
                            if Self::needs_move(&ty) {
                                Operand::Move(src_place)
                            } else {
                                Operand::Copy(src_place)
                            }
                        } else {
                            Operand::Copy(src_place)
                        }
                    };
                    self.emit_assign(dest, Rvalue::Use(operand), span);
                }
            }
            HirExprKind::BinOp(op, lhs, rhs) => {
                if matches!(op, BinOp::And | BinOp::Or) {
                    self.lower_short_circuit_binop(*op, lhs, rhs, dest, span);
                    return;
                }

                let lhs_temp = self.new_local_from_expr(lhs.ty.clone(), lhs);
                let rhs_temp = self.new_local_from_expr(rhs.ty.clone(), rhs);

                self.emit_storage_live(lhs_temp, Some(lhs.span.clone()));
                self.emit_storage_live(rhs_temp, Some(rhs.span.clone()));

                let lhs_place = Place {
                    local: lhs_temp,
                    projection: vec![],
                };
                let rhs_place = Place {
                    local: rhs_temp,
                    projection: vec![],
                };

                let is_comparison = matches!(
                    op,
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
                );
                self.lower_expr_with_context(lhs, lhs_place.clone(), is_comparison);
                self.lower_expr_with_context(rhs, rhs_place.clone(), is_comparison);

                let _current_block = match self.current_block {
                    Some(b) => b,
                    None => return,
                };

                let rvalue = Rvalue::BinaryOp(
                    MirBinOp::from(*op),
                    Operand::Copy(lhs_place),
                    Operand::Copy(rhs_place),
                );
                self.emit_assign(dest, rvalue, span);
            }
            HirExprKind::UnaryOp(op, operand) => {
                let operand_temp = self.new_local_from_expr(operand.ty.clone(), operand);
                self.emit_storage_live(operand_temp, Some(operand.span.clone()));

                let operand_place = Place {
                    local: operand_temp,
                    projection: vec![],
                };
                self.lower_expr(operand, operand_place.clone());

                let _current_block = match self.current_block {
                    Some(b) => b,
                    None => return,
                };

                let rvalue = Rvalue::UnaryOp(MirUnaryOp::from(*op), Operand::Copy(operand_place));
                self.emit_assign(dest, rvalue, span);
            }
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.lower_if_expr(dest, span, condition, then_branch, else_branch),
            HirExprKind::Loop(body) => self.lower_loop_expr(expr, dest, borrow_context, body),
            HirExprKind::While { condition, body } => {
                self.lower_while_expr(dest, span, condition, body)
            }
            HirExprKind::For {
                var,
                local_id,
                iter,
                body,
            } => self.lower_for_expr(dest, span, var, local_id, iter, body),
            HirExprKind::Block(block) | HirExprKind::UnsafeBlock(block) => {
                self.lower_block(block, dest);
            }
            HirExprKind::Assign(lhs, rhs) => {
                let rhs_temp = self.new_local_from_expr(rhs.ty.clone(), rhs);
                self.emit_storage_live(rhs_temp, Some(rhs.span.clone()));
                let rhs_place = Place {
                    local: rhs_temp,
                    projection: vec![],
                };
                self.lower_expr(rhs, rhs_place.clone());

                let lhs_place_opt = self.lower_place(lhs);

                if let Some(lhs_place) = lhs_place_opt {
                    if self.type_needs_cleanup(&lhs.ty)
                        && self.assignment_lhs_is_initialized_owned(lhs, &lhs_place)
                    {
                        self.emit_drop_for_place(&lhs.ty, lhs_place.clone(), None);
                    }

                    let operand = if Self::needs_move(&rhs.ty) {
                        Operand::Move(rhs_place)
                    } else {
                        Operand::Copy(rhs_place)
                    };
                    self.emit_assign(lhs_place, Rvalue::Use(operand), Some(lhs.span.clone()));

                    self.emit_assign(dest, Rvalue::Use(Operand::Constant(Constant::Unit)), span);
                }
            }
            HirExprKind::Call(func, args, target) => {
                self.lower_call_expr(expr, dest, span, func, args, target)
            }
            HirExprKind::Intrinsic { name, args } => {
                self.lower_intrinsic_expr(dest, span, name, args)
            }
            HirExprKind::Ref(mutable, base) => {
                let borrow_mutability = if *mutable {
                    Mutability::Mut
                } else {
                    Mutability::Not
                };

                if let HirExprKind::Deref(inner) = &base.kind {
                    if !*mutable && matches!(inner.ty, Type::Reference { .. }) {
                        self.lower_expr(inner, dest);
                        return;
                    }
                }

                if let Some(place) = self.lower_place(base) {
                    let place_is_existing_shared_ref = !*mutable
                        && place.projection.is_empty()
                        && matches!(
                            self.get_local_type(place.local),
                            Some(Type::Reference { mutable: false, inner })
                                if inner.as_ref() == &base.ty
                        );
                    if place_is_existing_shared_ref {
                        self.emit_assign(dest, Rvalue::Use(Operand::Copy(place)), span);
                    } else {
                        self.emit_assign(dest, Rvalue::Ref(borrow_mutability, place), span);
                    }
                } else {
                    let base_temp = self.new_local_from_expr(base.ty.clone(), base);
                    self.register_scoped_temp(base_temp);
                    self.emit_storage_live(base_temp, Some(base.span.clone()));
                    let base_place = Place {
                        local: base_temp,
                        projection: vec![],
                    };

                    self.lower_expr(base, base_place.clone());
                    let dest_local = dest.local;
                    self.emit_assign(dest, Rvalue::Ref(borrow_mutability, base_place), span);
                    if matches!(base.kind, HirExprKind::StringLiteral(_)) {
                        self.record_reference_origin(dest_local, ReferenceOrigin::Static);
                    }
                }
            }
            HirExprKind::Deref(base) => {
                if let Some(place) = self.lower_place(expr) {
                    let operand = self.operand_for_place(&expr.ty, place, borrow_context);
                    self.emit_assign(dest, Rvalue::Use(operand), span);
                } else {
                    let base_temp = self.new_local_from_expr(base.ty.clone(), base);
                    self.register_scoped_temp(base_temp);
                    self.emit_storage_live(base_temp, Some(base.span.clone()));
                    let base_place = Place {
                        local: base_temp,
                        projection: vec![],
                    };

                    self.lower_expr(base, base_place.clone());

                    let mut deref_place = base_place;
                    deref_place.projection.push(Projection::Deref);
                    let operand = self.operand_for_place(&expr.ty, deref_place, borrow_context);
                    self.emit_assign(dest, Rvalue::Use(operand), span);
                }
            }
            HirExprKind::MethodCall(_, method_name, _, _, target) => {
                panic!(
                    "unmaterialized method call reached MIR lowering in {:?} ({:?}): {method_name} ({target:?})",
                    self.current_function_id,
                    self.current_function_name
                );
            }
            HirExprKind::Try {
                expr: carrier,
                branch_method: _,
                branch_target,
                branch_self_receiver: _,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                control_flow_enum,
                break_variant,
                continue_variant,
            } => {
                self.lower_try_expr(
                    carrier,
                    branch_target,
                    from_residual_target,
                    output_ty,
                    residual_ty,
                    return_ty,
                    *control_flow_enum,
                    break_variant,
                    continue_variant,
                    dest,
                    span,
                );
            }
            HirExprKind::ArrayLiteral(elems) | HirExprKind::TupleLiteral(elems) => {
                let mut elem_operands = Vec::new();
                for elem in elems {
                    let elem_temp = self.new_local_from_expr(elem.ty.clone(), elem);
                    let elem_place = Place {
                        local: elem_temp,
                        projection: vec![],
                    };
                    self.lower_expr(elem, elem_place.clone());
                    elem_operands.push(Operand::Copy(elem_place));
                }

                let kind = if matches!(&expr.kind, HirExprKind::ArrayLiteral(_)) {
                    AggregateKind::Array
                } else {
                    AggregateKind::Tuple
                };

                let rvalue = Rvalue::Aggregate(kind, elem_operands);
                self.emit_assign(dest, rvalue, span);
            }
            HirExprKind::ArrayRepeat(value, len) => {
                let value_temp = self.new_local_from_expr(value.ty.clone(), value);
                let value_place = Place {
                    local: value_temp,
                    projection: vec![],
                };
                self.lower_expr(value, value_place.clone());
                let operand = if Self::needs_move(&value.ty) {
                    Operand::Move(value_place)
                } else {
                    Operand::Copy(value_place)
                };
                self.emit_assign(
                    dest,
                    Rvalue::Aggregate(AggregateKind::Array, vec![operand; *len]),
                    span,
                );
            }
            HirExprKind::StructLiteral(name, explicit_id, fields) => {
                self.lower_struct_literal_expr(expr, dest, span, name, explicit_id, fields)
            }
            HirExprKind::FieldAccess(base, _field, _location) => {
                if let Some(place) = self.lower_place(expr) {
                    let operand = self.operand_for_place(&expr.ty, place, false);
                    self.emit_assign(dest, Rvalue::Use(operand), span);
                } else {
                    let base_temp = self.new_local_from_expr(base.ty.clone(), base);
                    self.register_scoped_temp(base_temp);
                    self.emit_storage_live(base_temp, Some(base.span.clone()));
                    let base_place = Place {
                        local: base_temp,
                        projection: vec![],
                    };
                    self.lower_expr_with_context(base, base_place.clone(), true);

                    if let HirExprKind::FieldAccess(_, field_name, location) = &expr.kind {
                        if let Type::Struct { id, .. } = &base.ty {
                            if let Some((_, fields)) = self.program.struct_by_id(*id) {
                                let field_idx = location
                                    .as_ref()
                                    .filter(|location| location.owner == fields.id)
                                    .and_then(|location| {
                                        fields.fields.iter().position(|field| {
                                            field.id == location.field_id
                                                && field.name == location.name
                                                && field.name == *field_name
                                        })
                                    });
                                if let Some(field_idx) = field_idx {
                                    let mut place = base_place;
                                    let identity = location.as_ref().filter(|location| {
                                        location.owner == fields.id
                                            && fields.fields.get(field_idx).is_some_and(|field| {
                                                field.id == location.field_id
                                                    && field.name == location.name
                                                    && field.name == *field_name
                                            })
                                    });
                                    place.projection.push(self.field_projection(
                                        field_idx,
                                        identity.map(|location| location.owner),
                                        identity.map(|location| location.field_id),
                                    ));
                                    let operand = self.operand_for_place(&expr.ty, place, false);
                                    self.emit_assign(dest, Rvalue::Use(operand), span);
                                }
                            }
                        }
                    }
                }
            }
            HirExprKind::TupleIndex(base, _idx) => {
                if let Some(place) = self.lower_place(expr) {
                    let operand = self.operand_for_place(&expr.ty, place, false);
                    self.emit_assign(dest, Rvalue::Use(operand), span);
                } else {
                    let base_temp = self.new_local_from_expr(base.ty.clone(), base);
                    self.register_scoped_temp(base_temp);
                    self.emit_storage_live(base_temp, Some(base.span.clone()));
                    let base_place = Place {
                        local: base_temp,
                        projection: vec![],
                    };
                    self.lower_expr_with_context(base, base_place.clone(), true);

                    if let HirExprKind::TupleIndex(_, idx) = &expr.kind {
                        if matches!(&base.ty, Type::Tuple(elems) if (*idx as usize) < elems.len()) {
                            let mut place = base_place;
                            place
                                .projection
                                .push(self.field_projection(*idx as usize, None, None));
                            let operand = self.operand_for_place(&expr.ty, place, false);
                            self.emit_assign(dest, Rvalue::Use(operand), span);
                        }
                    }
                }
            }
            HirExprKind::Lambda {
                params,
                body,
                captures,
            } => {
                let (closure_id, lambda_name) = self.new_closure_id_and_name();
                self.push_closure_body_function(
                    closure_id.clone(),
                    &lambda_name,
                    params.clone(),
                    body.clone(),
                    captures.clone(),
                    expr.span.clone(),
                );
                let closure_captures = self.lower_closure_captures(captures);
                self.emit_assign(
                    dest,
                    Rvalue::Closure(MirClosure {
                        id: closure_id,
                        display_name: lambda_name,
                        captures: closure_captures,
                    }),
                    span,
                );
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.lower_match_expr(expr, dest, span, scrutinee, arms)
            }
            HirExprKind::EnumVariant(enum_name, variant_name, args, location) => self
                .lower_enum_variant_expr(expr, dest, span, enum_name, variant_name, args, location),
            HirExprKind::Cast(inner, target_ty) => {
                let inner_temp = self.new_local_from_expr(inner.ty.clone(), inner);
                self.emit_storage_live(inner_temp, Some(inner.span.clone()));
                let inner_place = Place {
                    local: inner_temp,
                    projection: vec![],
                };
                self.lower_expr(inner, inner_place.clone());
                if inner.ty != *target_ty {
                    self.emit_assign(
                        dest,
                        Rvalue::Cast(Operand::Copy(inner_place), self.type_id_for(target_ty)),
                        span,
                    );
                } else {
                    self.emit_assign(dest, Rvalue::Use(Operand::Copy(inner_place)), span);
                }
            }
        }
    }

    fn assignment_lhs_is_initialized_owned(&self, lhs: &HirExpr, place: &Place) -> bool {
        if place.projection.is_empty() {
            return true;
        }

        if matches!(place.projection.first(), Some(Projection::Index(_)))
            && matches!(self.get_local_type(place.local), Some(Type::Pointer(_)))
        {
            return false;
        }

        fn expr_is_owned(lhs: &HirExpr) -> bool {
            match &lhs.kind {
                HirExprKind::Var(_) | HirExprKind::ResolvedVar(_) => true,
                HirExprKind::FieldAccess(base, _, _) | HirExprKind::TupleIndex(base, _) => {
                    expr_is_owned(base)
                }
                HirExprKind::Deref(base) => matches!(base.ty, Type::Reference { .. }),
                _ => false,
            }
        }

        expr_is_owned(lhs)
    }
}
