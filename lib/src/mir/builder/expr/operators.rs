use crate::hir::BinOp;
use crate::types::Type;

use crate::mir::builder::{HirExpr, MirBuilder};
use crate::mir::{Constant, Operand, Place, Rvalue, Terminator};

impl<'a> MirBuilder<'a> {
    pub(super) fn lower_short_circuit_binop(
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
}
