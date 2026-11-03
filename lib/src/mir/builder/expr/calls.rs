use crate::hir::HirCallTarget;
use crate::lexer::Span;
use crate::type_services::facts::TypeFacts;
use crate::types::Type;

use crate::mir::builder::{HirExpr, HirExprKind, MirBuilder};
use crate::mir::{
    Constant, MirCallable, MirIntrinsicId, Mutability, Operand, Place, Projection, Rvalue,
    Terminator,
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

    pub(super) fn lower_call_expr(
        &mut self,
        expr: &HirExpr,
        dest: Place,
        span: Option<Span>,
        func: &HirExpr,
        args: &[HirExpr],
        target: &Option<HirCallTarget>,
    ) {
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
            Some(crate::types::ReceiverMode::Shared) | Some(crate::types::ReceiverMode::Mut)
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
                let mutability = if matches!(arg_ty, Type::Reference { mutable: true, .. }) {
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

    pub(super) fn lower_intrinsic_expr(
        &mut self,
        dest: Place,
        span: Option<Span>,
        name: &str,
        args: &[HirExpr],
    ) {
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
                self.emit_assign(dest, Rvalue::Use(Operand::Constant(Constant::Unit)), span);
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

            if intrinsic == MirIntrinsicId::ArrayLen && matches!(arg.ty, Type::Array(_, _)) {
                arg_operands.push(Operand::Constant(Constant::TypeId(
                    self.type_id_for(&arg.ty),
                )));
                continue;
            }

            if intrinsic == MirIntrinsicId::PtrFromParts && index == 0 {
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
}
