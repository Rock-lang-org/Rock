use crate::hir::{HirBindingMode, HirPattern};
use crate::mir::{
    AggregateKind, Constant, MirBinOp, MirCallable, MirCallableKey, MirIntrinsicId, Mutability,
    Operand, Place, Projection, Rvalue, Terminator,
};
use crate::types::{ArrayLen, Type};

use super::MirBuilder;

impl<'a> MirBuilder<'a> {
    fn pattern_intrinsic(&mut self, id: MirIntrinsicId, args: Vec<Operand>, ty: &Type) -> Place {
        let local = self.new_local(self.type_id_for(ty), Mutability::Not, None);
        self.register_scoped_temp(local);
        let destination = Place {
            local,
            projection: vec![],
        };
        self.emit_storage_live(local, None);
        let next = self.new_block();
        if let Some(current) = self.current_block {
            self.set_terminator(
                current,
                Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Intrinsic(id),
                    ))),
                    args,
                    destination: destination.clone(),
                    target: next,
                    span: None,
                },
            );
        }
        self.current_block = Some(next);
        self.mark_place_initialized(&destination, None);
        destination
    }

    pub(super) fn array_pattern_length(&mut self, place: &Place, ty: &Type) -> Operand {
        match ty {
            Type::Array(_, ArrayLen::Value(length)) => {
                Operand::Constant(Constant::Int(*length as i64))
            }
            _ => Operand::Copy(self.pattern_intrinsic(
                MirIntrinsicId::ArrayLen,
                vec![Operand::Copy(place.clone())],
                &Type::I64,
            )),
        }
    }

    fn pattern_index(&mut self, source: &Place, index: Operand) -> Place {
        let local = self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
        self.register_scoped_temp(local);
        self.emit_storage_live(local, None);
        self.emit_assign(
            Place {
                local,
                projection: vec![],
            },
            Rvalue::Use(index),
            None,
        );
        let mut place = source.clone();
        place.projection.push(Projection::Index(local));
        place
    }

    fn pattern_subtract(&mut self, left: Operand, right: usize) -> Operand {
        if let Operand::Constant(Constant::Int(value)) = left {
            return Operand::Constant(Constant::Int(value - right as i64));
        }
        let local = self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
        self.register_scoped_temp(local);
        self.emit_storage_live(local, None);
        let place = Place {
            local,
            projection: vec![],
        };
        self.emit_assign(
            place.clone(),
            Rvalue::BinaryOp(
                MirBinOp::Sub,
                left,
                Operand::Constant(Constant::Int(right as i64)),
            ),
            None,
        );
        Operand::Copy(place)
    }

    /// Projects sequence children only after the length check has succeeded.
    pub(super) fn array_pattern_children(
        &mut self,
        patterns: &[HirPattern],
        rest: Option<usize>,
        source: &Place,
        ty: &Type,
        consume: bool,
    ) -> Vec<(HirPattern, Place, Type)> {
        let element = match ty {
            Type::Array(element, _) | Type::Slice(element) => element.as_ref().clone(),
            _ => return Vec::new(),
        };
        let length = self.array_pattern_length(source, ty);
        // Consume the array as one value, then move its elements as ordinary
        // tuple fields. This gives cleanup an owner for every element,
        // including ignored fields, without permitting partial indexed moves.
        let unpacked = if consume
            && self.needs_move(&element)
            && !source.projection.contains(&Projection::Deref)
        {
            if let Type::Array(_, ArrayLen::Value(count)) = ty {
                let tuple = Type::Tuple(vec![element.clone(); *count as usize]);
                let unpacked = self.pattern_intrinsic(
                    MirIntrinsicId::ArrayUnpack,
                    vec![Operand::Move(source.clone())],
                    &tuple,
                );
                self.mark_place_moved_for_cleanup(source.clone());
                Some(unpacked)
            } else {
                None
            }
        } else {
            None
        };
        let required = patterns.len() - usize::from(rest.is_some());
        let mut children = Vec::new();
        for (index, pattern) in patterns.iter().enumerate() {
            if matches!(pattern, HirPattern::Wildcard) {
                continue;
            }
            if rest == Some(index) {
                let remaining = self.pattern_subtract(length.clone(), required);
                if let Operand::Constant(Constant::Int(count)) = remaining {
                    let array_ty =
                        Type::Array(Box::new(element.clone()), ArrayLen::Value(count as u64));
                    if matches!(
                        pattern,
                        HirPattern::Binding {
                            mode: HirBindingMode::Ref { .. },
                            ..
                        }
                    ) {
                        let place = self
                            .pattern_index(source, Operand::Constant(Constant::Int(index as i64)));
                        children.push((pattern.clone(), place, array_ty));
                    } else {
                        let mut operands = Vec::new();
                        for offset in 0..count {
                            let place = if let Some(unpacked) = &unpacked {
                                let mut place = unpacked.clone();
                                place.projection.push(Projection::Field {
                                    index: index + offset as usize,
                                    identity: None,
                                });
                                place
                            } else {
                                self.pattern_index(
                                    source,
                                    Operand::Constant(Constant::Int(index as i64 + offset)),
                                )
                            };
                            operands.push(self.operand_for_place(&element, place, false));
                        }
                        let local =
                            self.new_local(self.type_id_for(&array_ty), Mutability::Not, None);
                        self.register_scoped_temp(local);
                        self.emit_storage_live(local, None);
                        let place = Place {
                            local,
                            projection: vec![],
                        };
                        self.emit_assign(
                            place.clone(),
                            Rvalue::Aggregate(AggregateKind::Array, operands),
                            None,
                        );
                        children.push((pattern.clone(), place, array_ty));
                    }
                } else {
                    // Preserve a borrow of the original slice while constructing
                    // the adjusted fat pointer. Provenance follows these intrinsics.
                    let mutable = matches!(
                        pattern,
                        HirPattern::Binding {
                            mode: HirBindingMode::Ref { mutable: true },
                            ..
                        }
                    );
                    let slice = Type::Slice(Box::new(element.clone()));
                    let reference = Type::Reference {
                        mutable,
                        inner: Box::new(slice.clone()),
                    };
                    let local = self.new_local(self.type_id_for(&reference), Mutability::Not, None);
                    self.register_scoped_temp(local);
                    self.emit_storage_live(local, None);
                    let borrowed = Place {
                        local,
                        projection: vec![],
                    };
                    self.emit_assign(
                        borrowed.clone(),
                        Rvalue::Ref(
                            if mutable {
                                Mutability::Mut
                            } else {
                                Mutability::Not
                            },
                            source.clone(),
                        ),
                        None,
                    );
                    let pointer_ty = Type::Pointer(Box::new(element.clone()));
                    let pointer = self.pattern_intrinsic(
                        MirIntrinsicId::ArrPtr,
                        vec![Operand::Copy(borrowed)],
                        &pointer_ty,
                    );
                    let pointer = self.pattern_intrinsic(
                        MirIntrinsicId::PtrOffset,
                        vec![
                            Operand::Copy(pointer),
                            Operand::Constant(Constant::Int(index as i64)),
                        ],
                        &pointer_ty,
                    );
                    let wide = self.pattern_intrinsic(
                        MirIntrinsicId::PtrFromParts,
                        vec![Operand::Copy(pointer), remaining],
                        &Type::Pointer(Box::new(slice.clone())),
                    );
                    let mut place = wide;
                    place.projection.push(Projection::Deref);
                    children.push((pattern.clone(), place, slice));
                }
            } else {
                let offset = if rest.is_some_and(|rest| index > rest) {
                    self.pattern_subtract(length.clone(), patterns.len() - index)
                } else {
                    Operand::Constant(Constant::Int(index as i64))
                };
                let place = match (&unpacked, &offset) {
                    (Some(unpacked), Operand::Constant(Constant::Int(index))) => {
                        let mut place = unpacked.clone();
                        place.projection.push(Projection::Field {
                            index: *index as usize,
                            identity: None,
                        });
                        place
                    }
                    _ => self.pattern_index(source, offset),
                };
                children.push((pattern.clone(), place, element.clone()));
            }
        }
        children
    }
}
