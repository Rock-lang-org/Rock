use crate::lexer::Span;
use crate::types::Type;

use crate::mir::builder::{HirBlock, HirExpr, HirExprKind, LoopTargets, MirBuilder};
use crate::mir::{Constant, Mutability, Operand, Place, Rvalue, Terminator};

impl<'a> MirBuilder<'a> {
    pub(super) fn lower_if_expr(
        &mut self,
        dest: Place,
        span: Option<Span>,
        condition: &HirExpr,
        then_branch: &HirBlock,
        else_branch: &Option<HirBlock>,
    ) {
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

    pub(super) fn lower_loop_expr(
        &mut self,
        expr: &HirExpr,
        dest: Place,
        borrow_context: bool,
        body: &HirBlock,
    ) {
        // Share the ordinary loop CFG and cleanup rules with while.
        let loop_expr = HirExpr {
            ty: expr.ty.clone(),
            span: expr.span.clone(),
            kind: HirExprKind::While {
                condition: Box::new(HirExpr {
                    ty: Type::Bool,
                    kind: HirExprKind::BoolLiteral(true),
                    span: expr.span.clone(),
                }),
                body: body.clone(),
            },
        };
        self.lower_expr_with_context(&loop_expr, dest, borrow_context);
    }

    pub(super) fn lower_while_expr(
        &mut self,
        dest: Place,
        span: Option<Span>,
        condition: &HirExpr,
        body: &HirBlock,
    ) {
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
        let cond_temp = self.new_local(self.type_id_for(&Type::Bool), Mutability::Not, None);
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
}
