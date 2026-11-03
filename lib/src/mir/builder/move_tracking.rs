use crate::types::Type;

use crate::mir::{
    BasicBlockId, MirClosureCaptureKind, Operand, Place, Projection, Rvalue, Terminator,
};

use super::*;

impl<'a> MirBuilder<'a> {
    fn record_operand_move(&mut self, operand: &Operand) {
        if let Operand::Move(place) = operand {
            if place.projection.is_empty() {
                self.set_drop_flag(place.local, false, None);
                self.set_projection_drop_flags_for_local(place.local, false, None);
                self.moved_places.retain(|moved| moved.local != place.local);
            } else {
                self.set_drop_flags_for_place_tree(place, false, None);
                self.moved_places.insert(place.clone());
            }
        }
    }

    pub(super) fn clear_moved_place(&mut self, place: &Place) {
        if place.projection.is_empty() {
            self.moved_places.retain(|moved| moved.local != place.local);
            self.statically_moved_cleanup_places
                .retain(|moved| moved.local != place.local);
        } else {
            self.moved_places
                .retain(|moved| !Self::place_projection_is_prefix(place, moved));
            self.statically_moved_cleanup_places
                .retain(|moved| !Self::place_projection_is_prefix(place, moved));
        }
    }

    pub(super) fn mark_place_moved_for_cleanup(&mut self, place: Place) {
        if place.projection.is_empty() {
            self.set_drop_flag(place.local, false, None);
            self.set_projection_drop_flags_for_local(place.local, false, None);
            self.moved_places.retain(|moved| moved.local != place.local);
        } else {
            self.set_drop_flags_for_place_tree(&place, false, None);
            self.moved_places
                .retain(|moved| !Self::place_projection_is_prefix(&place, moved));
        }
        self.moved_places.insert(place.clone());
        self.statically_moved_cleanup_places.insert(place);
    }

    pub(super) fn place_may_be_moved(&self, place: &Place) -> bool {
        self.moved_places
            .iter()
            .any(|moved| Self::place_projection_is_prefix(moved, place))
    }

    pub(super) fn place_was_statically_moved_for_cleanup(&self, place: &Place) -> bool {
        self.statically_moved_cleanup_places
            .iter()
            .any(|moved| Self::place_projection_is_prefix(moved, place))
    }

    pub(super) fn place_has_moved_descendant(&self, place: &Place) -> bool {
        self.moved_places
            .iter()
            .any(|moved| Self::place_projection_is_strict_prefix(place, moved))
    }

    pub(super) fn place_projection_is_prefix(prefix: &Place, place: &Place) -> bool {
        prefix.local == place.local
            && Self::projection_is_prefix(&prefix.projection, &place.projection)
    }

    fn place_projection_is_strict_prefix(prefix: &Place, place: &Place) -> bool {
        prefix.local == place.local
            && prefix.projection.len() < place.projection.len()
            && Self::projection_is_prefix(&prefix.projection, &place.projection)
    }

    pub(super) fn projection_is_prefix(prefix: &[Projection], projection: &[Projection]) -> bool {
        prefix.len() <= projection.len()
            && prefix
                .iter()
                .zip(projection.iter())
                .all(|(lhs, rhs)| Self::projection_component_matches(lhs, rhs))
    }

    fn projection_component_matches(lhs: &Projection, rhs: &Projection) -> bool {
        match (lhs, rhs) {
            (Projection::Deref, Projection::Deref) => true,
            (Projection::Downcast(lhs), Projection::Downcast(rhs)) => lhs == rhs,
            (Projection::Index(lhs), Projection::Index(rhs)) => lhs == rhs,
            (Projection::Field { index: lhs, .. }, Projection::Field { index: rhs, .. }) => {
                lhs == rhs
            }
            _ => false,
        }
    }

    pub(super) fn move_source_place_for_expr(&self, expr: &HirExpr) -> Option<Place> {
        match &expr.kind {
            HirExprKind::Var(name) => self.var_map.get(name).copied().map(|local| Place {
                local,
                projection: vec![],
            }),
            HirExprKind::ResolvedVar(reference) => {
                self.var_map
                    .get(&reference.name)
                    .copied()
                    .map(|local| Place {
                        local,
                        projection: vec![],
                    })
            }
            HirExprKind::FieldAccess(base, _, location) => {
                let mut place = self.move_source_place_for_expr(base)?;
                let location = location.as_ref()?;
                let Type::Struct { id, .. } = &base.ty else {
                    return None;
                };
                let (_, structure) = self.program.struct_by_id(*id)?;
                let index = structure
                    .fields
                    .iter()
                    .position(|field| field.id == location.field_id)?;
                place.projection.push(self.field_projection(
                    index,
                    Some(location.owner),
                    Some(location.field_id),
                ));
                Some(place)
            }
            HirExprKind::TupleIndex(base, index) => {
                let mut place = self.move_source_place_for_expr(base)?;
                place
                    .projection
                    .push(self.field_projection(*index as usize, None, None));
                Some(place)
            }
            _ => None,
        }
    }

    pub(super) fn record_rvalue_moves(&mut self, rvalue: &Rvalue) {
        match rvalue {
            Rvalue::Use(operand) | Rvalue::Cast(operand, _) | Rvalue::UnaryOp(_, operand) => {
                self.record_operand_move(operand);
            }
            Rvalue::BinaryOp(_, lhs, rhs) => {
                self.record_operand_move(lhs);
                self.record_operand_move(rhs);
            }
            Rvalue::Aggregate(_, operands) => {
                for operand in operands {
                    self.record_operand_move(operand);
                }
            }
            Rvalue::Closure(closure) => {
                for capture in &closure.captures {
                    if capture.kind == MirClosureCaptureKind::ByValue
                        && capture.place().projection.is_empty()
                    {
                        self.set_drop_flag(capture.local, false, capture.span.clone());
                    }
                }
            }
            Rvalue::Ref(_, _) | Rvalue::Discriminant(_) => {}
        }
    }

    fn record_terminator_moves(&mut self, terminator: &Terminator) {
        if let Terminator::Call { func, args, .. } = terminator {
            self.record_operand_move(func);
            for arg in args {
                self.record_operand_move(arg);
            }
        }
    }

    pub(super) fn set_terminator(&mut self, block: BasicBlockId, terminator: Terminator) {
        self.record_terminator_moves(&terminator);
        self.blocks[block.0].terminator = Some(terminator);
    }
}
