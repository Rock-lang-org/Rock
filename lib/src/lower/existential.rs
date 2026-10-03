//! Scoped descriptor/dictionary authority for hidden object witnesses.
use crate::ast;
use crate::hir::*;
use crate::types::{Type, WitnessId};

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
        if let Type::Exists(package) = &source.ty {
            if package.binder.kind != crate::type_services::kind::Kind::Type
                || package.binder.maybe_unsized
            {
                self.diagnostics.push_type_with_span(
                    "opening requires an implemented Type-kind Sized package witness".into(),
                    source.span.clone(),
                );
                return self.error_expression_at(open.span.clone());
            }
            let value_ty = match crate::type_services::substitution::instantiate_existential(
                package,
                &Type::Witness(witness),
            ) {
                Ok(package) => package.payload,
                Err(error) => {
                    self.diagnostics
                        .push_type_with_span(error.to_string(), source.span.clone());
                    return self.error_expression_at(open.span.clone());
                }
            };
            let binding = HirPackageOpenBinding {
                witness,
                value: HirParam {
                    name: open.value.name.clone(),
                    local_id: value_id,
                    is_ref: matches!(value_ty, Type::Reference { .. }),
                    ty: value_ty.clone(),
                    mutable: false,
                },
                source_ty: source.ty.clone(),
            };
            self.push_scope();
            self.source_map.insert_local_in_scope(
                owner_id,
                value_id,
                open.value.span.clone(),
                self.source_scope_stack.last().copied(),
            );
            self.scope
                .define_local(open.value.name.clone(), value_ty, false, value_id);
            self.opened_packages
                .push((open.witness.name.clone(), binding.clone()));
            self.engine.enter_witness_scope(witness);
            let body = self.lower_block_expected(&open.body, expected);
            self.engine.exit_witness_scope(witness);
            self.opened_packages.pop();
            self.pop_scope();
            let ty = self.engine.resolve(&body.ty);
            if crate::type_services::visit::type_any(
                &ty,
                |ty| matches!(ty, Type::Witness(id) if *id == witness),
            ) {
                self.diagnostics.push_type_with_span(
                    "opened package witness escapes; repackage the actual payload".into(),
                    open.witness.span.clone(),
                );
            }
            return HirExpr {
                ty,
                span: open.span.clone(),
                kind: HirExprKind::PackageOpen {
                    source: Box::new(source),
                    binding,
                    body,
                },
            };
        }
        if matches!(&source.ty, Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::Exists(_)))
        {
            self.diagnostics.push_type_with_span(
                "borrowed-package opening is not implemented; open an existing package by value"
                    .into(),
                source.span.clone(),
            );
            return self.error_expression_at(open.span.clone());
        }
        let (object, value_ty, owner) = match self.open_handle(&source.ty, witness) {
            Ok(handle) => handle,
            Err(message) => {
                self.diagnostics
                    .push_type_with_span(message, source.span.clone());
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
        self.source_map.insert_local_in_scope(
            owner_id,
            value_id,
            open.value.span.clone(),
            self.source_scope_stack.last().copied(),
        );
        self.scope
            .define_local(open.value.name.clone(), value_ty, false, value_id);
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
                    return Err(
                        "open requires a borrowed object or a protocol-backed object owner".into(),
                    );
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
                    return Err(
                        "open requires a borrowed object or a protocol-backed object owner".into(),
                    );
                };
                let value_ty = Type::Struct {
                    id: *id,
                    args: vec![Type::Witness(witness)],
                };
                let owner = crate::hir::existential::prove_open_owner(
                    source,
                    &value_ty,
                    witness,
                    &self.selection_service(),
                    &self.language_items,
                )?;
                Ok((object.as_ref().clone(), value_ty, Some(owner)))
            }
            _ => Err("open requires a borrowed object or a protocol-backed object owner".into()),
        }
    }

    pub(crate) fn opened_object_schema(
        &self,
        witness: WitnessId,
    ) -> Option<crate::types::ObjectType> {
        self.opened_packages
            .iter()
            .rev()
            .find(|(_, binding)| binding.witness == witness)
            .and_then(|(_, binding)| binding.object_schema().ok())
            .or_else(|| {
                self.opened_witnesses
                    .iter()
                    .rev()
                    .find(|(_, binding)| binding.witness == witness)
                    .map(|(_, binding)| binding.object.clone())
            })
    }

    pub(crate) fn opened_witness_for_name(&self, name: &str) -> Option<WitnessId> {
        self.opened_witnesses
            .iter()
            .map(|(name, binding)| (name, binding.witness))
            .chain(
                self.opened_packages
                    .iter()
                    .map(|(name, binding)| (name, binding.witness)),
            )
            .filter(|(binder, _)| binder.as_str() == name)
            .max_by_key(|(_, witness)| witness.local.0)
            .map(|(_, witness)| witness)
    }

    pub(crate) fn lower_pack(&mut self, pack: &ast::Pack) -> HirExpr {
        let target = self.lower_parse_type(&pack.target);
        let Type::Exists(package) = &target else {
            if target != Type::Error {
                self.diagnostics.push_type_with_span(
                    "owner-target pack/destination construction is not implemented".into(),
                    pack.target.span(),
                );
            }
            return self.error_expression_at(pack.span.clone());
        };
        let mut payload = self.lower_expression(&pack.payload);
        payload.ty = self.engine.resolve(&payload.ty);
        let mut env = crate::type_services::normalize::TypeNormalizationEnv::new();
        crate::type_lowering::TypeLoweringContext::populate_type_normalization_env(self, &mut env);
        let proof = (|| {
            let witness = crate::type_services::existential::infer_pending_witness(
                package,
                &payload.ty,
                &env,
            )?;
            let instantiated =
                crate::type_services::substitution::instantiate_existential(package, &witness)
                    .map_err(|e| e.to_string())?;
            self.engine
                .unify_invariant(&instantiated.payload, &payload.ty)
                .map_err(|_| {
                    format!(
                        "package payload mismatch: expected {}, found {}",
                        instantiated.payload, payload.ty
                    )
                })?;
            payload.ty = self.engine.resolve(&payload.ty);
            let witness = self.engine.resolve(&witness);
            let instantiated =
                crate::type_services::substitution::instantiate_existential(package, &witness)
                    .map_err(|e| e.to_string())?;
            if !crate::type_services::visit::type_any(&witness, |ty| matches!(ty, Type::Witness(_)))
            {
                for bound in std::iter::once(instantiated.bounds.principal)
                    .chain(instantiated.bounds.guarantees)
                {
                    self.constraint_store.add_trait(
                        witness.clone(),
                        bound,
                        payload.span.clone(),
                        "existential package witness",
                    );
                }
                if let Some(sized) = &self.language_items.sized {
                    self.constraint_store.add_trait(
                        witness.clone(),
                        crate::types::TraitBound {
                            trait_id: sized.trait_id,
                            type_args: Vec::new(),
                        },
                        payload.span.clone(),
                        "existential package Sized witness",
                    );
                }
            }
            let opened = if let Type::Witness(witness) = &witness {
                self.opened_object_schema(*witness)
            } else {
                None
            };
            let structs = self
                .items
                .structures()
                .map(|(id, definition)| (id, definition.clone()))
                .collect();
            let enums = self
                .items
                .enumerations()
                .map(|(id, definition)| (id, definition.clone()))
                .collect();
            let context = crate::infer::object_coercion::ObjectEvidenceContext {
                traits: self.items.traits_for_selection(),
                impls: self.items.impls_for_selection(),
                structs: &structs,
                enums: &enums,
                language_items: &self.language_items,
                bounds: self.current_impl_bounds(),
            };
            match crate::hir::packages::prove_pack(&context, package, &witness, opened.as_ref()) {
                Ok(proof) => Ok(proof),
                Err(_)
                    if crate::type_services::visit::type_any(&witness, |ty| {
                        matches!(ty, Type::TypeVar(_) | Type::Generic(_))
                    }) =>
                {
                    Ok(HirPackagePack {
                        witness,
                        views: Vec::new(),
                    })
                }
                Err(message) => Err(message),
            }
        })();
        match proof {
            Ok(proof) => HirExpr {
                ty: target,
                span: pack.span.clone(),
                kind: HirExprKind::PackagePack {
                    payload: Box::new(payload),
                    proof,
                },
            },
            Err(message) => {
                self.diagnostics
                    .push_type_with_span(message, payload.span.clone());
                self.error_expression_at(pack.span.clone())
            }
        }
    }
}
