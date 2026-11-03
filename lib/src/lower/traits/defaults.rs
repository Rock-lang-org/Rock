//! Trait default method body lowering

use crate::ast;

use crate::ids::DefId;
use crate::lower::body_context::{BodyLoweringContext, BodyOwner, GenericLoweringContext};
use crate::lower::Lowerer;
use crate::types::{GenericParamId, TraitBound, Type};

fn type_contains_projection(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| matches!(nested, Type::Projection { .. }))
}

impl Lowerer {
    pub(crate) fn lower_trait_default_bodies(
        &mut self,
        module: &ast::Module,
        module_id: crate::ids::ModuleId,
    ) {
        for (ordinal, top_level) in module.top_levels.iter().enumerate() {
            let ast::TopLevel::TraitDecl(td) = top_level else {
                continue;
            };
            let Some(record) = self.item_index.item_at_source(module_id, ordinal) else {
                self.diagnostics.push_toolchain(
                    "missing indexed trait declaration while lowering default bodies".to_string(),
                );
                continue;
            };
            if record.kind != crate::collect::item_index::ItemKind::Trait {
                self.diagnostics.push_toolchain(
                    "indexed declaration kind does not match trait syntax".to_string(),
                );
                continue;
            }

            self.lower_trait_bodies(td, record.def_id);
        }

        // Recurse into sub-modules
        for tl in &module.top_levels {
            if let ast::TopLevel::Module(module_decl) = tl {
                let Some(module_name) = module_decl.0.name.as_ref() else {
                    continue;
                };
                let Some(child_module_id) = self
                    .item_index
                    .child_module_id(module_id, &module_name.name)
                else {
                    self.diagnostics.push_toolchain(format!(
                        "missing indexed module '{}' while lowering trait default bodies",
                        module_name.name
                    ));
                    continue;
                };
                self.lower_trait_default_bodies(&module_decl.0, child_module_id);
            }
        }
    }

    pub(crate) fn lower_trait_bodies(&mut self, td: &ast::TraitDecl, trait_id: DefId) {
        let trait_name = td.name.name.clone();
        let previous_trait = self.current_trait.clone();
        let previous_trait_id = self.current_trait_id;
        let prev_trait_generics = self.current_trait_generics.clone();
        let previous_generic_context = self.generic_context.clone();
        let Some(trait_def) = self.trait_by_id(trait_id).cloned() else {
            self.diagnostics.push_toolchain(
                "missing indexed trait declaration while lowering default bodies".to_string(),
            );
            return;
        };
        let Some(trait_context_name) = self.canonical_name_for_def_id(trait_id).map(str::to_string)
        else {
            self.diagnostics.push_toolchain(
                "missing canonical trait name while lowering default bodies".to_string(),
            );
            return;
        };

        // Set current trait so method lookups on TypeVar self can find sibling methods
        self.current_trait = Some(trait_context_name.clone());
        self.current_trait_id = Some(trait_id);
        self.current_trait_generics = td
            .generic_params
            .iter()
            .filter(|param| param.kind.is_none())
            .map(|param| param.name.name.clone())
            .collect();
        let mut params = self.current_trait_generics.clone();
        params.push("Self".to_string());
        self.generic_context = Some(GenericLoweringContext::new(trait_def.id, params));

        let mut methods = Vec::with_capacity(td.methods.len());
        for (ident, fd) in &td.methods {
            let method_name = ident.name.clone();
            let Some(method_id) = trait_def.methods.get(&method_name).map(|method| method.id)
            else {
                self.diagnostics.push_toolchain(format!(
                    "missing indexed trait method '{}' while lowering default body",
                    method_name
                ));
                continue;
            };
            methods.push((method_id, method_name, fd));
        }
        methods.sort_unstable_by_key(|(method_id, _, _)| *method_id);

        for (_, method_name, fd) in methods {
            let Some(mut func) = self
                .items
                .trait_def(trait_id)
                .and_then(|trait_def| trait_def.methods.get(&method_name))
                .cloned()
            else {
                self.diagnostics.push_toolchain(format!(
                    "missing indexed trait method '{}' while lowering default body",
                    method_name
                ));
                continue;
            };
            let signature_ret = trait_def
                .signatures
                .get(&method_name)
                .map(|sig| sig.ret.clone());
            if let Some(sig) = trait_def.signatures.get(&method_name) {
                func = self.lower_function_decl_header_with_sig_and_id(fd, sig, func.id);
            }
            // Body lowering reads declared return types through this canonical trait entry.
            let Some(method) = self
                .items
                .trait_def_mut(trait_id)
                .and_then(|trait_def| trait_def.methods.get_mut(&method_name))
            else {
                self.diagnostics.push_toolchain(format!(
                    "missing indexed trait method '{}' while lowering default body",
                    method_name
                ));
                continue;
            };
            *method = func.clone();
            let mut generic_bounds = func.generic_bounds.clone();
            let self_param = GenericParamId {
                owner: trait_def.id,
                index: trait_def.generic_params.len() as u32,
            };
            let trait_args = trait_def
                .generic_params
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    Type::Generic(GenericParamId {
                        owner: trait_def.id,
                        index: index as u32,
                    })
                })
                .collect();
            generic_bounds
                .entry(self_param)
                .or_insert_with(Vec::new)
                .push(TraitBound {
                    trait_id: trait_def.id,
                    type_args: trait_args,
                });

            let mut context = BodyLoweringContext::new(
                format!("{}::{}", trait_context_name, method_name),
                BodyOwner::TraitMethod {
                    trait_id: trait_def.id,
                    method_id: func.id,
                    method_name: method_name.clone(),
                },
                Some(trait_def.id),
                self.current_generic_params().to_vec(),
                generic_bounds,
                fd.is_unsafe,
            );
            // Default bodies share the declaration's generic identities,
            // including method-local parameters used in local annotations.
            for generic in trait_def
                .generic_params
                .iter()
                .chain(trait_def.target.iter())
                .chain(&func.generic_params)
            {
                context.register_generic_param(generic.name.clone(), generic.id);
            }
            context.seed_after_existing_locals(func.params.iter().map(|param| param.local_id));
            let body = self.with_body_context(context, |lowerer| {
                lowerer.push_scope();

                // Register self parameter if present
                if let Some(self_receiver) = func.self_receiver {
                    let self_ty = func.params[0].ty.clone();
                    lowerer.scope.define_local(
                        "self".to_string(),
                        self_ty,
                        matches!(self_receiver, crate::types::ReceiverMode::Mut),
                        func.params[0].local_id,
                    );
                }

                // Register other parameters
                let start = if func.self_receiver.is_some() { 1 } else { 0 };
                for param in &func.params[start..] {
                    lowerer.scope.define_local(
                        param.name.clone(),
                        param.ty.clone(),
                        param.mutable,
                        param.local_id,
                    );
                }

                let body = lowerer.lower_lambda_body(&fd.lambda);
                lowerer.pop_scope();
                body
            });

            let validate_return_now = signature_ret
                .as_ref()
                .map(|ret| !type_contains_projection(ret))
                .unwrap_or(true);
            if validate_return_now {
                if let Err(e) = self.engine.unify(&body.ty, &func.ret_type) {
                    let message = format!(
                        "In trait default method '{}.{}': return type mismatch: {}",
                        trait_name,
                        method_name,
                        e.render(&self.engine)
                    );
                    self.diagnostics
                        .push_type_with_span(message, fd.lambda.span.clone());
                }
            }

            func.body = body;

            // Resolve all TypeVars in the function that were unified during inference.
            // This is critical: when the default body is later injected into a concrete
            // impl, substitute_typevars_in_function replaces ALL remaining TypeVars with
            // the concrete type. If we don't resolve here, types like I32 (from puts)
            // that were inferred via unification would still be TypeVars and get wrongly
            // replaced with the impl type (e.g. I64).
            self.resolve_all_types_in_function(&mut func);

            // Update every alias for the same trait so ID-owned HIR can collapse
            // aliases without losing the lowered default body.
            let Some(method) = self
                .items
                .trait_def_mut(trait_id)
                .and_then(|trait_def| trait_def.methods.get_mut(&method_name))
            else {
                self.diagnostics.push_toolchain(format!(
                    "missing indexed trait method '{}' while lowering default body",
                    method_name
                ));
                continue;
            };
            *method = func;
        }

        self.current_trait = previous_trait;
        self.current_trait_id = previous_trait_id;
        self.current_trait_generics = prev_trait_generics;
        self.generic_context = previous_generic_context;
    }
}

#[cfg(test)]
mod tests;
