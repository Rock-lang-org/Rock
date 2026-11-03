use crate::lexer::Span;
use crate::types::Type;

use crate::mir::{Constant, Local, Operand, Place, Projection, Rvalue, StatementData};

use super::*;

impl<'a> MirBuilder<'a> {
    pub(super) fn emit_assign(&mut self, dest: Place, rvalue: Rvalue, span: Option<Span>) {
        self.record_rvalue_moves(&rvalue);
        self.record_reference_assignment(&dest, &rvalue);
        self.clear_moved_place(&dest);
        if let Some(current) = self.current_block {
            self.blocks[current.0]
                .statements
                .push(StatementData::assign(dest.clone(), rvalue, span.clone()));
        }
        self.mark_place_initialized(&dest, span);
    }

    fn emit_assert(&mut self, assertion: crate::mir::MirAssert, span: Option<Span>) {
        if let Some(current) = self.current_block {
            self.blocks[current.0]
                .statements
                .push(StatementData::assert(assertion, span));
        }
    }

    fn index_base_needs_bounds_check(base_ty: &Type) -> bool {
        !matches!(
            base_ty,
            Type::Pointer(inner) if !matches!(inner.as_ref(), Type::Slice(_) | Type::Str)
        )
    }

    pub(super) fn emit_bounds_check_for_index_place(
        &mut self,
        indexed_place: &Place,
        base_ty: &Type,
        span: Option<Span>,
    ) {
        if !Self::index_base_needs_bounds_check(base_ty) {
            return;
        }

        let Some(Projection::Index(index_local)) = indexed_place.projection.last().cloned() else {
            return;
        };
        let mut base_place = indexed_place.clone();
        base_place.projection.pop();
        self.emit_assert(
            crate::mir::MirAssert {
                kind: crate::mir::MirAssertKind::BoundsCheck,
                operands: vec![
                    Operand::Copy(base_place),
                    Operand::Copy(Place {
                        local: index_local,
                        projection: vec![],
                    }),
                ],
            },
            span,
        );
    }

    pub(super) fn emit_storage_live(&mut self, local: Local, span: Option<Span>) {
        let flag = self.ensure_drop_flag(local);
        let projection_flags = self.projection_drop_flags_for_local(local);
        if let Some(current) = self.current_block {
            self.blocks[current.0]
                .statements
                .push(StatementData::storage_live(local, span.clone()));
            if let Some(flag) = flag {
                self.blocks[current.0]
                    .statements
                    .push(StatementData::storage_live(flag, span.clone()));
                self.blocks[current.0]
                    .statements
                    .push(StatementData::assign(
                        Place {
                            local: flag,
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Constant(Constant::Bool(false))),
                        span.clone(),
                    ));
            }
            for projection_flag in projection_flags {
                self.blocks[current.0]
                    .statements
                    .push(StatementData::storage_live(projection_flag, span.clone()));
                self.blocks[current.0]
                    .statements
                    .push(StatementData::assign(
                        Place {
                            local: projection_flag,
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Constant(Constant::Bool(false))),
                        span.clone(),
                    ));
            }
        }
    }
}
