use std::collections::HashSet;

use crate::hir::{BinOp, HirCallTarget, HirVariantLocation};
use crate::type_services::facts::TypeFacts;
use crate::types::Type;

use super::{HirExpr, HirExprKind, LoopTargets, MatchScrutineeMode, MirBuilder};
use crate::mir::{
    AggregateKind, Constant, LocalSource, MirBinOp, MirCallable, MirClosure, MirIntrinsicId,
    MirUnaryOp, Mutability, Operand, Place, Projection, ReferenceOrigin, Rvalue, Terminator,
};

impl<'a> MirBuilder<'a> {
    fn lower_indirect_callee(&mut self, callee: &HirExpr) -> Operand {
        let span = Some(callee.span.clone());
        let mut function_ty = &callee.ty;
        let mut derefs = 0;
        while let Type::Reference { inner, .. } = function_ty {
            function_ty = inner;
            derefs += 1;
        }
        let kind = match function_ty {
            Type::Function { callable_kind, .. } => *callable_kind,
            _ => unreachable!("accepted indirect callee must be a function"),
        };
        let mut place = if let Some(place) = self.lower_place(callee) {
            place
        } else {
            let temp = self.new_local_from_expr(callee.ty.clone(), callee);
            self.register_scoped_temp(temp);
            self.emit_storage_live(temp, span.clone());
            let place = Place {
                local: temp,
                projection: Vec::new(),
            };
            self.lower_expr(callee, place.clone());
            place
        };
        place
            .projection
            .extend((0..derefs).map(|_| Projection::Deref));
        if kind == crate::types::CallableKind::FnOnce {
            return self.operand_for_place(function_ty, place, false);
        }
        let mutable = kind == crate::types::CallableKind::FnMut;
        let reference_ty = Type::Reference {
            mutable,
            inner: Box::new(function_ty.clone()),
        };
        let reference = self.new_local_from_expr(reference_ty, callee);
        let reference_place = Place {
            local: reference,
            projection: Vec::new(),
        };
        self.emit_assign(
            reference_place,
            Rvalue::Ref(
                if mutable {
                    Mutability::Mut
                } else {
                    Mutability::Not
                },
                place,
            ),
            span,
        );
        Operand::Copy(Place {
            local: reference,
            projection: vec![Projection::Deref],
        })
    }

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
            } => {
                let cond_temp = self.new_local_from_expr(Type::Bool, condition);
                self.emit_storage_live(cond_temp, Some(condition.span.clone()));

                let cond_place = Place {
                    local: cond_temp,
                    projection: vec![],
                };
                self.lower_expr(condition, cond_place.clone());

                let current_block = match self.current_block {
                    Some(b) => b,
                    None => return,
                };

                let then_block = self.new_block();
                let else_block = else_branch.as_ref().map(|_| self.new_block());
                let merge_block = self.new_block();

                let else_target = else_block.unwrap_or(merge_block);

                self.blocks[current_block.0].terminator = Some(Terminator::switch_int(
                    Operand::Copy(cond_place),
                    vec![(1, then_block)],
                    else_target,
                    self.source_origin(span.clone()),
                ));

                self.current_block = Some(then_block);
                self.lower_block(then_branch, dest.clone());
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(Terminator::goto(
                        merge_block,
                        crate::mir::MirOrigin::synthetic(
                            crate::mir::MirSyntheticOrigin::ControlFlow,
                            span.clone(),
                        ),
                    ));
                }

                if let Some(else_b) = else_branch {
                    let else_block = else_block.expect("else branch should have an else block");
                    self.current_block = Some(else_block);
                    self.lower_block(else_b, dest.clone());
                    if let Some(current) = self.current_block {
                        self.blocks[current.0].terminator = Some(Terminator::goto(
                            merge_block,
                            crate::mir::MirOrigin::synthetic(
                                crate::mir::MirSyntheticOrigin::ControlFlow,
                                span.clone(),
                            ),
                        ));
                    }
                }

                self.current_block = Some(merge_block);
                self.mark_place_initialized(&dest, span.clone());
            }
            HirExprKind::While { condition, body } => {
                let cond_block = self.new_block();
                let body_block = self.new_block();
                let merge_block = self.new_block();

                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(Terminator::goto(
                        cond_block,
                        crate::mir::MirOrigin::synthetic(
                            crate::mir::MirSyntheticOrigin::ControlFlow,
                            Some(condition.span.clone()),
                        ),
                    ));
                }

                self.current_block = Some(cond_block);
                let cond_temp =
                    self.new_local(self.type_id_for(&Type::Bool), Mutability::Not, None);
                self.emit_storage_live(cond_temp, Some(condition.span.clone()));

                let cond_place = Place {
                    local: cond_temp,
                    projection: vec![],
                };
                self.lower_expr(condition, cond_place.clone());

                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(Terminator::switch_int(
                        Operand::Copy(cond_place),
                        vec![(1, body_block)],
                        merge_block,
                        self.source_origin(Some(condition.span.clone())),
                    ));
                }

                self.current_block = Some(body_block);
                self.loop_stack.push(LoopTargets {
                    break_target: merge_block,
                    continue_target: cond_block,
                    cleanup_depth: self.scope_locals.len(),
                });
                let body_dest = Place {
                    local: self.new_local(self.type_id_for(&Type::Unit), Mutability::Not, None),
                    projection: vec![],
                };
                self.lower_block(body, body_dest);
                self.loop_stack.pop();

                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(Terminator::goto(
                        cond_block,
                        crate::mir::MirOrigin::synthetic(
                            crate::mir::MirSyntheticOrigin::ControlFlow,
                            Some(condition.span.clone()),
                        ),
                    ));
                }

                self.current_block = Some(merge_block);
                self.emit_assign(dest, Rvalue::Use(Operand::Constant(Constant::Unit)), span);
            }
            HirExprKind::For {
                var,
                local_id,
                iter,
                body,
            } => {
                let binding_span = self.source_local_span(self.current_source_owner, *local_id);
                let cond_block = self.new_block();
                let body_block = self.new_block();
                let inc_block = self.new_block();
                let merge_block = self.new_block();

                let nominal_range = match &iter.kind {
                    HirExprKind::EnumVariant(_, _, args, Some(location)) if args.len() == 2 => self
                        .program
                        .language_items
                        .range
                        .as_ref()
                        .filter(|items| items.enum_id == location.owner)
                        .and_then(|items| {
                            if location.variant_id == items.exclusive_variant_id {
                                Some((&args[0], &args[1], false))
                            } else if location.variant_id == items.inclusive_variant_id {
                                Some((&args[0], &args[1], true))
                            } else {
                                None
                            }
                        }),
                    _ => None,
                };
                let (loop_local, end_operand, iter_place, iter_base_ty, inclusive_range) =
                    if let Some((start, end, inclusive)) = nominal_range {
                        let loop_local = self.new_local_with_source(
                            self.type_id_for(&Type::I64),
                            Mutability::Mut,
                            Some(var.clone()),
                            LocalSource::UserBinding,
                            binding_span.clone(),
                        );
                        self.register_scoped_temp(loop_local);
                        self.emit_storage_live(loop_local, Some(start.span.clone()));
                        self.lower_expr(
                            start,
                            Place {
                                local: loop_local,
                                projection: vec![],
                            },
                        );

                        let end_local = self.new_local_from_expr(Type::I64, end);
                        self.register_scoped_temp(end_local);
                        self.emit_storage_live(end_local, Some(end.span.clone()));
                        self.lower_expr(
                            end,
                            Place {
                                local: end_local,
                                projection: vec![],
                            },
                        );

                        (
                            loop_local,
                            Operand::Copy(Place {
                                local: end_local,
                                projection: vec![],
                            }),
                            None,
                            Type::I64,
                            inclusive,
                        )
                    } else {
                        let iter_temp = self.new_local_from_expr(iter.ty.clone(), iter);
                        self.register_scoped_temp(iter_temp);
                        self.emit_storage_live(iter_temp, Some(iter.span.clone()));
                        let iter_place = Place {
                            local: iter_temp,
                            projection: vec![],
                        };
                        self.lower_expr(iter, iter_place.clone());

                        let counter = self.new_local(
                            self.type_id_for(&Type::I64),
                            Mutability::Mut,
                            Some(format!("{}_index", var)),
                        );
                        self.register_scoped_temp(counter);
                        self.emit_storage_live(counter, Some(iter.span.clone()));
                        self.emit_assign(
                            Place {
                                local: counter,
                                projection: vec![],
                            },
                            Rvalue::Use(Operand::Constant(Constant::Int(0))),
                            binding_span.clone(),
                        );

                        let (elem_ty, len) = match &iter.ty {
                            Type::Array(elem, len) => ((**elem).clone(), *len as i64),
                            Type::Slice(elem) => ((**elem).clone(), 0),
                            _ => (Type::I64, 0),
                        };
                        let loop_local = self.new_local_with_source(
                            self.type_id_for(&elem_ty),
                            Mutability::Not,
                            Some(var.clone()),
                            LocalSource::UserBinding,
                            binding_span.clone(),
                        );
                        self.register_scoped_temp(loop_local);
                        self.emit_storage_live(loop_local, binding_span.clone());

                        (
                            counter,
                            Operand::Constant(Constant::Int(len)),
                            Some((iter_place, loop_local)),
                            iter.ty.clone(),
                            false,
                        )
                    };

                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(Terminator::goto(
                        cond_block,
                        crate::mir::MirOrigin::synthetic(
                            crate::mir::MirSyntheticOrigin::ControlFlow,
                            Some(iter.span.clone()),
                        ),
                    ));
                }

                let previous_binding = self.var_map.insert(
                    var.clone(),
                    iter_place
                        .as_ref()
                        .map(|(_, loop_local)| *loop_local)
                        .unwrap_or(loop_local),
                );

                self.current_block = Some(cond_block);
                let cond_temp =
                    self.new_local(self.type_id_for(&Type::Bool), Mutability::Not, None);
                self.register_scoped_temp(cond_temp);
                self.emit_storage_live(cond_temp, Some(iter.span.clone()));
                self.emit_assign(
                    Place {
                        local: cond_temp,
                        projection: vec![],
                    },
                    Rvalue::BinaryOp(
                        if inclusive_range {
                            MirBinOp::Le
                        } else {
                            MirBinOp::Lt
                        },
                        Operand::Copy(Place {
                            local: loop_local,
                            projection: vec![],
                        }),
                        end_operand,
                    ),
                    Some(iter.span.clone()),
                );
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(Terminator::switch_int(
                        Operand::Copy(Place {
                            local: cond_temp,
                            projection: vec![],
                        }),
                        vec![(1, body_block)],
                        merge_block,
                        self.source_origin(Some(iter.span.clone())),
                    ));
                }

                self.current_block = Some(body_block);
                if let Some((iter_place, item_local)) = iter_place {
                    let mut item_place = iter_place;
                    item_place.projection.push(Projection::Index(loop_local));
                    self.emit_bounds_check_for_index_place(
                        &item_place,
                        &iter_base_ty,
                        Some(iter.span.clone()),
                    );
                    let item_ty = self
                        .type_context
                        .borrow()
                        .type_for(self.locals[item_local.0].ty);
                    let operand = self.operand_for_place(&item_ty, item_place, false);
                    self.emit_assign(
                        Place {
                            local: item_local,
                            projection: vec![],
                        },
                        Rvalue::Use(operand),
                        Some(iter.span.clone()),
                    );
                }

                self.loop_stack.push(LoopTargets {
                    break_target: merge_block,
                    continue_target: inc_block,
                    cleanup_depth: self.scope_locals.len(),
                });
                let body_dest = Place {
                    local: self.new_local(self.type_id_for(&Type::Unit), Mutability::Not, None),
                    projection: vec![],
                };
                self.lower_block(body, body_dest);
                self.loop_stack.pop();
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(Terminator::goto(
                        inc_block,
                        crate::mir::MirOrigin::synthetic(
                            crate::mir::MirSyntheticOrigin::ControlFlow,
                            Some(iter.span.clone()),
                        ),
                    ));
                }

                self.current_block = Some(inc_block);
                let next_temp = self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
                self.register_scoped_temp(next_temp);
                self.emit_storage_live(next_temp, Some(iter.span.clone()));
                self.emit_assign(
                    Place {
                        local: next_temp,
                        projection: vec![],
                    },
                    Rvalue::BinaryOp(
                        MirBinOp::Add,
                        Operand::Copy(Place {
                            local: loop_local,
                            projection: vec![],
                        }),
                        Operand::Constant(Constant::Int(1)),
                    ),
                    Some(iter.span.clone()),
                );
                self.emit_assign(
                    Place {
                        local: loop_local,
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: next_temp,
                        projection: vec![],
                    })),
                    Some(iter.span.clone()),
                );
                if let Some(current) = self.current_block {
                    self.blocks[current.0].terminator = Some(Terminator::goto(
                        cond_block,
                        crate::mir::MirOrigin::synthetic(
                            crate::mir::MirSyntheticOrigin::ControlFlow,
                            Some(iter.span.clone()),
                        ),
                    ));
                }

                if let Some(previous) = previous_binding {
                    self.var_map.insert(var.clone(), previous);
                } else {
                    self.var_map.remove(var);
                }

                self.current_block = Some(merge_block);
                self.emit_assign(dest, Rvalue::Use(Operand::Constant(Constant::Unit)), span);
            }
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
                let unresolved_field_error = match &func.kind {
                    HirExprKind::FieldAccess(receiver, method_name, location)
                        if !self.field_access_resolves_to_struct_field(
                            &receiver.ty,
                            method_name,
                            location.as_ref(),
                        ) =>
                    {
                        Some(self.unknown_field_message(&receiver.ty, method_name))
                    }
                    _ => None,
                };
                if let Some(message) = unresolved_field_error {
                    panic!("{message}");
                }
                let callable = target
                    .as_ref()
                    .and_then(|target| self.callable_for_call_target(target, &expr.ty))
                    .or_else(|| self.callable_for_expr(func, &expr.ty));
                let receiver_mode = callable
                    .as_ref()
                    .and_then(|callable| self.method_receiver_mode_for_callable(callable));
                let call_args = args.iter().collect::<Vec<_>>();
                let expected_param_types = match &func.ty {
                    Type::Function { params, .. } => Some(params.as_slice()),
                    _ => None,
                };
                let user_arg_offset = 0;

                let direct_borrowed_receiver_call = matches!(
                    receiver_mode,
                    Some(crate::types::ReceiverMode::Shared)
                        | Some(crate::types::ReceiverMode::Mut)
                );
                let direct_move_receiver_call =
                    matches!(receiver_mode, Some(crate::types::ReceiverMode::Move));

                let func_operand = if let Some(callable) = callable {
                    Operand::Constant(Constant::Callable(callable))
                } else {
                    self.lower_indirect_callee(func)
                };

                let mut arg_operands = Vec::new();
                for (index, arg) in call_args.into_iter().enumerate() {
                    let borrowed_receiver = index == 0
                        && direct_borrowed_receiver_call
                        && expected_param_types
                            .and_then(|types| types.get(index - user_arg_offset))
                            .is_none_or(|expected| expected != &arg.ty);
                    let move_receiver = index == 0 && direct_move_receiver_call;
                    let reborrowed_receiver = index == 0
                        && direct_borrowed_receiver_call
                        && !borrowed_receiver
                        && matches!(arg.ty, Type::Reference { .. });
                    let expected_arg_ty = if index < user_arg_offset {
                        None
                    } else {
                        expected_param_types.and_then(|types| types.get(index - user_arg_offset))
                    };
                    let arg_ty = if borrowed_receiver {
                        Type::Reference {
                            mutable: matches!(receiver_mode, Some(crate::types::ReceiverMode::Mut)),
                            inner: Box::new(arg.ty.clone()),
                        }
                    } else if let Some(expected_arg_ty) = expected_arg_ty {
                        expected_arg_ty.clone()
                    } else {
                        arg.ty.clone()
                    };
                    let arg_temp = self.new_local_from_expr(arg_ty.clone(), arg);
                    let arg_place = Place {
                        local: arg_temp,
                        projection: vec![],
                    };
                    if borrowed_receiver || reborrowed_receiver {
                        let mutability = if matches!(arg_ty, Type::Reference { mutable: true, .. })
                        {
                            Mutability::Mut
                        } else {
                            Mutability::Not
                        };
                        if let Some(mut place) = self.lower_place(arg) {
                            if reborrowed_receiver {
                                place.projection.push(Projection::Deref);
                            }
                            self.emit_assign(
                                arg_place.clone(),
                                Rvalue::Ref(mutability, place),
                                span.clone(),
                            );
                        } else {
                            let base_temp = self.new_local_from_expr(arg.ty.clone(), arg);
                            self.register_scoped_temp(base_temp);
                            self.emit_storage_live(base_temp, Some(arg.span.clone()));
                            let base_place = Place {
                                local: base_temp,
                                projection: vec![],
                            };
                            self.lower_expr(arg, base_place.clone());
                            let mut base_place = base_place;
                            if reborrowed_receiver {
                                base_place.projection.push(Projection::Deref);
                            }
                            self.emit_assign(
                                arg_place.clone(),
                                Rvalue::Ref(mutability, base_place),
                                span.clone(),
                            );
                        }
                    } else if move_receiver {
                        if let Some(place) = self.lower_place(arg) {
                            let operand = if TypeFacts::is_copy(&arg.ty) {
                                Operand::Copy(place)
                            } else {
                                Operand::Move(place)
                            };
                            self.emit_assign(arg_place.clone(), Rvalue::Use(operand), span.clone());
                        } else {
                            self.lower_expr_with_context(arg, arg_place.clone(), false);
                        }
                    } else {
                        self.lower_expr_with_context(arg, arg_place.clone(), true);
                    }
                    arg_operands.push(self.operand_for_place(&arg_ty, arg_place, false));
                }

                let merge_block = self.new_block();

                if let Some(current) = self.current_block {
                    self.set_terminator(
                        current,
                        Terminator::Call {
                            func: func_operand,
                            args: arg_operands,
                            destination: dest.clone(),
                            target: merge_block,
                            span: span.clone(),
                        },
                    );
                }

                self.current_block = Some(merge_block);
                self.mark_place_initialized(&dest, span.clone());
            }
            HirExprKind::Intrinsic { name, args } => {
                let intrinsic = MirIntrinsicId::from_name(name)
                    .unwrap_or_else(|| panic!("unknown intrinsic reached MIR lowering: {name}"));
                if intrinsic == MirIntrinsicId::DropInPlace && args.len() == 1 {
                    let arg = &args[0];
                    if let Type::Pointer(pointee_ty) = &arg.ty {
                        let mut pointee_place = if let Some(place) = self.lower_place(arg) {
                            place
                        } else {
                            let arg_temp = self.new_local_from_expr(arg.ty.clone(), arg);
                            self.emit_storage_live(arg_temp, Some(arg.span.clone()));
                            let arg_place = Place {
                                local: arg_temp,
                                projection: vec![],
                            };
                            self.lower_expr_with_context(arg, arg_place.clone(), true);
                            arg_place
                        };
                        pointee_place.projection.push(Projection::Deref);
                        self.emit_drop_for_place(pointee_ty, pointee_place, None);
                        self.emit_assign(
                            dest,
                            Rvalue::Use(Operand::Constant(Constant::Unit)),
                            span,
                        );
                        return;
                    }
                }

                let mut arg_operands = Vec::new();
                for (index, arg) in args.iter().enumerate() {
                    if intrinsic == MirIntrinsicId::SizeOf {
                        arg_operands.push(Operand::Constant(Constant::TypeId(
                            self.type_id_for(&arg.ty),
                        )));
                        continue;
                    }

                    if intrinsic == MirIntrinsicId::ArrayLen && matches!(arg.ty, Type::Array(_, _))
                    {
                        arg_operands.push(Operand::Constant(Constant::TypeId(
                            self.type_id_for(&arg.ty),
                        )));
                        continue;
                    }

                    if matches!(
                        intrinsic,
                        MirIntrinsicId::BorrowSlice | MirIntrinsicId::BorrowSliceMut
                    ) && index == 0
                    {
                        if let Some(place) = self.lower_place(arg) {
                            arg_operands.push(Operand::Copy(place));
                            continue;
                        }
                    }

                    let arg_temp = self.new_local_from_expr(arg.ty.clone(), arg);
                    self.emit_storage_live(arg_temp, Some(arg.span.clone()));
                    let arg_place = Place {
                        local: arg_temp,
                        projection: vec![],
                    };
                    self.lower_expr_with_context(arg, arg_place.clone(), true);
                    arg_operands.push(self.operand_for_place(&arg.ty, arg_place, false));
                }

                let merge_block = self.new_block();

                if let Some(current) = self.current_block {
                    self.set_terminator(
                        current,
                        Terminator::Call {
                            func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                                crate::mir::MirCallableKey::Intrinsic(intrinsic),
                            ))),
                            args: arg_operands,
                            destination: dest.clone(),
                            target: merge_block,
                            span: span.clone(),
                        },
                    );
                }

                self.current_block = Some(merge_block);
                self.mark_place_initialized(&dest, span.clone());
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
                let mut lowered_fields = Vec::new();
                for field in fields {
                    let field_expr = &field.value;
                    let field_temp = self.new_local_from_expr(field_expr.ty.clone(), field_expr);
                    let field_place = Place {
                        local: field_temp,
                        projection: vec![],
                    };
                    self.lower_expr(field_expr, field_place.clone());
                    lowered_fields.push((field, Operand::Copy(field_place)));
                }

                let struct_id = explicit_id.or_else(|| match &expr.ty {
                    Type::Struct { id, .. } => Some(*id),
                    _ => None,
                });
                let Some(struct_id) = struct_id else {
                    panic!("struct literal MIR lowering requires canonical struct id for {name}");
                };

                let Some((_, struct_def)) = self.program.struct_by_id(struct_id) else {
                    panic!(
                        "struct literal MIR lowering missing struct definition for {struct_id:?}"
                    );
                };

                let mut ordered: Vec<Option<Operand>> = vec![None; struct_def.fields.len()];
                for (field, operand) in lowered_fields {
                    let Some(location) = field.field.as_ref() else {
                        panic!(
                            "struct literal MIR lowering requires HIR field location for {}.{}",
                            name, field.name
                        );
                    };
                    let Some(field_idx) = struct_def.fields.iter().position(|struct_field| {
                        location.owner == struct_def.id
                            && struct_field.id == location.field_id
                            && struct_field.name == location.name
                            && struct_field.name == field.name
                    }) else {
                        panic!(
                            "struct literal MIR lowering found mismatched HIR field location for {}.{}",
                            name, field.name
                        );
                    };

                    if let Some(slot) = ordered.get_mut(field_idx) {
                        *slot = Some(operand);
                    }
                }

                let field_operands = ordered
                    .into_iter()
                    .enumerate()
                    .map(|(field_idx, operand)| {
                        operand.unwrap_or_else(|| {
                            panic!(
                                "struct literal MIR lowering missing operand for field index {field_idx} of {name}"
                            )
                        })
                    })
                    .collect();

                let rvalue = Rvalue::Aggregate(
                    AggregateKind::Struct {
                        id: struct_id,
                        display_name: name.clone(),
                    },
                    field_operands,
                );
                self.emit_assign(dest, rvalue, span);
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
                let borrowed_scrutinee_inner = match &scrutinee.kind {
                    HirExprKind::Deref(inner) if matches!(inner.ty, Type::Reference { .. }) => {
                        Some(inner.as_ref())
                    }
                    _ => None,
                };
                let (scrut_place, scrutinee_mode, scrutinee_cleanup_local) =
                    if let Some(inner) = borrowed_scrutinee_inner {
                        let mut scrut_place = if let Some(place) = self.lower_place(inner) {
                            place
                        } else {
                            let ref_temp = self.new_local_from_expr(inner.ty.clone(), inner);
                            self.register_scoped_temp(ref_temp);
                            self.emit_storage_live(ref_temp, Some(inner.span.clone()));
                            let ref_place = Place {
                                local: ref_temp,
                                projection: vec![],
                            };
                            self.lower_expr_with_context(inner, ref_place.clone(), true);
                            ref_place
                        };
                        scrut_place.projection.push(Projection::Deref);
                        (scrut_place, MatchScrutineeMode::Borrowed, None)
                    } else {
                        let scrut_temp = self.new_local_from_expr(scrutinee.ty.clone(), scrutinee);
                        self.emit_storage_live(scrut_temp, Some(scrutinee.span.clone()));
                        let scrut_place = Place {
                            local: scrut_temp,
                            projection: vec![],
                        };
                        self.lower_expr_with_context(scrutinee, scrut_place.clone(), false);
                        if Self::needs_move(&scrutinee.ty) {
                            if let Some(source_place) = self.move_source_place_for_expr(scrutinee) {
                                self.mark_place_moved_for_cleanup(source_place);
                            }
                        }
                        (scrut_place, MatchScrutineeMode::ByValue, Some(scrut_temp))
                    };

                let uses_enum_patterns = arms
                    .iter()
                    .any(|arm| self.match_arm_variant_id(&arm.pattern).is_some());
                let discr_place = if uses_enum_patterns {
                    let discr_temp =
                        self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
                    self.emit_storage_live(discr_temp, Some(expr.span.clone()));
                    let discr_place = Place {
                        local: discr_temp,
                        projection: vec![],
                    };
                    self.emit_assign(
                        discr_place.clone(),
                        Rvalue::Discriminant(scrut_place.clone()),
                        span.clone(),
                    );
                    Some(discr_place)
                } else {
                    None
                };

                let check_blocks: Vec<_> = arms.iter().map(|_| self.new_block()).collect();
                let merge_block = self.new_block();
                let enum_patterns_exhaustive = match &scrutinee.ty {
                    Type::Enum { id, .. } => {
                        self.program
                            .enum_by_id(*id)
                            .is_some_and(|(_, enumeration)| {
                                let matched_variants: HashSet<_> = arms
                                    .iter()
                                    .filter(|arm| {
                                        arm.guard.is_none()
                                            && Self::enum_payload_patterns_irrefutable(&arm.pattern)
                                    })
                                    .filter_map(|arm| self.match_arm_variant_id(&arm.pattern))
                                    .collect();
                                matched_variants.len() == enumeration.variants.len()
                            })
                    }
                    _ => false,
                };
                if let Some(current) = self.current_block {
                    let target = check_blocks.first().copied().unwrap_or(merge_block);
                    self.blocks[current.0].terminator = Some(Terminator::goto(
                        target,
                        crate::mir::MirOrigin::synthetic(
                            crate::mir::MirSyntheticOrigin::ControlFlow,
                            Some(scrutinee.span.clone()),
                        ),
                    ));
                }

                for (index, arm) in arms.iter().enumerate() {
                    let check_block = check_blocks[index];
                    let next_block = check_blocks.get(index + 1).copied().unwrap_or(merge_block);
                    let matched_block = self.new_block();
                    let body_block = self.new_block();

                    self.current_block = Some(check_block);
                    if let Some(variant_id) = self.match_arm_variant_id(&arm.pattern) {
                        let variant_success =
                            if Self::enum_payload_patterns_irrefutable(&arm.pattern) {
                                matched_block
                            } else {
                                self.new_block()
                            };
                        let discr_place = discr_place
                            .clone()
                            .expect("enum pattern match should have discriminant temp");
                        if let Some(current) = self.current_block {
                            let otherwise = if enum_patterns_exhaustive
                                && index + 1 == arms.len()
                                && arm.guard.is_none()
                            {
                                variant_success
                            } else {
                                next_block
                            };
                            self.blocks[current.0].terminator = Some(Terminator::switch_int(
                                Operand::Copy(discr_place),
                                vec![(variant_id.0 as i64, variant_success)],
                                otherwise,
                                self.source_origin(Some(scrutinee.span.clone())),
                            ));
                        }
                        if variant_success != matched_block {
                            self.current_block = Some(variant_success);
                            self.lower_enum_payload_pattern_checks(
                                &arm.pattern,
                                scrut_place.clone(),
                                &scrutinee.ty,
                                matched_block,
                                next_block,
                                Some(scrutinee.span.clone()),
                            );
                        }
                    } else if self.match_arm_is_catch_all(&arm.pattern) {
                        if let Some(current) = self.current_block {
                            self.blocks[current.0].terminator = Some(Terminator::goto(
                                matched_block,
                                crate::mir::MirOrigin::synthetic(
                                    crate::mir::MirSyntheticOrigin::ControlFlow,
                                    Some(scrutinee.span.clone()),
                                ),
                            ));
                        }
                    } else {
                        self.lower_match_pattern_check(
                            &arm.pattern,
                            scrut_place.clone(),
                            &scrutinee.ty,
                            matched_block,
                            next_block,
                            Some(scrutinee.span.clone()),
                        );
                    }

                    self.current_block = Some(matched_block);
                    self.scope_locals.push(Vec::new());
                    let mut restored_bindings = Vec::new();
                    let mut moved_enum_scrutinee = None;
                    self.bind_match_pattern_places(
                        &arm.pattern,
                        scrut_place.clone(),
                        &scrutinee.ty,
                        arm.guard.is_some(),
                        false,
                        scrutinee_mode,
                        &mut moved_enum_scrutinee,
                        &mut restored_bindings,
                    );

                    if let Some(guard) = &arm.guard {
                        let guard_temp = self.new_local_from_expr(Type::Bool, guard);
                        if let Some(scope_locals) = self.scope_locals.last_mut() {
                            scope_locals.push(guard_temp);
                        }
                        self.emit_storage_live(guard_temp, Some(guard.span.clone()));
                        let guard_place = Place {
                            local: guard_temp,
                            projection: vec![],
                        };
                        let mut guard_abrupt_cleanup_locals = Vec::new();
                        if let Some(discr_place) = &discr_place {
                            guard_abrupt_cleanup_locals.push(discr_place.local);
                        }
                        if let Some(scrutinee_cleanup_local) = scrutinee_cleanup_local {
                            guard_abrupt_cleanup_locals.push(scrutinee_cleanup_local);
                        }
                        self.scope_locals.push(guard_abrupt_cleanup_locals);
                        self.lower_expr(guard, guard_place.clone());
                        self.scope_locals.pop();
                        let cleanup_locals = self.scope_locals.last().cloned().unwrap_or_default();
                        let guard_false_cleanup =
                            self.scope_cleanup_block(cleanup_locals, next_block);
                        if let Some(current) = self.current_block {
                            self.blocks[current.0].terminator = Some(Terminator::switch_int(
                                Operand::Copy(guard_place),
                                vec![(1, body_block)],
                                guard_false_cleanup,
                                self.source_origin(Some(guard.span.clone())),
                            ));
                        }
                    } else if let Some(current) = self.current_block {
                        self.blocks[current.0].terminator = Some(Terminator::goto(
                            body_block,
                            crate::mir::MirOrigin::synthetic(
                                crate::mir::MirSyntheticOrigin::ControlFlow,
                                Some(scrutinee.span.clone()),
                            ),
                        ));
                    }

                    self.current_block = Some(body_block);
                    let mut match_temp_locals = Vec::new();
                    if let Some(discr_place) = &discr_place {
                        match_temp_locals.push(discr_place.local);
                    }
                    if let Some(scrutinee_cleanup_local) = scrutinee_cleanup_local {
                        match_temp_locals.push(scrutinee_cleanup_local);
                    }
                    self.scope_locals.push(match_temp_locals);
                    self.lower_block(&arm.body, dest.clone());
                    let match_temp_locals = self.scope_locals.pop().unwrap_or_default();
                    self.finish_scope_locals(match_temp_locals);
                    let binding_locals = self.scope_locals.pop().unwrap_or_default();
                    self.finish_scope_locals(binding_locals);
                    self.restore_match_bindings(restored_bindings);
                    if let Some(current) = self.current_block {
                        self.blocks[current.0].terminator = Some(Terminator::goto(
                            merge_block,
                            crate::mir::MirOrigin::synthetic(
                                crate::mir::MirSyntheticOrigin::ControlFlow,
                                Some(scrutinee.span.clone()),
                            ),
                        ));
                    }
                }

                self.current_block = Some(merge_block);
                self.mark_place_initialized(&dest, span.clone());
            }
            HirExprKind::EnumVariant(enum_name, variant_name, args, location) => {
                let field_types = location
                    .as_ref()
                    .map(|location| {
                        let field_types =
                            self.enum_variant_field_types(location.owner, location.variant_id);
                        let generic_subst = match &expr.ty {
                            Type::Enum { args, .. } => {
                                let mut generic_params = std::collections::HashSet::new();
                                for field_ty in &field_types {
                                    field_ty.collect_generic_params(&mut generic_params);
                                }
                                generic_params
                                    .into_iter()
                                    .filter_map(|param| {
                                        args.get(param.index as usize)
                                            .cloned()
                                            .map(|arg| (param, arg))
                                    })
                                    .collect::<std::collections::HashMap<_, _>>()
                            }
                            _ => std::collections::HashMap::new(),
                        };
                        field_types
                            .into_iter()
                            .map(|ty| ty.substitute_generics(&generic_subst))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let mut arg_operands = Vec::new();
                for (index, arg) in args.iter().enumerate() {
                    let arg_ty = field_types
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| arg.ty.clone());
                    let arg_temp = self.new_local_from_expr(arg_ty, arg);
                    let arg_place = Place {
                        local: arg_temp,
                        projection: vec![],
                    };
                    self.lower_expr(arg, arg_place.clone());
                    arg_operands.push(Operand::Copy(arg_place));
                }

                if let Some(location) = location {
                    self.emit_assign(
                        dest,
                        Rvalue::Aggregate(
                            AggregateKind::EnumVariant {
                                enum_id: location.owner,
                                variant_id: location.variant_id,
                                enum_name: enum_name.clone(),
                                variant_name: variant_name.clone(),
                            },
                            arg_operands,
                        ),
                        span,
                    );
                }
            }
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
            _ => {}
        }
    }

    fn lower_try_expr(
        &mut self,
        carrier: &HirExpr,
        branch_target: &Option<HirCallTarget>,
        from_residual_target: &HirCallTarget,
        output_ty: &Type,
        residual_ty: &Type,
        return_ty: &Type,
        control_flow_enum: crate::ids::DefId,
        break_variant: &HirVariantLocation,
        continue_variant: &HirVariantLocation,
        dest: Place,
        span: Option<crate::lexer::Span>,
    ) {
        let control_flow_ty = Type::Enum {
            id: control_flow_enum,
            args: vec![residual_ty.clone(), output_ty.clone()],
        };
        let branch_temp = self.new_local_from_expr(control_flow_ty.clone(), carrier);
        self.register_scoped_temp(branch_temp);
        self.emit_storage_live(branch_temp, Some(carrier.span.clone()));
        let branch_place = Place {
            local: branch_temp,
            projection: vec![],
        };
        let branch_instance = match branch_target {
            Some(HirCallTarget::Instance(instance_id)) => *instance_id,
            target => {
                panic!("Try branch reached MIR without a materialized instance target: {target:?}")
            }
        };
        let branch_callee = HirExpr {
            ty: Type::function(vec![carrier.ty.clone()], control_flow_ty.clone()),
            kind: HirExprKind::ResolvedVar(crate::hir::HirVarRef {
                name: "branch".to_string(),
                target: crate::hir::HirVarTarget::Instance(branch_instance),
            }),
            span: carrier.span.clone(),
        };
        let branch_call = HirExpr {
            ty: control_flow_ty,
            kind: HirExprKind::Call(
                Box::new(branch_callee),
                vec![carrier.clone()],
                Some(HirCallTarget::Instance(branch_instance)),
            ),
            span: carrier.span.clone(),
        };
        self.lower_expr(&branch_call, branch_place.clone());

        let Some(current) = self.current_block else {
            return;
        };
        let discr_temp = self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
        self.register_scoped_temp(discr_temp);
        self.emit_storage_live(discr_temp, span.clone());
        let discr_place = Place {
            local: discr_temp,
            projection: vec![],
        };
        self.emit_assign(
            discr_place.clone(),
            Rvalue::Discriminant(branch_place.clone()),
            span.clone(),
        );

        let continue_block = self.new_block();
        let break_block = self.new_block();
        let merge_block = self.new_block();
        self.blocks[current.0].terminator = Some(Terminator::switch_int(
            Operand::Copy(discr_place),
            vec![(continue_variant.variant_id.0 as i64, continue_block)],
            break_block,
            self.source_origin(span.clone()),
        ));

        self.current_block = Some(break_block);
        let Some(from_residual_callable) =
            self.callable_for_call_target(from_residual_target, return_ty)
        else {
            panic!(
                "Try residual reached MIR without a materialized callable target: {from_residual_target:?}"
            );
        };
        let mut residual_place = branch_place.clone();
        residual_place
            .projection
            .push(Projection::Downcast(break_variant.variant_id));
        residual_place.projection.push(Projection::Field {
            index: 0,
            identity: None,
        });
        let residual_operand = self.operand_for_place(residual_ty, residual_place, false);
        let after_from_residual = self.new_block();
        if let Some(current) = self.current_block {
            self.set_terminator(
                current,
                Terminator::Call {
                    func: Operand::Constant(Constant::Callable(from_residual_callable)),
                    args: vec![residual_operand],
                    destination: Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    target: after_from_residual,
                    span: span.clone(),
                },
            );
        }
        self.current_block = Some(after_from_residual);
        let ret_place = Place {
            local: crate::mir::Local(0),
            projection: vec![],
        };
        self.mark_place_initialized(&ret_place, span.clone());
        self.terminate_with_cleanup(
            0,
            Terminator::return_with_origin(self.source_origin(span.clone())),
        );

        self.current_block = Some(continue_block);
        let mut output_place = branch_place;
        output_place
            .projection
            .push(Projection::Downcast(continue_variant.variant_id));
        output_place.projection.push(Projection::Field {
            index: 0,
            identity: None,
        });
        let output_operand = self.operand_for_place(output_ty, output_place, false);
        self.emit_assign(dest.clone(), Rvalue::Use(output_operand), span.clone());
        if let Some(current) = self.current_block {
            self.blocks[current.0].terminator = Some(Terminator::goto(
                merge_block,
                crate::mir::MirOrigin::synthetic(
                    crate::mir::MirSyntheticOrigin::ControlFlow,
                    span.clone(),
                ),
            ));
        }

        self.current_block = Some(merge_block);
        self.mark_place_initialized(&dest, span);
    }

    fn lower_short_circuit_binop(
        &mut self,
        op: BinOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
        dest: Place,
        span: Option<crate::lexer::Span>,
    ) {
        let lhs_temp = self.new_local_from_expr(Type::Bool, lhs);
        self.emit_storage_live(lhs_temp, Some(lhs.span.clone()));

        let lhs_place = Place {
            local: lhs_temp,
            projection: vec![],
        };
        self.lower_expr(lhs, lhs_place.clone());

        let Some(lhs_block) = self.current_block else {
            return;
        };

        let rhs_block = self.new_block();
        let default_block = self.new_block();
        let merge_block = self.new_block();

        let (true_target, false_target, default_value) = match op {
            BinOp::And => (rhs_block, default_block, false),
            BinOp::Or => (default_block, rhs_block, true),
            _ => unreachable!("short-circuit lowering only handles boolean operators"),
        };

        self.blocks[lhs_block.0].terminator = Some(Terminator::switch_int(
            Operand::Copy(lhs_place),
            vec![(1, true_target)],
            false_target,
            self.source_origin(span.clone()),
        ));

        self.current_block = Some(default_block);
        self.emit_assign(
            dest.clone(),
            Rvalue::Use(Operand::Constant(Constant::Bool(default_value))),
            span.clone(),
        );
        if let Some(current) = self.current_block {
            self.blocks[current.0].terminator = Some(Terminator::goto(
                merge_block,
                crate::mir::MirOrigin::synthetic(
                    crate::mir::MirSyntheticOrigin::ControlFlow,
                    span.clone(),
                ),
            ));
        }

        self.current_block = Some(rhs_block);
        self.lower_expr(rhs, dest);
        if let Some(current) = self.current_block {
            self.blocks[current.0].terminator = Some(Terminator::goto(
                merge_block,
                crate::mir::MirOrigin::synthetic(
                    crate::mir::MirSyntheticOrigin::ControlFlow,
                    span.clone(),
                ),
            ));
        }

        self.current_block = Some(merge_block);
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
