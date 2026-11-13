//! Trait conformance checks.

use super::default_method_injector::{
    prepare_default_method, DefaultMethodInjectionContext, TraitDefaultMethodInjector,
};
use super::index_alpha_equivalence::{
    alpha_equivalent_type, associated_type_for_index, index_impls_are_alpha_equivalent,
};
use super::service::{collect_effective_trait_methods, TraitConformanceService};
use super::trait_impl_substitution::substitute_self;
use crate::hir::*;
use crate::ids::DefId;
use crate::lower::ResolveError;
use crate::type_services::visit::remap_generic_params_in_place;
use crate::types::{GenericParamId, TraitBound, Type};
use std::collections::HashMap;

impl TraitConformanceService<'_> {
    pub(super) fn display_type(&self, ty: &Type) -> String {
        let mut context =
            crate::type_services::display::TypeDisplayContext::from_resolver(self.resolver);
        for resolver in self.dependency_resolvers.values() {
            context.extend_resolver(resolver);
        }
        for (_, function) in self.items.functions() {
            context.insert_generic_names(&function.generic_params);
        }
        for (_, signature) in self.items.function_sigs() {
            context.insert_generic_names(&signature.generic_params);
        }
        for (_, structure) in self.items.structures() {
            context.insert_generic_names(&structure.generic_params);
        }
        for (_, enumeration) in self.items.enumerations() {
            context.insert_generic_names(&enumeration.generic_params);
        }
        for (_, alias) in self.items.type_aliases() {
            context.insert_generic_names(&alias.generic_params);
        }
        for (_, trait_def) in self.items.trait_defs() {
            context.insert_generic_names(&trait_def.generic_params);
            if let Some(target) = &trait_def.target {
                context.insert_generic_name(target.id, target.name.clone());
            }
            for associated in &trait_def.associated_types {
                context.insert_associated_name(
                    crate::types::AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: associated.id,
                    },
                    associated.name.clone(),
                );
            }
            for function in trait_def.methods.values() {
                context.insert_generic_names(&function.generic_params);
            }
            for signature in trait_def.signatures.values() {
                context.insert_generic_names(&signature.generic_params);
            }
        }
        for (_, impl_def) in self.items.impl_defs() {
            context.insert_generic_names(&impl_def.type_generics);
            context.insert_generic_names(&impl_def.trait_generics);
            for associated in &impl_def.associated_types {
                context.insert_associated_name(
                    crate::types::AssociatedTypeKey {
                        owner: impl_def.id,
                        assoc_type_id: associated.id,
                    },
                    associated.name.clone(),
                );
            }
            for function in impl_def.methods.values() {
                context.insert_generic_names(&function.generic_params);
            }
        }
        crate::type_services::display::display_type_with_context(ty, &context).to_string()
    }

    fn resolve_item_id(&self, name: &str) -> Option<DefId> {
        self.resolver.resolve_item_or_alias(name).or_else(|| {
            name.contains("::").then(|| {
                self.dependency_resolvers
                    .values()
                    .find_map(|resolver| resolver.resolve_item_or_alias(name))
            })?
        })
    }

    fn resolve_module_alias_or_item_id(&self, name: &str) -> Option<DefId> {
        self.resolver
            .module_aliases
            .get(name)
            .copied()
            .or_else(|| self.resolve_item_id(name))
    }

    fn resolve_trait_id(&self, name: &str) -> Option<DefId> {
        self.resolve_trait_type_id(name)
    }

    fn resolve_trait_type_id(&self, name: &str) -> Option<DefId> {
        self.resolve_module_alias_or_item_id(name)
            .and_then(|id| self.items.trait_def(id).map(|trait_def| trait_def.id))
    }

    pub(crate) fn check_trait_conformance(&mut self) {
        let impl_ids: Vec<_> = self
            .items
            .impl_defs_in_order()
            .map(|(impl_id, _)| impl_id)
            .collect();
        for impl_id in impl_ids {
            let trait_snapshot = self
                .items
                .impl_def(impl_id)
                .unwrap()
                .trait_id
                .or_else(|| {
                    self.items
                        .impl_def(impl_id)
                        .unwrap()
                        .trait_name
                        .as_deref()
                        .and_then(|name| self.resolve_trait_id(name))
                })
                .and_then(|trait_id| self.items.trait_def(trait_id))
                .cloned();
            let impl_span = self
                .source_map
                .definition_declaration_span(impl_id)
                .cloned();
            let imp = self.items.impl_def_mut(impl_id).unwrap();
            if imp.id.crate_id != self.root_crate_id {
                continue;
            }

            if let Some(trait_name) = imp.trait_name.clone() {
                let trait_def = trait_snapshot;

                if let Some(trait_def) = trait_def.as_ref() {
                    if imp.trait_id.is_none() {
                        imp.trait_id = Some(trait_def.id);
                    }

                    let self_type = match &imp.receiver_pattern {
                        HirImplReceiverPattern::Exact(ty)
                        | HirImplReceiverPattern::Constructor(ty) => ty.clone(),
                        HirImplReceiverPattern::SliceFamily { element } => {
                            Type::Slice(Box::new(element.clone()))
                        }
                    };
                    let impl_trait_id = Some(trait_def.id);
                    let impl_associated_types = imp.associated_types.clone();
                    let mut trait_generic_subst: HashMap<GenericParamId, Type> = trait_def
                        .generic_params
                        .iter()
                        .enumerate()
                        .filter_map(|(index, _)| {
                            imp.trait_arg_types.get(index).cloned().map(|actual| {
                                (
                                    GenericParamId {
                                        owner: trait_def.id,
                                        index: index as u32,
                                    },
                                    actual,
                                )
                            })
                        })
                        .collect();
                    trait_generic_subst.insert(
                        GenericParamId {
                            owner: trait_def.id,
                            index: trait_def.generic_params.len() as u32,
                        },
                        self_type.clone(),
                    );

                    for assoc in &trait_def.associated_types {
                        if !imp.associated_types.iter().any(|item| item.id == assoc.id) {
                            self.output.diagnostics.push(
                                crate::lower::ResolveError::from_optional_span(
                                    format!(
                                        "Type '{}' does not define required associated type '{}' from trait '{}'",
                                        imp.type_name, assoc.name, trait_name
                                    ),
                                    impl_span.clone(),
                                ),
                            );
                        }
                    }

                    for assoc in &imp.associated_types {
                        if !trait_def
                            .associated_types
                            .iter()
                            .any(|decl| decl.id == assoc.id)
                        {
                            let span = self
                                .source_map
                                .symbol(&crate::source_map::SourceSymbol::AssociatedType {
                                    owner: imp.id,
                                    associated: assoc.id,
                                })
                                .map(|source| source.name_span.clone())
                                .or_else(|| impl_span.clone());
                            self.output.diagnostics.push(
                                crate::lower::ResolveError::from_optional_span(
                                    format!(
                                    "Type '{}' defines unknown associated type '{}' for trait '{}'",
                                    imp.type_name, assoc.name, trait_name
                                ),
                                    span,
                                ),
                            );
                        }
                    }

                    // Check required signatures are implemented and unify types
                    for (sig_name, sig) in &trait_def.signatures {
                        if !imp.methods.contains_key(sig_name)
                            && !trait_def.methods.contains_key(sig_name)
                        {
                            // Required method not implemented and no default
                            self.output.diagnostics.push(
                                crate::lower::ResolveError::from_optional_span(
                                    format!(
                                        "Type '{}' does not implement required method '{}' from trait '{}'",
                                        imp.type_name, sig_name, trait_name
                                    ),
                                    impl_span.clone(),
                                ),
                            );
                        } else if let Some(method) = imp.methods.get_mut(sig_name) {
                            let method_span = self
                                .source_map
                                .definition_declaration_span(method.id)
                                .cloned();
                            let signature_span = method_span.clone();
                            let signature_method_generics = sig
                                .generic_params
                                .iter()
                                .filter_map(|signature_param| {
                                    method
                                        .generic_params
                                        .iter()
                                        .find(|method_param| {
                                            method_param.name == signature_param.name
                                                && method_param.kind == signature_param.kind
                                        })
                                        .map(|method_param| (signature_param.id, method_param.id))
                                })
                                .collect::<HashMap<_, _>>();
                            // Unify impl method types with trait signature (after substituting Self)
                            // First, handle curried function types: Self -> Self -> Self
                            // The trait signature params are [Self, Self] for a binary op
                            // We need to unify each parameter with the substituted type

                            if method.params.len() != sig.params.len() {
                                self.output.diagnostics.push(crate::lower::ResolveError::from_optional_span_code(format!(
                                        "Type '{}' method '{}' parameter count mismatch for trait '{}': expected {}, found {}",
                                        imp.type_name,
                                        sig_name,
                                        trait_name,
                                        sig.params.len(),
                                        method.params.len()
                                    ), signature_span.clone(), crate::diagnostic::DiagnosticCode::Type));
                                continue;
                            }

                            for (i, param) in method.params.iter_mut().enumerate() {
                                param.ty = substitute_self(
                                    &param.ty,
                                    &self_type,
                                    impl_trait_id,
                                    &imp.trait_arg_types,
                                    &impl_associated_types,
                                    &trait_generic_subst,
                                );
                                let mut expected_ty = substitute_self(
                                    &sig.params[i],
                                    &self_type,
                                    impl_trait_id,
                                    &imp.trait_arg_types,
                                    &impl_associated_types,
                                    &trait_generic_subst,
                                );
                                remap_generic_params_in_place(&mut expected_ty, &mut |param| {
                                    signature_method_generics
                                        .get(&param)
                                        .copied()
                                        .unwrap_or(param)
                                });
                                if let Err(err) = self.engine.unify(&param.ty, &expected_ty) {
                                    self.output.diagnostics.push(crate::lower::ResolveError::from_optional_span_code(format!(
                                            "Type '{}' method '{}' parameter {} type mismatch for trait '{}': {}",
                                            imp.type_name,
                                            sig_name,
                                            i,
                                            trait_name,
                                            err.render(&self.engine)
                                        ), signature_span.clone(), crate::diagnostic::DiagnosticCode::Type));
                                } else {
                                    let resolved = self.engine.resolve(&param.ty);
                                    // Update to resolved type
                                    param.ty = resolved;
                                }
                            }

                            // Unify return type
                            method.ret_type = substitute_self(
                                &method.ret_type,
                                &self_type,
                                impl_trait_id,
                                &imp.trait_arg_types,
                                &impl_associated_types,
                                &trait_generic_subst,
                            );
                            let mut expected_ret = substitute_self(
                                &sig.ret,
                                &self_type,
                                impl_trait_id,
                                &imp.trait_arg_types,
                                &impl_associated_types,
                                &trait_generic_subst,
                            );
                            remap_generic_params_in_place(&mut expected_ret, &mut |param| {
                                signature_method_generics
                                    .get(&param)
                                    .copied()
                                    .unwrap_or(param)
                            });
                            if let Err(err) = self.engine.unify(&method.ret_type, &expected_ret) {
                                self.output.diagnostics.push(crate::lower::ResolveError::from_optional_span_code(format!(
                                        "Type '{}' method '{}' return type mismatch for trait '{}': {}",
                                        imp.type_name, sig_name, trait_name, err.render(&self.engine)
                                    ), method_span, crate::diagnostic::DiagnosticCode::Type));
                            } else {
                                // Update to resolved type
                                method.ret_type = self.engine.resolve(&method.ret_type);
                            }
                        }
                    }

                    // Inject default methods from trait that aren't overridden.
                    for (method_name, default_func) in &trait_def.methods {
                        if !imp.methods.contains_key(method_name) {
                            let default_trait_name = trait_name.clone();
                            let trait_arg_types = imp.trait_arg_types.clone();
                            let func = TraitDefaultMethodInjector::new(
                                self.engine,
                                self.root_crate_id,
                                self.local_def_ids,
                                self.current_def_ids,
                                self.source_map,
                                &mut self.output.diagnostics,
                            )
                            .prepare_missing_default_method(
                                imp,
                                method_name,
                                default_func,
                                &self_type,
                                DefaultMethodInjectionContext {
                                    trait_name: &default_trait_name,
                                },
                                |func| {
                                    prepare_default_method(
                                        func,
                                        &self_type,
                                        impl_trait_id,
                                        &trait_arg_types,
                                        &impl_associated_types,
                                        &trait_generic_subst,
                                    );
                                },
                            );
                            imp.methods.insert(method_name.to_string(), func);
                        }
                    }
                } else {
                    self.output
                        .diagnostics
                        .push(crate::lower::ResolveError::from_optional_span(
                            format!("unknown trait '{}' in impl", trait_name),
                            impl_span.clone(),
                        ));
                }
            }
        }

        self.check_supertrait_obligations();

        self.validate_index_mut_pairs();

        collect_effective_trait_methods(self.items, self.root_crate_id, &mut self.output);
    }

    fn check_supertrait_obligations(&mut self) {
        let traits = self
            .items
            .trait_defs()
            .map(|(id, trait_def)| (id, trait_def.clone()))
            .collect::<HashMap<_, _>>();
        let impls = self
            .items
            .impl_defs_in_order()
            .map(|(id, imp)| (id, imp.clone()))
            .collect::<HashMap<_, _>>();
        let mut impl_ids = impls.keys().copied().collect::<Vec<_>>();
        impl_ids.sort();

        for impl_id in impl_ids {
            let Some(imp) = impls.get(&impl_id) else {
                continue;
            };
            if imp.id.crate_id != self.root_crate_id {
                continue;
            }
            let Some(trait_id) = imp.trait_id else {
                continue;
            };
            let Some(trait_def) = traits.get(&trait_id) else {
                continue;
            };
            let subject = match &imp.receiver_pattern {
                HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                    ty.clone()
                }
                HirImplReceiverPattern::SliceFamily { element } => {
                    Type::Slice(Box::new(element.clone()))
                }
            };
            let mut subst = crate::selection::generic_substitution_for_owner(
                &trait_def.generic_params,
                &imp.trait_arg_types,
            );
            let target_id =
                trait_def
                    .target
                    .as_ref()
                    .map(|target| target.id)
                    .unwrap_or(GenericParamId {
                        owner: trait_def.id,
                        index: trait_def.generic_params.len() as u32,
                    });
            subst.insert(target_id, subject);
            let selection = crate::selection::SelectionService::new(
                &traits,
                &impls,
                None,
                Some(trait_id),
                &imp.bounds,
            );
            for predicate in &trait_def.predicates {
                let crate::types::Predicate::Trait {
                    subject,
                    trait_id: required_trait,
                    args,
                } = predicate.substitute_generics(&subst);
                let bound = TraitBound {
                    trait_id: required_trait,
                    type_args: args,
                };
                if !selection.trait_bound_satisfied(&subject, &bound) {
                    let required_name = traits
                        .get(&required_trait)
                        .map(|required| required.name.as_str())
                        .unwrap_or("<unknown>");
                    // Written as source syntax: `Task: FnOnce (), ()`.
                    let required_args = bound
                        .type_args
                        .iter()
                        .map(|arg| self.display_type(arg))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let required = if required_args.is_empty() {
                        crate::type_services::display::short_item_name(required_name).to_string()
                    } else {
                        format!(
                            "{} {required_args}",
                            crate::type_services::display::short_item_name(required_name)
                        )
                    };
                    let message = format!(
                        "implementation of trait '{}' for '{}' does not satisfy supertrait obligation '{}: {}'",
                        crate::type_services::display::short_item_name(&trait_def.name),
                        crate::type_services::display::short_item_name(&imp.type_name),
                        self.display_type(&subject),
                        required,
                    );
                    let span = self.source_map.definition_declaration_span(imp.id).cloned();
                    self.output
                        .diagnostics
                        .push(ResolveError::from_optional_span_code(
                            message,
                            span,
                            crate::diagnostic::DiagnosticCode::Type,
                        ));
                }
            }
        }
    }

    pub(super) fn validate_index_mut_pairs(&mut self) {
        let Some(protocols) = self.index_protocol_ids else {
            return;
        };

        let mut index_mut_impls = self
            .items
            .impl_defs_in_order()
            .filter(|(_, imp)| {
                imp.id.crate_id == self.root_crate_id
                    && imp.trait_id == Some(protocols.index_mut_trait)
            })
            .map(|(_, imp)| imp.clone())
            .collect::<Vec<_>>();
        index_mut_impls.sort_by_key(|imp| imp.id);

        let mut index_impls = self
            .items
            .impl_defs_in_order()
            .filter(|(_, imp)| {
                imp.id.crate_id == self.root_crate_id && imp.trait_id == Some(protocols.index_trait)
            })
            .map(|(_, imp)| imp.clone())
            .collect::<Vec<_>>();
        index_impls.sort_by_key(|imp| imp.id);

        for index_mut in index_mut_impls {
            let index_mut_span = self
                .source_map
                .definition_declaration_span(index_mut.id)
                .cloned();
            let mut matches = index_impls
                .iter()
                .filter(|index| index_impls_are_alpha_equivalent(&index_mut, index))
                .collect::<Vec<_>>();
            matches.sort_by_key(|imp| imp.id);

            let key = index_mut
                .trait_arg_types
                .first()
                .map(|ty| self.display_type(ty))
                .unwrap_or_else(|| "<missing>".to_string());
            let context = format!(
                "IndexMut implementation for {} with key {}",
                index_mut.type_name, key
            );

            let Some(index) = (match matches.as_slice() {
                [] => {
                    self.output
                        .diagnostics
                        .push(ResolveError::from_optional_span(
                            format!("{context} requires a matching Index implementation"),
                            index_mut_span.clone(),
                        ));
                    None
                }
                [index] => Some(*index),
                _matches => {
                    self.output
                        .diagnostics
                        .push(ResolveError::from_optional_span(
                            format!("{context} has multiple matching Index implementations"),
                            index_mut_span.clone(),
                        ));
                    None
                }
            }) else {
                continue;
            };

            let Some(index_output) = associated_type_for_index(&index, protocols.index_output)
            else {
                continue;
            };
            let Some(index_mut_output) =
                associated_type_for_index(&index_mut, protocols.index_mut_output)
            else {
                continue;
            };
            if !alpha_equivalent_type(
                &index_output.ty,
                index.id,
                &index_mut_output.ty,
                index_mut.id,
            ) {
                self.output
                    .diagnostics
                    .push(ResolveError::from_optional_span(
                        format!(
                            "IndexMut implementation output {} does not match Index output {}",
                            self.display_type(&index_mut_output.ty),
                            self.display_type(&index_output.ty)
                        ),
                        index_mut_span,
                    ));
            }
        }
    }
}
