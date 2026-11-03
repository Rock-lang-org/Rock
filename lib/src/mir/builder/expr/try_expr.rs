use crate::hir::{HirCallTarget, HirVariantLocation};
use crate::types::Type;

use crate::mir::builder::{HirExpr, HirExprKind, MirBuilder};
use crate::mir::{Constant, Mutability, Operand, Place, Projection, Rvalue, Terminator};

impl<'a> MirBuilder<'a> {
    pub(super) fn lower_try_expr(
        &mut self,
        carrier: &HirExpr,
        branch_target: &Option<HirCallTarget>,
        from_residual_target: &HirCallTarget,
        output_ty: &Type,
        residual_ty: &Type,
        return_ty: &Type,
        control_flow_enum: crate::ids::DefId,
        break_variant: &HirVariantLocation,
        continue_variant: &HirVariantLocation,
        dest: Place,
        span: Option<crate::lexer::Span>,
    ) {
        let control_flow_ty = Type::Enum {
            id: control_flow_enum,
            args: vec![residual_ty.clone(), output_ty.clone()],
        };
        let branch_temp = self.new_local_from_expr(control_flow_ty.clone(), carrier);
        self.register_scoped_temp(branch_temp);
        self.emit_storage_live(branch_temp, Some(carrier.span.clone()));
        let branch_place = Place {
            local: branch_temp,
            projection: vec![],
        };
        let branch_instance = match branch_target {
            Some(HirCallTarget::Instance(instance_id)) => *instance_id,
            target => {
                panic!("Try branch reached MIR without a materialized instance target: {target:?}")
            }
        };
        let branch_callee = HirExpr {
            ty: Type::function(vec![carrier.ty.clone()], control_flow_ty.clone()),
            kind: HirExprKind::ResolvedVar(crate::hir::HirVarRef {
                name: "branch".to_string(),
                target: crate::hir::HirVarTarget::Instance(branch_instance),
            }),
            span: carrier.span.clone(),
        };
        let branch_call = HirExpr {
            ty: control_flow_ty,
            kind: HirExprKind::Call(
                Box::new(branch_callee),
                vec![carrier.clone()],
                Some(HirCallTarget::Instance(branch_instance)),
            ),
            span: carrier.span.clone(),
        };
        self.lower_expr(&branch_call, branch_place.clone());

        let Some(current) = self.current_block else {
            return;
        };
        let discr_temp = self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
        self.register_scoped_temp(discr_temp);
        self.emit_storage_live(discr_temp, span.clone());
        let discr_place = Place {
            local: discr_temp,
            projection: vec![],
        };
        self.emit_assign(
            discr_place.clone(),
            Rvalue::Discriminant(branch_place.clone()),
            span.clone(),
        );

        let continue_block = self.new_block();
        let break_block = self.new_block();
        let merge_block = self.new_block();
        self.blocks[current.0].terminator = Some(Terminator::switch_int(
            Operand::Copy(discr_place),
            vec![(continue_variant.variant_id.0 as i64, continue_block)],
            break_block,
            self.source_origin(span.clone()),
        ));

        self.current_block = Some(break_block);
        let Some(from_residual_callable) =
            self.callable_for_call_target(from_residual_target, return_ty)
        else {
            panic!(
                "Try residual reached MIR without a materialized callable target: {from_residual_target:?}"
            );
        };
        let mut residual_place = branch_place.clone();
        residual_place
            .projection
            .push(Projection::Downcast(break_variant.variant_id));
        residual_place.projection.push(Projection::Field {
            index: 0,
            identity: None,
        });
        let residual_operand = self.operand_for_place(residual_ty, residual_place, false);
        let after_from_residual = self.new_block();
        if let Some(current) = self.current_block {
            self.set_terminator(
                current,
                Terminator::Call {
                    func: Operand::Constant(Constant::Callable(from_residual_callable)),
                    args: vec![residual_operand],
                    destination: Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    target: after_from_residual,
                    span: span.clone(),
                },
            );
        }
        self.current_block = Some(after_from_residual);
        let ret_place = Place {
            local: crate::mir::Local(0),
            projection: vec![],
        };
        self.mark_place_initialized(&ret_place, span.clone());
        self.terminate_with_cleanup(
            0,
            Terminator::return_with_origin(self.source_origin(span.clone())),
        );

        self.current_block = Some(continue_block);
        let mut output_place = branch_place;
        output_place
            .projection
            .push(Projection::Downcast(continue_variant.variant_id));
        output_place.projection.push(Projection::Field {
            index: 0,
            identity: None,
        });
        let output_operand = self.operand_for_place(output_ty, output_place, false);
        self.emit_assign(dest.clone(), Rvalue::Use(output_operand), span.clone());
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
        self.mark_place_initialized(&dest, span);
    }
}
