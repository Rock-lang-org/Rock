use crate::ids::DefId;
use crate::lexer::Span;
use crate::types::Type;

use crate::mir::{
    Local, LocalSource, MirClosureCapture, MirClosureCaptureKind, MirFunction, MirFunctionId,
    MirOwnershipMetadata, Mutability, Place, ReferenceOrigin,
};

use super::*;

impl<'a> MirBuilder<'a> {
    pub(super) fn build_function(
        &mut self,
        id: MirFunctionId,
        display_name: &str,
        func: &HirFunction,
        skip_self_param_cleanup: bool,
        source_owner: Option<DefId>,
        operation_span: Option<Span>,
    ) -> MirFunction {
        self.current_function_id = Some(id.clone());
        self.current_function_name = Some(display_name.to_string());
        self.current_source_owner = source_owner;
        self.drops.reset_for_function();
        self.ownership = MirOwnershipMetadata::default();
        let entry_block = self.new_block();
        self.current_block = Some(entry_block);

        let return_span = self.source_operation_span(source_owner, operation_span.clone());
        self.new_local_with_source(
            self.type_id_for(&func.ret_type),
            Mutability::Mut,
            Some("return_place".to_string()),
            LocalSource::ReturnPlace,
            return_span,
        );

        let mut function_scope_locals = Vec::new();
        let mut cleanup_param_locals = Vec::new();
        for (index, param) in func.params.iter().enumerate() {
            let mutability = if param.mutable {
                Mutability::Mut
            } else {
                Mutability::Not
            };
            let param_span = self.source_local_span(source_owner, param.local_id);
            let local = self.new_local_with_source(
                self.type_id_for(&param.ty),
                mutability,
                Some(param.name.clone()),
                LocalSource::Argument,
                param_span,
            );
            self.var_map.insert(param.name.clone(), local);
            let borrowed_self_param = index == 0
                && matches!(
                    func.self_receiver,
                    Some(crate::types::ReceiverMode::Shared | crate::types::ReceiverMode::Mut)
                );
            if borrowed_self_param || matches!(param.ty, Type::Reference { .. }) {
                self.record_reference_origin(local, ReferenceOrigin::Param(local));
            }
            if !(skip_self_param_cleanup && index == 0) && !borrowed_self_param {
                function_scope_locals.push(local);
                cleanup_param_locals.push(local);
            }
        }

        for local in cleanup_param_locals {
            self.ensure_drop_flag(local);
            self.mark_place_initialized(
                &Place {
                    local,
                    projection: vec![],
                },
                None,
            );
        }

        for capture in self.closures.take_pending_body_captures() {
            let mutability = if capture.mutable
                || capture.kind == crate::hir::HirClosureCaptureKind::MutableBorrow
            {
                Mutability::Mut
            } else {
                Mutability::Not
            };
            let capture_ty = match capture.kind {
                crate::hir::HirClosureCaptureKind::Move => capture.ty.clone(),
                crate::hir::HirClosureCaptureKind::SharedBorrow => Type::Reference {
                    mutable: false,
                    inner: Box::new(capture.ty.clone()),
                },
                crate::hir::HirClosureCaptureKind::MutableBorrow => Type::Reference {
                    mutable: true,
                    inner: Box::new(capture.ty.clone()),
                },
            };
            let capture_span = self
                .source_local_span(source_owner, capture.local_id)
                .or_else(|| operation_span.clone());
            let local = self.new_local_with_source(
                self.type_id_for(&capture_ty),
                mutability,
                Some(capture.name.clone()),
                LocalSource::ClosureCapture,
                capture_span.clone(),
            );
            self.var_map.insert(capture.name.clone(), local);
            self.closures.record_capture(MirClosureCapture {
                name: capture.name,
                local,
                kind: MirClosureCaptureKind::from(capture.kind),
                span: capture_span,
            });
        }

        let ret_place = Place {
            local: Local(0),
            projection: vec![],
        };

        self.scope_locals.push(function_scope_locals);
        self.lower_block(&func.body, ret_place);
        let function_scope_locals = self.scope_locals.pop().unwrap_or_default();
        self.finish_scope_locals(function_scope_locals);

        if let Some(current) = self.current_block {
            let implicit_return_origin = self.source_origin(
                self.source_operation_span(self.current_source_owner, operation_span.clone()),
            );
            let block = &mut self.blocks[current.0];
            if block.terminator.is_none() {
                block.terminator = Some(crate::mir::Terminator::return_with_origin(
                    implicit_return_origin,
                ));
            }
        }

        MirFunction {
            id,
            name: display_name.to_string(),
            basic_blocks: std::mem::take(&mut self.blocks),
            local_decls: std::mem::take(&mut self.locals),
            closure_captures: self.closures.take_captures(),
            arg_count: func.params.len(),
            ret_type: self.type_id_for(&func.ret_type),
            ownership: std::mem::take(&mut self.ownership),
        }
    }
}
