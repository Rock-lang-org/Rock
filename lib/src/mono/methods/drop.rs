use crate::ids::DefId;
use crate::types::Type;

use crate::mono::Monomorphizer;

fn drop_diagnostic(
    message: String,
    origin_span: Option<&crate::lexer::Span>,
) -> crate::diagnostic::Diagnostic {
    match origin_span {
        Some(span) => crate::diagnostic::Diagnostic::new(message, span.clone())
            .with_code(crate::diagnostic::DiagnosticCode::Mono),
        None => crate::diagnostic::Diagnostic::for_internal(message)
            .with_code(crate::diagnostic::DiagnosticCode::Mono),
    }
}

impl Monomorphizer {
    pub(in crate::mono) fn monomorphize_drop_for_type(
        &mut self,
        ty: &Type,
        origin_span: Option<crate::lexer::Span>,
    ) {
        let origin_span = origin_span.or_else(|| self.source_span_for_type(ty));
        self.monomorphize_drop_fields_for_type(ty);

        let Some(drop_trait_id) = self.drop_trait_id() else {
            return;
        };
        let Some(drop_method_id) = self.drop_method_id else {
            return;
        };
        let mut matching_impls = self
            .trait_impls
            .get(&drop_trait_id)
            .into_iter()
            .flat_map(|impls| impls.iter())
            .filter(|imp| imp.trait_id == Some(drop_trait_id))
            .filter(|imp| self.drop_receiver_matches(imp, ty))
            .cloned()
            .collect::<Vec<_>>();
        matching_impls.sort_by_key(|imp| imp.id);
        matching_impls.dedup_by_key(|imp| imp.id);
        let [imp] = matching_impls.as_slice() else {
            if matching_impls.len() > 1 {
                self.diagnostics.push(drop_diagnostic(
                    format!(
                        "ambiguous Drop implementation for type {}",
                        self.display_type_for_diagnostic(ty)
                    ),
                    origin_span.as_ref(),
                ));
            }
            return;
        };
        let Some(method_id) = self
            .effective_trait_methods
            .get(&(imp.id, drop_method_id))
            .copied()
        else {
            self.diagnostics.push(drop_diagnostic(
                format!("selected Drop implementation has no resolved @drop method",),
                origin_span.as_ref(),
            ));
            return;
        };
        let Some(method) = imp
            .methods
            .values()
            .find(|method| method.id == method_id)
            .cloned()
        else {
            self.diagnostics.push(drop_diagnostic(
                format!("selected Drop implementation resolves @drop to a missing method",),
                origin_span.as_ref(),
            ));
            return;
        };

        let type_args = if !imp.type_generics.is_empty() || !method.generic_params.is_empty() {
            let Some(receiver_type_args) =
                self.extract_type_args_from_receiver_pattern(imp, ty, &[])
            else {
                self.diagnostics.push(drop_diagnostic(
                    format!(
                        "selected Drop implementation could not resolve complete receiver substitution for type {}",
                        self.display_type_for_diagnostic(ty)
                    ),
                    origin_span.as_ref(),
                ));
                return;
            };
            if receiver_type_args.len() != imp.type_generics.len() {
                self.diagnostics.push(drop_diagnostic(
                    format!(
                        "selected Drop implementation could not resolve complete receiver substitution for type {}",
                        self.display_type_for_diagnostic(ty)
                    ),
                    origin_span.as_ref(),
                ));
                return;
            }
            receiver_type_args
        } else {
            Vec::new()
        };

        let origin = self.method_instance_origin(imp, &method);
        let substitution = type_args.clone();
        let instance_key = crate::mono::InstanceKey::new(origin.clone(), substitution.clone());
        let backend_symbol = String::new();
        if let Some(instance_id) = self.instances.get(&instance_key) {
            let receiver = self
                .instances
                .pre_mir_body(instance_id)
                .and_then(|function| function.params.first())
                .map(|param| param.ty.clone())
                .or_else(|| {
                    self.instances
                        .record(instance_id)
                        .and_then(|record| record.declared.as_ref())
                        .and_then(|function| function.params.first())
                        .map(|param| param.ty.clone())
                })
                .unwrap_or_else(|| ty.clone());
            let receiver_ty = self.intern_type(&receiver);
            self.generated_drop_instances.insert(
                receiver_ty,
                crate::mono::GeneratedMethodInstance {
                    receiver_ty,
                    trait_id: drop_trait_id,
                    member_id: drop_method_id,
                    instance_id,
                    origin_span,
                },
            );
            return;
        }
        let in_progress_key = (imp.id, substitution.clone());
        if !self
            .drop_monomorphization_in_progress
            .insert(in_progress_key.clone())
        {
            return;
        }

        let type_suffix = if type_args.is_empty() {
            Self::receiver_mono_suffix(ty)
        } else {
            type_args
                .iter()
                .map(|id| Self::type_to_mono_suffix(&self.type_for(*id)))
                .collect::<Vec<_>>()
                .join("_")
        };
        let specialized_name = format!("{}_{}_drop", imp.type_name, type_suffix);
        let specialized_func = self.specialize_impl_method(
            &method,
            &type_args,
            &imp.type_generics,
            &specialized_name,
            imp.id,
            imp.trait_id,
        );
        let concrete_receiver = specialized_func
            .params
            .first()
            .map(|param| param.ty.clone())
            .unwrap_or_else(|| ty.clone());
        let receiver_ty = self.intern_type(&concrete_receiver);
        self.drop_monomorphization_in_progress
            .remove(&in_progress_key);

        let instance_id = self
            .instances
            .intern(instance_key, |id| crate::mono::InstanceRecord {
                id,
                origin: origin.clone(),
                substitution: substitution.clone(),
                symbols: crate::mono::InstanceSymbols::new(
                    format!("{}::drop", imp.type_name),
                    backend_symbol.clone(),
                ),
                declared: None,
                provided_by_object: false,
                is_specialization: true,
            });
        self.instances
            .insert_pre_mir_body(instance_id, specialized_func);
        self.generated_drop_instances.insert(
            receiver_ty,
            crate::mono::GeneratedMethodInstance {
                receiver_ty,
                trait_id: drop_trait_id,
                member_id: drop_method_id,
                instance_id,
                origin_span,
            },
        );
    }

    pub(in crate::mono) fn drop_trait_id(&self) -> Option<DefId> {
        self.drop_trait_id
    }
}
