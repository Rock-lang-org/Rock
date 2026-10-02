//! Scoped descriptor/dictionary authority for hidden object witnesses.
use crate::ast;
use crate::hir::*;
use crate::types::{FunctionSafety, NominalTypeKind, Type, WitnessId};

use super::Lowerer;

impl Lowerer {
    pub(crate) fn lower_open(&mut self, open: &ast::Open) -> HirExpr {
        self.lower_open_expected(open, None)
    }

    pub(crate) fn lower_open_expected(
        &mut self,
        open: &ast::Open,
        expected: Option<&Type>,
    ) -> HirExpr {
        let mut source = self.lower_expression(&open.source);
        source.ty = self.engine.resolve(&source.ty);
        let Some(owner_id) = self.current_body_def_id() else {
            self.diagnostics
                .push_type_with_span("an opening requires a body scope".into(), open.span.clone());
            return self.error_expression_at(open.span.clone());
        };
        let value_id = self.fresh_local_id();
        let witness = WitnessId {
            owner: owner_id,
            local: value_id,
        };
        let (object, value_ty, owner) = match self.open_handle(&source.ty, witness) {
            Ok(handle) => handle,
            Err(message) => {
                self.diagnostics
                    .push_type_with_span(message, open.span.clone());
                return self.error_expression_at(open.span.clone());
            }
        };
        let binding = HirOpenBinding {
            witness,
            value: HirParam {
                name: open.value.name.clone(),
                local_id: value_id,
                is_ref: matches!(value_ty, Type::Reference { .. }),
                ty: value_ty.clone(),
                mutable: false,
            },
            object,
            source_ty: source.ty.clone(),
            owner,
        };
        self.push_scope();
        self.scope
            .define_local(&open.value.name, value_ty, false, value_id);
        self.opened_witnesses
            .push((open.witness.name.clone(), binding.clone()));
        self.engine.enter_witness_scope(witness);
        let body = self.lower_block_expected(&open.body, expected);
        self.engine.exit_witness_scope(witness);
        self.opened_witnesses.pop();
        self.pop_scope();
        let ty = self.engine.resolve(&body.ty);
        if crate::type_services::visit::type_any(
            &ty,
            |nested| matches!(nested, Type::Witness(id) if *id == witness),
        ) {
            self.diagnostics.push_type_with_span(
                format!("opened witness '{}' escapes through the block result; return an independent result or repackage it", open.witness.name),
                open.witness.span.clone(),
            );
        }
        HirExpr {
            ty,
            span: open.span.clone(),
            kind: HirExprKind::Open {
                source: Box::new(source),
                binding,
                body,
            },
        }
    }

    fn open_handle(
        &self,
        source: &Type,
        witness: WitnessId,
    ) -> Result<(crate::types::ObjectType, Type, Option<HirOpenOwner>), String> {
        match source {
            Type::Reference { mutable, inner } => {
                let Type::Object(object) = inner.as_ref() else {
                    return Err("open requires an object handle or existential package".into());
                };
                Ok((
                    object.as_ref().clone(),
                    Type::Reference {
                        mutable: *mutable,
                        inner: Box::new(Type::Witness(witness)),
                    },
                    None,
                ))
            }
            Type::Struct { id, args } => {
                let [Type::Object(object)] = args.as_slice() else {
                    return Err("open requires an object handle or existential package".into());
                };
                let protocol = self.language_items.object_owner.as_ref().ok_or(
                    "opening an owner requires an explicitly provided object_owner protocol",
                )?;
                let constructor = Type::Constructor {
                    id: *id,
                    flavor: NominalTypeKind::Struct,
                };
                let service = self.selection_service();
                let state = service
                    .infer_trait_arguments(&constructor, protocol.trait_id)
                    .ok_or("opening requires one proven owner State")?;
                let [state_ty] = state.as_slice() else {
                    return Err("invalid object_owner State arity".into());
                };
                if crate::type_services::visit::type_any(state_ty, |ty| {
                    matches!(
                        ty,
                        Type::Witness(_)
                            | Type::Generic(_)
                            | Type::TypeVar(_)
                            | Type::ObjectSelf { .. }
                    )
                }) || crate::type_services::layout::TypeLayout::sizedness(state_ty, &|_| {
                    crate::type_services::layout::Sizedness::Unknown
                }) != crate::type_services::layout::Sizedness::Sized
                {
                    return Err(
                        "owner State must be sized and independent of the hidden witness".into(),
                    );
                }
                let value_ty = Type::Struct {
                    id: *id,
                    args: vec![Type::Witness(witness)],
                };
                let (_, into_parts) = crate::infer::owner_coercion::select_operation(
                    &service,
                    &constructor,
                    protocol.trait_id,
                    &state,
                    protocol.into_parts_id,
                    &Type::function_with_safety(
                        vec![source.clone()],
                        Type::Tuple(vec![
                            Type::Pointer(Box::new(args[0].clone())),
                            state_ty.clone(),
                        ]),
                        FunctionSafety::Unsafe,
                    ),
                )?;
                let (_, from_parts) = crate::infer::owner_coercion::select_operation(
                    &service,
                    &constructor,
                    protocol.trait_id,
                    &state,
                    protocol.from_parts_id,
                    &Type::function_with_safety(
                        vec![
                            Type::Pointer(Box::new(Type::Witness(witness))),
                            state_ty.clone(),
                        ],
                        value_ty.clone(),
                        FunctionSafety::Unsafe,
                    ),
                )?;
                Ok((
                    object.as_ref().clone(),
                    value_ty,
                    Some(HirOpenOwner {
                        state_ty: state_ty.clone(),
                        into_parts,
                        from_parts,
                    }),
                ))
            }
            _ => Err("open requires an object handle or existential package".into()),
        }
    }
}
