use std::collections::HashSet;

use crate::lexer::Span;
use crate::types::Type;

use crate::hir::AcceptedHir;
use crate::mir::builder::{HirExpr, HirExprKind, MatchScrutineeMode, MirBuilder};
use crate::mir::{Mutability, Operand, Place, Projection, Rvalue, Terminator};

impl<'a> MirBuilder<'a> {
    pub(super) fn lower_match_expr(
        &mut self,
        expr: &HirExpr,
        dest: Place,
        span: Option<Span>,
        scrutinee: &HirExpr,
        arms: &[crate::hir::HirMatchArmFor<AcceptedHir>],
    ) {
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
            let discr_temp = self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
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
                let variant_success = if Self::enum_payload_patterns_irrefutable(&arm.pattern) {
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
                let guard_false_cleanup = self.scope_cleanup_block(cleanup_locals, next_block);
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
}
