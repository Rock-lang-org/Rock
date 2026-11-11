use crate::ids::HirLocalId;
use crate::lexer::Span;
use crate::types::Type;

use crate::mir::builder::drop_state::LoopOwnedArray;
use crate::mir::builder::{HirBlock, HirExpr, HirExprKind, LoopTargets, MirBuilder};
use crate::mir::{
    Constant, LocalSource, MirBinOp, Mutability, Operand, Place, Projection, Rvalue, Terminator,
};

impl<'a> MirBuilder<'a> {
    pub(super) fn lower_for_expr(
        &mut self,
        dest: Place,
        span: Option<Span>,
        var: &String,
        local_id: &HirLocalId,
        iter: &HirExpr,
        body: &HirBlock,
    ) {
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
        let mut loop_owned_elem_ty: Option<Type> = None;
        let mut loop_owned_temp = None;
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
                // The counter is registered before the iterated temporary so scope-exit
                // cleanup (reverse order) can still read it while dropping the
                // temporary's unvisited tail.
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

                let iter_temp = self.new_local_from_expr(iter.ty.clone(), iter);
                self.register_scoped_temp(iter_temp);
                self.emit_storage_live(iter_temp, Some(iter.span.clone()));
                let iter_place = Place {
                    local: iter_temp,
                    projection: vec![],
                };
                self.lower_expr(iter, iter_place.clone());

                let (elem_ty, end_operand, array_len) = match &iter.ty {
                    Type::Array(elem, len) => {
                        let Some(len) = self.concrete_array_length(*len, Some(iter.span.clone())) else { return; };
                        ((**elem).clone(), Operand::Constant(Constant::Int(len as i64)), Some(len))
                    }
                    Type::Slice(elem) => {
                        let length = self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
                        self.register_scoped_temp(length);
                        self.emit_storage_live(length, Some(iter.span.clone()));
                        let next = self.new_block();
                        if let Some(current) = self.current_block {
                            self.set_terminator(current, Terminator::Call {
                                func: Operand::Constant(Constant::Callable(crate::mir::MirCallable::Resolved(
                                    crate::mir::MirCallableKey::Intrinsic(crate::mir::MirIntrinsicId::ArrayLen),
                                ))),
                                args: vec![Operand::Copy(iter_place.clone())],
                                destination: Place { local: length, projection: Vec::new() },
                                target: next,
                                span: Some(iter.span.clone()),
                            });
                        }
                        self.current_block = Some(next);
                        self.mark_place_initialized(&Place { local: length, projection: Vec::new() }, Some(iter.span.clone()));
                        ((**elem).clone(), Operand::Copy(Place { local: length, projection: Vec::new() }), None)
                    }
                    _ => {
                        self.diagnostics.push(crate::diagnostic::Diagnostic::new(
                            format!("cannot lower for-loop iterator type {}", iter.ty), iter.span.clone(),
                        ).with_code(crate::diagnostic::DiagnosticCode::Mono));
                        self.current_block = None;
                        return;
                    }
                };
                // A by-value array of cleanup elements is consumed by the loop: it moves
                // each element out and drops the unvisited tail itself on every exit.
                loop_owned_elem_ty = (matches!(&iter.ty, Type::Array(_, _))
                    && self.type_needs_cleanup(&elem_ty))
                .then(|| elem_ty.clone());
                let loop_local = self.new_local_with_source(
                    self.type_id_for(&elem_ty),
                    Mutability::Not,
                    Some(var.clone()),
                    LocalSource::UserBinding,
                    binding_span.clone(),
                );
                if let (Some(owned_elem_ty), Some(len)) = (&loop_owned_elem_ty, array_len) {
                    loop_owned_temp = Some(iter_temp);
                    self.drops.register_loop_owned_array(
                        iter_temp,
                        LoopOwnedArray {
                            counter,
                            item_local: loop_local,
                            elem_ty: owned_elem_ty.clone(),
                            len: len as usize,
                        },
                    );
                }
                self.register_scoped_temp(loop_local);
                self.emit_storage_live(loop_local, binding_span.clone());

                (
                    counter,
                    end_operand,
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
        let cond_temp = self.new_local(self.type_id_for(&Type::Bool), Mutability::Not, None);
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
        let item_local_for_drop = iter_place.as_ref().map(|(_, item_local)| *item_local);
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
            let item_dest = Place {
                local: item_local,
                projection: vec![],
            };
            if loop_owned_elem_ty.is_some() {
                self.emit_loop_owned_array_move(item_dest, operand, Some(iter.span.clone()));
            } else {
                self.emit_assign(item_dest, Rvalue::Use(operand), Some(iter.span.clone()));
            }
        }

        self.loop_stack.push(LoopTargets {
            break_target: merge_block,
            continue_target: inc_block,
            cleanup_depth: self.scope_locals.len(),
            owned_array: loop_owned_temp,
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
        // The binding owns the moved-out element for one pass; drop it before the
        // next element replaces it unless the body moved it away.
        if let (Some(elem_ty), Some(item_local)) = (&loop_owned_elem_ty, item_local_for_drop) {
            let previous_cleanup_mode = self.drops.enter_cleanup();
            self.emit_drop_for_place(
                elem_ty,
                Place {
                    local: item_local,
                    projection: vec![],
                },
                None,
            );
            self.drops.leave_cleanup(previous_cleanup_mode);
        }
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
        // Normal completion and `break` (which dropped the tail itself) both leave
        // the array fully consumed, so exits after the loop need no tail cleanup.
        if let Some(temp) = loop_owned_temp {
            self.finish_loop_owned_array(temp);
        }
        self.emit_assign(dest, Rvalue::Use(Operand::Constant(Constant::Unit)), span);
    }
}
