use crate::types::Type;

use super::{HirBlock, HirStmt, MirBuilder};
use crate::mir::{
    Constant, DropObligationKind, LocalSource, Mutability, Operand, Place, Projection, Rvalue,
    StatementData, Terminator,
};

impl<'a> MirBuilder<'a> {
    pub(super) fn finish_scope_locals(&mut self, block_locals: Vec<crate::mir::Local>) {
        let previous_cleanup_mode = self.cleanup_mode;
        self.cleanup_mode = true;
        let mut current_opt = self.current_block;
        for local in block_locals.into_iter().rev() {
            if let Some(current) = current_opt {
                let ty = self.get_local_type(local).unwrap_or(Type::Unit);
                if self.type_needs_cleanup(&ty) {
                    let root_place = Place {
                        local,
                        projection: vec![],
                    };
                    self.emit_drop_for_place(&ty, root_place, Some(local));
                    current_opt = self.current_block;
                } else {
                    self.blocks[current.0]
                        .statements
                        .push(StatementData::storage_dead(local, None));
                }
            }
        }
        self.cleanup_mode = previous_cleanup_mode;
    }

    pub(super) fn emit_drop_for_place(
        &mut self,
        ty: &Type,
        place: Place,
        storage_dead: Option<crate::mir::Local>,
    ) {
        if let Some(flag) = self.drop_flag_for_place(&place) {
            let drop_block = self.new_block();
            let next_block = self.new_block();
            if let Some(current) = self.current_block {
                self.blocks[current.0].terminator = Some(Terminator::switch_int(
                    Operand::Copy(Place {
                        local: flag,
                        projection: vec![],
                    }),
                    vec![(1, drop_block)],
                    next_block,
                    self.cleanup_origin(&place),
                ));
            }

            self.current_block = Some(drop_block);
            self.set_drop_flag_for_place(&place, false, None);
            self.emit_unconditional_drop_for_place(ty, place.clone());
            if let Some(current) = self.current_block {
                self.blocks[current.0].terminator =
                    Some(Terminator::goto(next_block, self.cleanup_origin(&place)));
            }

            if let Some(local) = storage_dead {
                let flag = self.drop_flag_for(local);
                let projection_flags = self.projection_drop_flags_for_local(local);
                self.blocks[next_block.0]
                    .statements
                    .push(StatementData::storage_dead(local, None));
                self.blocks[next_block.0]
                    .statements
                    .extend(flag.map(|flag| StatementData::storage_dead(flag, None)));
                self.blocks[next_block.0].statements.extend(
                    projection_flags
                        .into_iter()
                        .map(|flag| StatementData::storage_dead(flag, None)),
                );
            }
            self.current_block = Some(next_block);
            return;
        }

        self.emit_unconditional_drop_for_place(ty, place);

        if let Some(local) = storage_dead {
            if let Some(current) = self.current_block {
                self.blocks[current.0]
                    .statements
                    .push(StatementData::storage_dead(local, None));
                if let Some(flag) = self.drop_flag_for(local) {
                    self.blocks[current.0]
                        .statements
                        .push(StatementData::storage_dead(flag, None));
                }
                for flag in self.projection_drop_flags_for_local(local) {
                    self.blocks[current.0]
                        .statements
                        .push(StatementData::storage_dead(flag, None));
                }
            }
        }
    }

    fn emit_unconditional_drop_for_place(&mut self, ty: &Type, place: Place) {
        if self.place_was_statically_moved_for_cleanup(&place) {
            return;
        }

        if self.has_direct_drop_impl(ty) && !self.place_has_moved_descendant(&place) {
            let next_block = self.new_block();
            self.record_drop_obligation(
                place.clone(),
                self.type_id_for(ty),
                DropObligationKind::Direct,
            );
            if let Some(current) = self.current_block {
                self.blocks[current.0].terminator = Some(Terminator::drop(
                    place.clone(),
                    next_block,
                    self.drop_origin(&place),
                ));
            }
            self.current_block = Some(next_block);
        }

        self.emit_child_drop_chain(ty, place);
    }

    fn has_direct_drop_impl(&self, ty: &Type) -> bool {
        self.direct_drop_types.contains(ty)
    }

    fn emit_child_drop_chain(&mut self, ty: &Type, place: Place) {
        match ty {
            Type::Struct { id, args } => {
                self.record_drop_obligation(
                    place.clone(),
                    self.type_id_for(ty),
                    DropObligationKind::Structural,
                );
                self.emit_struct_field_drop_chain(*id, args, place);
            }
            Type::Tuple(elems) => {
                self.record_drop_obligation(
                    place.clone(),
                    self.type_id_for(ty),
                    DropObligationKind::Structural,
                );
                self.emit_tuple_drop_chain(elems, place);
            }
            Type::Array(elem, len) => {
                self.record_drop_obligation(
                    place.clone(),
                    self.type_id_for(ty),
                    DropObligationKind::Structural,
                );
                self.emit_array_drop_chain(elem, *len, place);
            }
            Type::Enum { id, args } => {
                self.record_drop_obligation(
                    place.clone(),
                    self.type_id_for(ty),
                    DropObligationKind::Structural,
                );
                self.emit_enum_payload_drop_chain(*id, args, place);
            }
            _ => {}
        }
    }

    fn emit_struct_field_drop_chain(&mut self, id: crate::ids::DefId, args: &[Type], place: Place) {
        let field_entries = Self::struct_field_types_with_substitution(self.program, id, args);
        for (i, (_field_id, field_ty)) in field_entries.iter().enumerate().rev() {
            if self.type_needs_cleanup(field_ty) {
                let mut projection = place.projection.clone();
                projection.push(Projection::Field {
                    index: i,
                    identity: None,
                });
                let field_place = Place {
                    local: place.local,
                    projection,
                };
                if !self.place_may_be_moved(&field_place)
                    || self.drop_flag_for_place(&field_place).is_some()
                {
                    self.emit_drop_for_place(field_ty, field_place, None);
                }
            }
        }
    }

    fn emit_tuple_drop_chain(&mut self, elems: &[Type], place: Place) {
        for (i, elem_ty) in elems.iter().enumerate().rev() {
            if self.type_needs_cleanup(elem_ty) {
                let mut projection = place.projection.clone();
                projection.push(Projection::Field {
                    index: i,
                    identity: None,
                });
                let field_place = Place {
                    local: place.local,
                    projection,
                };
                if !self.place_may_be_moved(&field_place)
                    || self.drop_flag_for_place(&field_place).is_some()
                {
                    self.emit_drop_for_place(elem_ty, field_place, None);
                }
            }
        }
    }

    fn emit_array_drop_chain(&mut self, elem_ty: &Type, len: usize, place: Place) {
        if !self.type_needs_cleanup(elem_ty) {
            return;
        }

        for index in (0..len).rev() {
            let index_local = self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
            if let Some(current) = self.current_block {
                self.blocks[current.0]
                    .statements
                    .push(StatementData::storage_live(index_local, None));
                self.blocks[current.0]
                    .statements
                    .push(StatementData::assign(
                        Place {
                            local: index_local,
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Constant(Constant::Int(index as i64))),
                        None,
                    ));
            }
            let mut projection = place.projection.clone();
            projection.push(Projection::Index(index_local));
            let elem_place = Place {
                local: place.local,
                projection,
            };
            if !self.place_may_be_moved(&elem_place)
                || self.drop_flag_for_place(&elem_place).is_some()
            {
                self.emit_drop_for_place(elem_ty, elem_place, None);
            }
            if let Some(current) = self.current_block {
                self.blocks[current.0]
                    .statements
                    .push(StatementData::storage_dead(index_local, None));
            }
        }
    }

    fn emit_enum_payload_drop_chain(&mut self, id: crate::ids::DefId, args: &[Type], place: Place) {
        let Some((_, enumeration)) = self.program.enum_by_id(id) else {
            return;
        };

        let generic_subst = Self::generic_substitution_for_fields(
            &enumeration
                .variants
                .iter()
                .flat_map(|variant| {
                    Self::enum_variant_field_types_for(self.program, id, variant.id)
                })
                .collect::<Vec<_>>(),
            &Type::Enum {
                id,
                args: args.to_vec(),
            },
        );

        let variant_fields = enumeration
            .variants
            .iter()
            .map(|variant| {
                let fields = Self::enum_variant_field_types_for(self.program, id, variant.id)
                    .into_iter()
                    .map(|ty| ty.substitute_generics(&generic_subst))
                    .collect::<Vec<_>>();
                (variant.id, fields)
            })
            .filter(|(_, fields)| fields.iter().any(|field| self.type_needs_cleanup(field)))
            .collect::<Vec<_>>();
        if variant_fields.is_empty() {
            return;
        }

        let discr_local = self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
        if let Some(current) = self.current_block {
            self.blocks[current.0]
                .statements
                .push(StatementData::storage_live(discr_local, None));
            self.blocks[current.0]
                .statements
                .push(StatementData::cleanup_assign(
                    Place {
                        local: discr_local,
                        projection: vec![],
                    },
                    Rvalue::Discriminant(place.clone()),
                    None,
                ));
        }

        let done_block = self.new_block();
        let variant_blocks = variant_fields
            .iter()
            .map(|_| self.new_block())
            .collect::<Vec<_>>();
        if let Some(current) = self.current_block {
            self.blocks[current.0].terminator = Some(Terminator::switch_int(
                Operand::Copy(Place {
                    local: discr_local,
                    projection: vec![],
                }),
                variant_fields
                    .iter()
                    .zip(variant_blocks.iter())
                    .map(|((variant_id, _), block)| (variant_id.0 as i64, *block))
                    .collect(),
                done_block,
                self.cleanup_origin(&place),
            ));
        }

        for ((variant_id, fields), block) in variant_fields.into_iter().zip(variant_blocks) {
            self.current_block = Some(block);
            for (index, field_ty) in fields.iter().enumerate().rev() {
                if !self.type_needs_cleanup(field_ty) {
                    continue;
                }
                let mut projection = place.projection.clone();
                projection.push(Projection::Downcast(variant_id));
                projection.push(Projection::Field {
                    index,
                    identity: None,
                });
                let field_place = Place {
                    local: place.local,
                    projection,
                };
                if !self.place_may_be_moved(&field_place)
                    || self.drop_flag_for_place(&field_place).is_some()
                {
                    self.emit_drop_for_place(field_ty, field_place, None);
                }
            }
            if let Some(current) = self.current_block {
                self.blocks[current.0].terminator =
                    Some(Terminator::goto(done_block, self.cleanup_origin(&place)));
            }
        }

        self.current_block = Some(done_block);
        if let Some(current) = self.current_block {
            self.blocks[current.0]
                .statements
                .push(StatementData::storage_dead(discr_local, None));
        }
    }

    pub(super) fn type_needs_cleanup(&self, ty: &Type) -> bool {
        if self.has_direct_drop_impl(ty) {
            return true;
        }

        match ty {
            Type::Struct { id, args } => {
                Self::struct_field_types_with_substitution(self.program, *id, args)
                    .iter()
                    .any(|(_, field_ty)| self.type_needs_cleanup(field_ty))
            }
            Type::Enum { id, args } => {
                self.program
                    .enum_by_id(*id)
                    .is_some_and(|(_, enumeration)| {
                        let generic_subst = Self::generic_substitution_for_fields(
                            &enumeration
                                .variants
                                .iter()
                                .flat_map(|variant| {
                                    Self::enum_variant_field_types_for(
                                        self.program,
                                        *id,
                                        variant.id,
                                    )
                                })
                                .collect::<Vec<_>>(),
                            &Type::Enum {
                                id: *id,
                                args: args.to_vec(),
                            },
                        );
                        enumeration.variants.iter().any(|variant| {
                            Self::enum_variant_field_types_for(self.program, *id, variant.id)
                                .into_iter()
                                .map(|field_ty| field_ty.substitute_generics(&generic_subst))
                                .any(|field_ty| self.type_needs_cleanup(&field_ty))
                        })
                    })
            }
            Type::Array(elem, _) => self.type_needs_cleanup(elem),
            Type::Tuple(elems) => elems.iter().any(|elem| self.type_needs_cleanup(elem)),
            Type::Function { .. } => true,
            _ => false,
        }
    }

    pub(super) fn scope_cleanup_block(
        &mut self,
        block_locals: Vec<crate::mir::Local>,
        target: crate::mir::BasicBlockId,
    ) -> crate::mir::BasicBlockId {
        let previous_block = self.current_block;
        let cleanup_block = self.new_block();
        self.current_block = Some(cleanup_block);
        self.finish_scope_locals(block_locals);
        if let Some(current) = self.current_block {
            self.blocks[current.0].terminator = Some(Terminator::goto(
                target,
                crate::mir::MirOrigin::synthetic(crate::mir::MirSyntheticOrigin::Cleanup, None),
            ));
        }
        self.current_block = previous_block;
        cleanup_block
    }

    pub(super) fn active_cleanup_locals_from(&self, depth: usize) -> Vec<crate::mir::Local> {
        self.scope_locals
            .iter()
            .skip(depth)
            .flat_map(|locals| locals.iter().copied())
            .collect()
    }

    pub(super) fn terminate_with_cleanup(&mut self, cleanup_depth: usize, terminator: Terminator) {
        let cleanup_locals = self.active_cleanup_locals_from(cleanup_depth);
        self.finish_scope_locals(cleanup_locals);
        if let Some(current) = self.current_block {
            self.blocks[current.0].terminator = Some(terminator);
            self.current_block = None;
        }
    }

    pub(super) fn lower_block(&mut self, block: &HirBlock, dest: Place) {
        self.scope_locals.push(Vec::new());
        let mut restored_bindings = Vec::new();
        let stmts_len = block.stmts.len();

        for (i, stmt) in block.stmts.iter().enumerate() {
            let is_last = i == stmts_len - 1;
            match stmt {
                HirStmt::Let {
                    name,
                    local_id,
                    ty,
                    value,
                    mutable,
                    ..
                } => {
                    let mutability = if *mutable {
                        Mutability::Mut
                    } else {
                        Mutability::Not
                    };
                    let local = self.new_local_with_source(
                        self.type_id_for(ty),
                        mutability,
                        Some(name.clone()),
                        LocalSource::UserBinding,
                        self.source_local_span(self.current_source_owner, *local_id),
                    );
                    if let Some(scope_locals) = self.scope_locals.last_mut() {
                        scope_locals.push(local);
                    }

                    let place = Place {
                        local,
                        projection: vec![],
                    };

                    self.emit_storage_live(local, Some(value.span.clone()));
                    self.scope_locals.push(Vec::new());
                    self.lower_expr(value, place);
                    let temp_locals = self.scope_locals.pop().unwrap_or_default();
                    self.finish_scope_locals(temp_locals);
                    // Initializers still see the outer binding. Restore it
                    // when this lexical block ends, including after shadowing.
                    restored_bindings
                        .push((name.clone(), self.var_map.insert(name.clone(), local)));
                }
                HirStmt::Expr(expr) => {
                    if is_last && !matches!(block.ty, Type::Unit) {
                        self.scope_locals.push(Vec::new());
                        self.lower_expr(expr, dest.clone());
                        let temp_locals = self.scope_locals.pop().unwrap_or_default();
                        self.finish_scope_locals(temp_locals);
                    } else {
                        let local = self.new_local_from_expr(expr.ty.clone(), expr);
                        self.emit_storage_live(local, Some(expr.span.clone()));

                        let place = Place {
                            local,
                            projection: vec![],
                        };
                        self.scope_locals.push(Vec::new());
                        if let Some(scope_locals) = self.scope_locals.last_mut() {
                            scope_locals.push(local);
                        }
                        self.lower_expr(expr, place);
                        let temp_locals = self.scope_locals.pop().unwrap_or_default();
                        self.finish_scope_locals(temp_locals);
                    }
                }
                HirStmt::Return(expr) => {
                    if let Some(e) = expr {
                        let ret_place = Place {
                            local: crate::mir::Local(0),
                            projection: vec![],
                        };
                        self.scope_locals.push(Vec::new());
                        self.lower_expr(e, ret_place);
                        let temp_locals = self.scope_locals.pop().unwrap_or_default();
                        self.finish_scope_locals(temp_locals);
                    }
                    let return_span = self.source_operation_span(
                        self.current_source_owner,
                        expr.as_ref().map(|expr| expr.span.clone()),
                    );
                    self.terminate_with_cleanup(
                        0,
                        Terminator::return_with_origin(self.source_origin(return_span)),
                    );
                }
                HirStmt::Break(expr) => {
                    if let Some(loop_targets) = self.loop_stack.last().copied() {
                        let branch_span = self.source_operation_span(
                            self.current_source_owner,
                            expr.as_ref().map(|expr| expr.span.clone()),
                        );
                        self.terminate_with_cleanup(
                            loop_targets.cleanup_depth,
                            Terminator::goto(
                                loop_targets.break_target,
                                self.source_origin(branch_span),
                            ),
                        );
                    }
                }
                HirStmt::Continue => {
                    if let Some(loop_targets) = self.loop_stack.last().copied() {
                        self.terminate_with_cleanup(
                            loop_targets.cleanup_depth,
                            Terminator::goto(
                                loop_targets.continue_target,
                                self.source_origin(None),
                            ),
                        );
                    }
                }
            }
        }

        let block_locals = self.scope_locals.pop().unwrap_or_default();
        self.finish_scope_locals(block_locals);
        for (name, previous) in restored_bindings.into_iter().rev() {
            if let Some(local) = previous {
                self.var_map.insert(name, local);
            } else {
                self.var_map.remove(&name);
            }
        }
    }
}
