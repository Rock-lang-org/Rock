use crate::types::Type;

use crate::mir::{
    BasicBlockId, MirClosureCaptureKind, Operand, Place, Rvalue, Terminator,
};

use super::*;

impl<'a> MirBuilder<'a> {
    fn record_operand_move(&mut self, operand: &Operand) {
        if let Operand::Move(place) = operand {
            if place.projection.is_empty() {
                self.set_drop_flag(place.local, false, None);
                self.set_projection_drop_flags_for_local(place.local, false, None);
                self.drops.record_local_moved(place.local);
            } else {
                self.set_drop_flags_for_place_tree(place, false, None);
                self.drops.record_projection_moved(place.clone());
            }
        }
    }

    pub(super) fn mark_place_moved_for_cleanup(&mut self, place: Place) {
        if place.projection.is_empty() {
            self.set_drop_flag(place.local, false, None);
            self.set_projection_drop_flags_for_local(place.local, false, None);
        } else {
            self.set_drop_flags_for_place_tree(&place, false, None);
        }
        self.drops.record_statically_moved_for_cleanup(place);
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
