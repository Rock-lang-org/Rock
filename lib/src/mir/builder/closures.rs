use crate::hir::HirClosureCapture;
use crate::lexer::Span;
use crate::types::Type;

use crate::mir::{
    LocalSource, MirClosureCapture, MirClosureCaptureKind, MirClosureId, MirFunctionId, Mutability, Place, Rvalue,
};

use super::*;

impl<'a> MirBuilder<'a> {
    pub(super) fn new_closure_id_and_name(&mut self) -> (MirClosureId, String) {
        let owner = self
            .current_function_id
            .clone()
            .expect("closure lowering requires current MIR function id");
        let (index, name) = self.closures.fresh_lambda();
        (
            MirClosureId {
                owner,
                local_index: index,
            },
            name,
        )
    }

    pub(super) fn lower_closure_captures(
        &mut self,
        captures: &[HirClosureCapture],
    ) -> Vec<MirClosureCapture> {
        captures
            .iter()
            .filter_map(|capture| {
                let mut local = self.var_map.get(&capture.name).copied()?;
                let span = self.locals.get(local.0).and_then(|decl| decl.span.clone());
                let mut kind = MirClosureCaptureKind::from(capture.kind);
                if let Some(place) = self.place_for_borrowed_binding_value(local, &capture.ty) {
                    // A transitive capture borrows the original value, not the
                    // enclosing closure's reference slot. Materialize that
                    // reborrow because MIR captures store locals, not places.
                    let (ty, value) = match kind {
                        MirClosureCaptureKind::ByValue => (
                            capture.ty.clone(),
                            Rvalue::Use(self.operand_for_place(&capture.ty, place, false)),
                        ),
                        MirClosureCaptureKind::ByRef | MirClosureCaptureKind::ByMutRef => {
                            let mutable = kind == MirClosureCaptureKind::ByMutRef;
                            (
                                Type::Reference {
                                    mutable,
                                    inner: Box::new(capture.ty.clone()),
                                },
                                Rvalue::Ref(
                                    if mutable {
                                        Mutability::Mut
                                    } else {
                                        Mutability::Not
                                    },
                                    place,
                                ),
                            )
                        }
                    };
                    local = self.new_local_with_source(
                        self.type_id_for(&ty),
                        Mutability::Not,
                        None,
                        LocalSource::Temporary,
                        span.clone(),
                    );
                    self.register_scoped_temp(local);
                    self.emit_storage_live(local, span.clone());
                    self.emit_assign(
                        Place {
                            local,
                            projection: vec![],
                        },
                        value,
                        span.clone(),
                    );
                    kind = MirClosureCaptureKind::ByValue;
                }
                Some(MirClosureCapture {
                    name: capture.name.clone(),
                    local,
                    kind,
                    span,
                })
            })
            .collect()
    }

    pub(super) fn push_closure_body_function(
        &mut self,
        closure_id: MirClosureId,
        lambda_name: &str,
        params: Vec<crate::hir::HirParam>,
        body: HirBlock,
        captures: Vec<HirClosureCapture>,
        operation_span: Span,
    ) {
        let hir_id = match &closure_id.owner {
            MirFunctionId::Function(id) | MirFunctionId::Extern(id) => *id,
            MirFunctionId::Instance(_) | MirFunctionId::Closure(_)
            | MirFunctionId::ArrayTailDrop(_) => {
                crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0))
            }
        };
        let function = HirFunction {
            id: hir_id,
            name: lambda_name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: crate::hir::HirGenericBounds::new(),
            params,
            ret_type: body.ty.clone(),
            body,
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };

        let mut builder = MirBuilder::with_instance_metadata(
            self.program,
            self.type_context,
            self.method_instance_receiver_modes.clone(),
            self.direct_drop_types.clone(),
        )
        .with_source_map(&self.source_map);
        builder.callable_instances_by_def_id = self.callable_instances_by_def_id.clone();
        builder.method_def_ids = self.method_def_ids.clone();
        builder.closures.set_pending_body_captures(captures);
        let function_id = MirFunctionId::Closure(Box::new(closure_id));
        let mir_func = builder.build_function(
            function_id,
            lambda_name,
            &function,
            false,
            self.current_source_owner,
            Some(operation_span),
        );
        self.closures.push_nested(mir_func);
        self.closures.extend_nested(builder.closures.take_nested());
    }
}
